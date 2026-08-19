use std::fs;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    ActionAuthorization, ActionReceipt, CancelAck, ClickRequest, ComputerBackend, ComputerUseError,
    KeyPressRequest, Observation, PermissionRequest, PermissionStatus, PrepareActionRequest,
    PreparedAction, TypeTextRequest, WindowFilter, WindowInfo, WindowTarget,
};

use super::protocol::{
    read_control_response, read_response, write_control_request, write_request, ControlRequest,
    ControlRequestFrame, ControlResponse, ControlResponseFrame, Request, RequestFrame, Response,
    ResponseFrame, PROTOCOL_VERSION,
};

mod action_gate;
use action_gate::ActionGate;

const SERVICE_APP_NAME: &str = env!("DESKTOP_COMPUTER_USE_MAC_HELPER_APP");
const SERVICE_EXECUTABLE: &str = env!("DESKTOP_COMPUTER_USE_HELPER_EXECUTABLE");
const SERVICE_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(not(test))]
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const CALL_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(not(test))]
const CONTROL_TIMEOUT: Duration = Duration::from_secs(6);
#[cfg(test)]
const CONTROL_TIMEOUT: Duration = Duration::from_millis(200);

type Connector = dyn Fn() -> Result<RawConnection, ComputerUseError> + Send + Sync + 'static;

pub struct MacHelperBackend {
    manager: Mutex<ConnectionManager>,
    action_gate: ActionGate,
}

impl MacHelperBackend {
    pub fn new() -> Result<Self, ComputerUseError> {
        Ok(Self {
            manager: Mutex::new(ConnectionManager::new(Arc::new(spawn_helper))),
            action_gate: ActionGate::default(),
        })
    }

    #[cfg(test)]
    fn with_connector(connector: Arc<Connector>) -> Self {
        Self {
            manager: Mutex::new(ConnectionManager::new(connector)),
            action_gate: ActionGate::default(),
        }
    }

    fn connection(&self) -> Result<Arc<Connection>, ComputerUseError> {
        self.manager
            .lock()
            .map_err(|_| {
                ComputerUseError::HelperUnavailable(
                    "helper connection lock was poisoned".to_string(),
                )
            })?
            .connection()
    }

    fn call(&self, request: Request) -> Result<Response, ComputerUseError> {
        let connection = self.connection()?;
        self.call_on_connection(connection, request)
    }

    fn call_on_connection(
        &self,
        connection: Arc<Connection>,
        request: Request,
    ) -> Result<Response, ComputerUseError> {
        match connection.call(request) {
            Ok(response) => Ok(response),
            Err(CallError::Remote(message)) => Err(ComputerUseError::HelperRejected(message)),
            Err(CallError::Transport(error)) => {
                drop(connection.force_terminate());
                if let Ok(mut manager) = self.manager.lock() {
                    manager.invalidate(&connection);
                }
                Err(error)
            }
        }
    }
}

impl ComputerBackend for MacHelperBackend {
    fn permissions(&self) -> Result<PermissionStatus, ComputerUseError> {
        match self.call(Request::Permissions)? {
            Response::Permissions(status) => Ok(status),
            response => Err(unexpected("Permissions", response)),
        }
    }

    fn request_permissions(
        &self,
        request: PermissionRequest,
    ) -> Result<PermissionStatus, ComputerUseError> {
        match self.call(Request::PromptPermissions(request))? {
            Response::Permissions(status) => Ok(status),
            response => Err(unexpected("Permissions", response)),
        }
    }

    fn list_windows(&self, filter: WindowFilter) -> Result<Vec<WindowInfo>, ComputerUseError> {
        match self.call(Request::ListWindows(filter))? {
            Response::Windows(windows) => Ok(windows),
            response => Err(unexpected("Windows", response)),
        }
    }

    fn launch_application(&self, bundle_id: &str) -> Result<(), ComputerUseError> {
        expect_unit(self.call(Request::LaunchApplication {
            bundle_id: bundle_id.to_string(),
        })?)
    }

    fn observe(&self, window: &WindowTarget) -> Result<Observation, ComputerUseError> {
        let generation = self.action_gate.generation()?;
        match self.call(Request::Observe(window.clone()))? {
            Response::Observation(observation) => {
                if !self
                    .action_gate
                    .register_observation(generation, &observation.observation_id)?
                {
                    return Err(ComputerUseError::InputCancelled);
                }
                Ok(observation)
            }
            response => Err(unexpected("Observation", response)),
        }
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> Result<PreparedAction, ComputerUseError> {
        let generation = self.action_gate.generation()?;
        self.action_gate
            .consume_observation(generation, &request.observation_id)?;
        match self.call(Request::PrepareAction(request))? {
            Response::PreparedAction(prepared) => {
                if !self
                    .action_gate
                    .register_prepared(generation, &prepared.id)?
                {
                    return Err(ComputerUseError::InputCancelled);
                }
                Ok(prepared)
            }
            response => Err(unexpected("PreparedAction", response)),
        }
    }

    fn prepared_action(&self, id: &str) -> Result<PreparedAction, ComputerUseError> {
        match self.call(Request::PreparedAction { id: id.to_string() })? {
            Response::PreparedAction(prepared) => Ok(prepared),
            response => Err(unexpected("PreparedAction", response)),
        }
    }

    fn authorize_action(
        &self,
        id: &str,
        authorization: ActionAuthorization,
    ) -> Result<(), ComputerUseError> {
        expect_unit(self.call(Request::AuthorizeAction {
            id: id.to_string(),
            authorization,
        })?)
    }

    fn commit_action(&self, id: &str) -> Result<ActionReceipt, ComputerUseError> {
        // Establish the authenticated helper connection before registering the
        // action. Cancellation can therefore either invalidate this prepared
        // capability before the guard is acquired, or find a concrete helper
        // process to cancel/terminate after it is acquired. There is no
        // untracked window in which a late commit can start after a quiesced
        // cancellation acknowledgment.
        let connection = self.connection()?;
        let _action = self.action_gate.begin_prepared(id)?;
        match self.call_on_connection(connection, Request::CommitAction { id: id.to_string() })? {
            Response::ActionReceipt(receipt) => Ok(receipt),
            response => Err(unexpected("ActionReceipt", response)),
        }
    }

    fn cancel_active(&self) -> Result<CancelAck, ComputerUseError> {
        let action_was_active = self.action_gate.cancel()?;
        let connection = self
            .manager
            .lock()
            .map_err(|_| {
                ComputerUseError::HelperUnavailable(
                    "helper connection lock was poisoned".to_string(),
                )
            })?
            .current();
        let Some(connection) = connection else {
            let quiesced = self.action_gate.wait_inactive(CONTROL_TIMEOUT)?;
            return Ok(CancelAck {
                lease_id: None,
                quiesced,
                helper_terminated: false,
            });
        };
        let attempted = connection.cancel_active();
        let lease_id = attempted.as_ref().ok().and_then(|ack| ack.lease_id.clone());
        let helper_acknowledged = attempted.as_ref().is_ok_and(|ack| ack.quiesced);
        let action_still_active = self.action_gate.is_active()?;
        let late_start_race = action_was_active && action_still_active && lease_id.is_none();
        let mut helper_terminated = false;

        if !helper_acknowledged || late_start_race {
            helper_terminated = connection.force_terminate()?;
            if helper_terminated {
                if let Ok(mut manager) = self.manager.lock() {
                    manager.invalidate(&connection);
                }
            } else {
                attempted?;
            }
        }

        let mut quiesced = self.action_gate.wait_inactive(CONTROL_TIMEOUT)?;
        if !quiesced && !helper_terminated {
            helper_terminated = connection.force_terminate()?;
            if helper_terminated {
                if let Ok(mut manager) = self.manager.lock() {
                    manager.invalidate(&connection);
                }
                quiesced = self.action_gate.wait_inactive(CONTROL_TIMEOUT)?;
            }
        }
        Ok(CancelAck {
            lease_id,
            quiesced,
            helper_terminated,
        })
    }

    fn click(&self, request: ClickRequest) -> Result<(), ComputerUseError> {
        let connection = self.connection()?;
        let _action = self
            .action_gate
            .begin_observation(&request.observation_id)?;
        expect_unit(self.call_on_connection(connection, Request::Click(request))?)
    }

    fn type_text(&self, request: TypeTextRequest) -> Result<(), ComputerUseError> {
        let connection = self.connection()?;
        let _action = self
            .action_gate
            .begin_observation(&request.observation_id)?;
        expect_unit(self.call_on_connection(connection, Request::TypeText(request))?)
    }

    fn keypress(&self, request: KeyPressRequest) -> Result<(), ComputerUseError> {
        let connection = self.connection()?;
        let _action = self
            .action_gate
            .begin_observation(&request.observation_id)?;
        expect_unit(self.call_on_connection(connection, Request::KeyPress(request))?)
    }
}

struct ConnectionManager {
    connection: Option<Arc<Connection>>,
    connector: Arc<Connector>,
}

impl ConnectionManager {
    fn new(connector: Arc<Connector>) -> Self {
        Self {
            connection: None,
            connector,
        }
    }

    fn connection(&mut self) -> Result<Arc<Connection>, ComputerUseError> {
        if self.connection.is_none() {
            self.connection = Some(Arc::new(Connection::establish((self.connector)()?)?));
        }
        Ok(self
            .connection
            .as_ref()
            .expect("connection established above")
            .clone())
    }

    fn current(&self) -> Option<Arc<Connection>> {
        self.connection.clone()
    }

    fn invalidate(&mut self, connection: &Arc<Connection>) {
        if self
            .connection
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, connection))
        {
            self.connection = None;
        }
    }
}

struct RawConnection {
    stream: UnixStream,
    control_stream: Option<UnixStream>,
    managed_service: bool,
    socket_path: Option<PathBuf>,
}

struct Connection {
    primary: Mutex<PrimaryChannel>,
    control: Mutex<Option<ControlChannel>>,
    service_pid: Mutex<Option<u32>>,
    socket_path: Option<PathBuf>,
    session_id: String,
    action_in_flight: AtomicBool,
}

struct PrimaryChannel {
    stream: UnixStream,
    next_request_id: u64,
}

struct ControlChannel {
    stream: UnixStream,
    next_request_id: u64,
}

impl Connection {
    fn establish(mut raw: RawConnection) -> Result<Self, ComputerUseError> {
        let (session_id, next_request_id, service_pid) = handshake(&mut raw.stream)?;
        if raw.managed_service {
            super::auth::verify_service_pid(service_pid)
                .map_err(ComputerUseError::HelperRejected)?;
        }
        let control = match raw.control_stream.as_mut() {
            Some(stream) => {
                let next_request_id = handshake_control(stream, &session_id)?;
                Some(ControlChannel {
                    stream: raw
                        .control_stream
                        .take()
                        .expect("control stream was present above"),
                    next_request_id,
                })
            }
            None => None,
        };
        Ok(Self {
            primary: Mutex::new(PrimaryChannel {
                stream: raw.stream,
                next_request_id,
            }),
            control: Mutex::new(control),
            service_pid: Mutex::new(raw.managed_service.then_some(service_pid)),
            socket_path: raw.socket_path,
            session_id,
            action_in_flight: AtomicBool::new(false),
        })
    }

    fn call(&self, body: Request) -> Result<Response, CallError> {
        let _action_guard = ActionInFlightGuard::new(
            &self.action_in_flight,
            matches!(&body, Request::CommitAction { .. }),
        );
        if let Some(service_pid) = *self.service_pid.lock().map_err(|_| {
            CallError::Transport(ComputerUseError::HelperUnavailable(
                "service process lock was poisoned".to_string(),
            ))
        })? {
            if unsafe { libc::kill(service_pid as libc::pid_t, 0) } != 0 {
                return Err(CallError::Transport(ComputerUseError::HelperUnavailable(
                    "computer-use service exited before request".to_string(),
                )));
            }
        }
        let mut primary = self.primary.lock().map_err(|_| {
            CallError::Transport(ComputerUseError::HelperUnavailable(
                "helper request lock was poisoned".to_string(),
            ))
        })?;
        let request_id = primary.next_request_id;
        primary.next_request_id = primary.next_request_id.checked_add(1).ok_or_else(|| {
            CallError::Transport(ComputerUseError::HelperProtocol(
                "request id overflow".to_string(),
            ))
        })?;
        let request = RequestFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: self.session_id.clone(),
            request_id,
            body,
        };
        write_request(&mut primary.stream, &request)
            .map_err(|error| CallError::Transport(transport_error(error)))?;
        let response = read_response(&mut primary.stream)
            .map_err(|error| CallError::Transport(transport_error(error)))?;
        validate_response(&response, &self.session_id, request_id).map_err(CallError::Transport)?;
        response
            .body
            .map_err(|error| CallError::Remote(error.message))
    }

    fn cancel_active(&self) -> Result<CancelAck, ComputerUseError> {
        let mut control = self.control.lock().map_err(|_| {
            ComputerUseError::HelperUnavailable("helper cancellation lock was poisoned".to_string())
        })?;
        let control = control.as_mut().ok_or_else(|| {
            ComputerUseError::HelperUnavailable(
                "helper cancellation channel is unavailable".to_string(),
            )
        })?;
        let request_id = control.next_request_id;
        control.next_request_id = control.next_request_id.checked_add(1).ok_or_else(|| {
            ComputerUseError::HelperProtocol("control request id overflow".to_string())
        })?;
        write_control_request(
            &mut control.stream,
            &ControlRequestFrame {
                protocol_version: PROTOCOL_VERSION,
                session_id: self.session_id.clone(),
                request_id,
                body: ControlRequest::CancelActive,
            },
        )
        .map_err(control_transport_error)?;
        let response =
            read_control_response(&mut control.stream).map_err(control_transport_error)?;
        validate_control_response(&response, &self.session_id, request_id)?;
        match response.body {
            Ok(ControlResponse::CancelAck(ack))
                if ack.lease_id.is_none() && self.action_in_flight.load(Ordering::Acquire) =>
            {
                Ok(CancelAck {
                    quiesced: false,
                    ..ack
                })
            }
            Ok(ControlResponse::CancelAck(ack)) => Ok(ack),
            Ok(ControlResponse::Hello) => Err(ComputerUseError::HelperProtocol(
                "expected cancellation acknowledgment, received Hello".to_string(),
            )),
            Err(error) => Err(ComputerUseError::HelperRejected(error.message)),
        }
    }

    fn force_terminate(&self) -> Result<bool, ComputerUseError> {
        let mut service_pid = self.service_pid.lock().map_err(|_| {
            ComputerUseError::HelperUnavailable("service process lock was poisoned".to_string())
        })?;
        let Some(pid) = service_pid.take() else {
            return Ok(false);
        };
        if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
            return Ok(true);
        }
        super::auth::verify_service_pid(pid).map_err(ComputerUseError::HelperRejected)?;
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } != 0 {
            return Err(ComputerUseError::HelperUnavailable(format!(
                "could not terminate service after cancellation failure: {}",
                io::Error::last_os_error()
            )));
        }
        Ok(true)
    }
}

struct ActionInFlightGuard<'a> {
    flag: Option<&'a AtomicBool>,
}

impl<'a> ActionInFlightGuard<'a> {
    fn new(flag: &'a AtomicBool, active: bool) -> Self {
        if active {
            flag.store(true, Ordering::Release);
            Self { flag: Some(flag) }
        } else {
            Self { flag: None }
        }
    }
}

impl Drop for ActionInFlightGuard<'_> {
    fn drop(&mut self) {
        if let Some(flag) = self.flag {
            flag.store(false, Ordering::Release);
        }
    }
}

fn handshake(stream: &mut UnixStream) -> Result<(String, u64, u32), ComputerUseError> {
    stream
        .set_read_timeout(Some(CALL_TIMEOUT))
        .map_err(transport_error)?;
    stream
        .set_write_timeout(Some(CALL_TIMEOUT))
        .map_err(transport_error)?;
    let session_id = uuid::Uuid::new_v4().to_string();
    let hello = RequestFrame {
        protocol_version: PROTOCOL_VERSION,
        session_id: session_id.clone(),
        request_id: 0,
        body: Request::Hello {
            parent_pid: std::process::id(),
        },
    };
    write_request(stream, &hello).map_err(transport_error)?;
    let response = read_response(stream).map_err(transport_error)?;
    validate_response(&response, &session_id, 0)?;
    match response.body {
        Ok(Response::Hello { helper_pid }) if helper_pid > 1 => Ok((session_id, 1, helper_pid)),
        Ok(response) => Err(unexpected("Hello", response)),
        Err(error) => Err(ComputerUseError::HelperRejected(error.message)),
    }
}

fn handshake_control(stream: &mut UnixStream, session_id: &str) -> Result<u64, ComputerUseError> {
    stream
        .set_read_timeout(Some(CONTROL_TIMEOUT))
        .map_err(control_transport_error)?;
    stream
        .set_write_timeout(Some(CONTROL_TIMEOUT))
        .map_err(control_transport_error)?;
    let request = ControlRequestFrame {
        protocol_version: PROTOCOL_VERSION,
        session_id: session_id.to_string(),
        request_id: 0,
        body: ControlRequest::Hello {
            parent_pid: std::process::id(),
        },
    };
    write_control_request(stream, &request).map_err(control_transport_error)?;
    let response = read_control_response(stream).map_err(control_transport_error)?;
    validate_control_response(&response, session_id, 0)?;
    match response.body {
        Ok(ControlResponse::Hello) => Ok(1),
        Ok(ControlResponse::CancelAck(_)) => Err(ComputerUseError::HelperProtocol(
            "expected control Hello response, received cancellation acknowledgment".to_string(),
        )),
        Err(error) => Err(ComputerUseError::HelperRejected(error.message)),
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Ok(service_pid) = self.service_pid.get_mut() {
            if let Some(pid) = service_pid.take() {
                if super::auth::verify_service_pid(pid).is_ok() {
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGTERM);
                    }
                }
            }
        }
        if let Some(socket_path) = self.socket_path.as_ref() {
            drop(fs::remove_file(socket_path));
        }
    }
}

enum CallError {
    Remote(String),
    Transport(ComputerUseError),
}

fn spawn_helper() -> Result<RawConnection, ComputerUseError> {
    let service_app = service_app_path()?;
    let data_dir = crate::default_approval_store()?.root().to_path_buf();
    super::auth::verify_service_at_path(&service_app).map_err(ComputerUseError::HelperRejected)?;
    let socket_path = service_socket_path()?;
    let mut command = Command::new("/usr/bin/open");
    command
        .arg("-n")
        .arg(&service_app)
        .arg("--args")
        .arg("--socket")
        .arg(&socket_path)
        .arg("--data-dir")
        .arg(&data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = command.status().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not launch signed service at {}: {error}",
            service_app.display()
        ))
    })?;
    if !status.success() {
        return Err(ComputerUseError::HelperUnavailable(format!(
            "LaunchServices rejected the signed computer-use service with {status}"
        )));
    }
    let deadline = Instant::now() + SERVICE_STARTUP_TIMEOUT;
    let parent = connect_service(&socket_path, deadline)?;
    let control_parent = connect_service(&socket_path, deadline)?;
    Ok(RawConnection {
        stream: parent,
        control_stream: Some(control_parent),
        managed_service: true,
        socket_path: Some(socket_path),
    })
}

fn connect_service(path: &Path, deadline: Instant) -> Result<UnixStream, ComputerUseError> {
    let mut last_error = None;
    while Instant::now() < deadline {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    drop(fs::remove_file(path));
    Err(ComputerUseError::HelperUnavailable(format!(
        "computer-use service did not open its authenticated socket within {} seconds: {}",
        SERVICE_STARTUP_TIMEOUT.as_secs(),
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no connection attempt completed".to_string())
    )))
}

fn service_socket_path() -> Result<PathBuf, ComputerUseError> {
    let path = PathBuf::from("/tmp").join(format!(
        "agent-cua-{}-{}.sock",
        unsafe { libc::geteuid() },
        uuid::Uuid::new_v4()
    ));
    if path.as_os_str().as_encoded_bytes().len() >= 100 {
        return Err(ComputerUseError::HelperUnavailable(format!(
            "computer-use service socket path is too long: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn service_app_path() -> Result<PathBuf, ComputerUseError> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("DESKTOP_COMPUTER_USE_SERVICE_APP_PATH") {
        return validate_service_app_path(PathBuf::from(path), None);
    }
    let executable = std::env::current_exe().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!("could not locate desktop executable: {error}"))
    })?;
    let directory = executable.parent().ok_or_else(|| {
        ComputerUseError::HelperUnavailable(
            "desktop executable has no parent directory".to_string(),
        )
    })?;
    let contents = directory.parent().ok_or_else(|| {
        ComputerUseError::HelperUnavailable(
            "desktop executable is not inside a macOS app Contents directory".to_string(),
        )
    })?;
    let expected_resources = contents.join("Resources").canonicalize().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not resolve desktop app resources {}: {error}",
            contents.join("Resources").display()
        ))
    })?;
    validate_service_app_path(
        expected_resources.join(SERVICE_APP_NAME),
        Some(&expected_resources),
    )
}

fn validate_service_app_path(
    path: PathBuf,
    expected_directory: Option<&std::path::Path>,
) -> Result<PathBuf, ComputerUseError> {
    let canonical = path.canonicalize().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "computer-use service app is missing at {}: {error}",
            path.display()
        ))
    })?;
    if canonical.file_name().and_then(|name| name.to_str()) != Some(SERVICE_APP_NAME) {
        return Err(ComputerUseError::HelperRejected(format!(
            "service app path must end with {SERVICE_APP_NAME}"
        )));
    }
    if !canonical.is_dir() {
        return Err(ComputerUseError::HelperRejected(format!(
            "service app path is not a directory: {}",
            canonical.display()
        )));
    }
    if expected_directory.is_some_and(|directory| canonical.parent() != Some(directory)) {
        return Err(ComputerUseError::HelperRejected(
            "release service must be a real nested app in Clark Code's Resources directory"
                .to_string(),
        ));
    }
    let executable = canonical
        .join("Contents")
        .join("MacOS")
        .join(SERVICE_EXECUTABLE);
    if !executable.is_file() {
        return Err(ComputerUseError::HelperRejected(format!(
            "service app is missing its executable at {}",
            executable.display()
        )));
    }
    Ok(canonical)
}

fn validate_response(
    response: &ResponseFrame,
    session_id: &str,
    request_id: u64,
) -> Result<(), ComputerUseError> {
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(ComputerUseError::HelperProtocol(format!(
            "helper replied with protocol version {}, expected {PROTOCOL_VERSION}",
            response.protocol_version
        )));
    }
    if response.session_id != session_id {
        return Err(ComputerUseError::HelperProtocol(
            "helper response changed session identity".to_string(),
        ));
    }
    if response.request_id != request_id {
        return Err(ComputerUseError::HelperProtocol(format!(
            "helper response id {} did not match request {request_id}",
            response.request_id
        )));
    }
    Ok(())
}

fn validate_control_response(
    response: &ControlResponseFrame,
    session_id: &str,
    request_id: u64,
) -> Result<(), ComputerUseError> {
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(ComputerUseError::HelperProtocol(format!(
            "helper replied with control protocol version {}, expected {PROTOCOL_VERSION}",
            response.protocol_version
        )));
    }
    if response.session_id != session_id || response.request_id != request_id {
        return Err(ComputerUseError::HelperProtocol(
            "helper control response changed session or request identity".to_string(),
        ));
    }
    Ok(())
}

fn expect_unit(response: Response) -> Result<(), ComputerUseError> {
    match response {
        Response::Unit => Ok(()),
        response => Err(unexpected("Unit", response)),
    }
}

fn unexpected(expected: &str, response: Response) -> ComputerUseError {
    ComputerUseError::HelperProtocol(format!(
        "expected {expected} response, received {}",
        response_name(&response)
    ))
}

fn response_name(response: &Response) -> &'static str {
    match response {
        Response::Hello { .. } => "Hello",
        Response::Permissions(_) => "Permissions",
        Response::Windows(_) => "Windows",
        Response::Observation(_) => "Observation",
        Response::PreparedAction(_) => "PreparedAction",
        Response::ActionReceipt(_) => "ActionReceipt",
        Response::Unit => "Unit",
    }
}

fn control_transport_error(error: io::Error) -> ComputerUseError {
    let context = if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        format!(
            "cancellation exceeded the {} second deadline",
            CONTROL_TIMEOUT.as_secs_f64()
        )
    } else {
        error.to_string()
    };
    ComputerUseError::HelperUnavailable(context)
}

fn transport_error(error: io::Error) -> ComputerUseError {
    let context = if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        format!(
            "request exceeded the {} second deadline",
            CALL_TIMEOUT.as_secs()
        )
    } else {
        error.to_string()
    };
    ComputerUseError::HelperUnavailable(context)
}

#[cfg(test)]
mod tests;
