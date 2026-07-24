use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

const HELPER_EXECUTABLE: &str = "clark-computer-use-helper";
const CHILD_IPC_FD: libc::c_int = 3;
const CHILD_CONTROL_FD: libc::c_int = 4;
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
                let _ = connection.force_terminate();
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
    child: Option<Child>,
}

struct Connection {
    primary: Mutex<PrimaryChannel>,
    control: Mutex<Option<ControlChannel>>,
    child: Mutex<Option<Child>>,
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
        let (session_id, next_request_id) = match handshake(&mut raw.stream) {
            Ok(value) => value,
            Err(error) => {
                terminate_child(&mut raw.child);
                return Err(error);
            }
        };
        let control = match raw.control_stream.as_mut() {
            Some(stream) => match handshake_control(stream, &session_id) {
                Ok(next_request_id) => Some(ControlChannel {
                    stream: raw
                        .control_stream
                        .take()
                        .expect("control stream was present above"),
                    next_request_id,
                }),
                Err(error) => {
                    terminate_child(&mut raw.child);
                    return Err(error);
                }
            },
            None => None,
        };
        Ok(Self {
            primary: Mutex::new(PrimaryChannel {
                stream: raw.stream,
                next_request_id,
            }),
            control: Mutex::new(control),
            child: Mutex::new(raw.child),
            session_id,
            action_in_flight: AtomicBool::new(false),
        })
    }

    fn call(&self, body: Request) -> Result<Response, CallError> {
        let _action_guard = ActionInFlightGuard::new(
            &self.action_in_flight,
            matches!(&body, Request::CommitAction { .. }),
        );
        if let Some(child) = self
            .child
            .lock()
            .map_err(|_| {
                CallError::Transport(ComputerUseError::HelperUnavailable(
                    "helper process lock was poisoned".to_string(),
                ))
            })?
            .as_mut()
        {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(CallError::Transport(ComputerUseError::HelperUnavailable(
                        format!("helper exited before request with {status}"),
                    )))
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(CallError::Transport(ComputerUseError::HelperUnavailable(
                        format!("could not inspect helper process: {error}"),
                    )))
                }
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
        let mut child = self.child.lock().map_err(|_| {
            ComputerUseError::HelperUnavailable("helper process lock was poisoned".to_string())
        })?;
        let Some(mut child) = child.take() else {
            return Ok(false);
        };
        match child.try_wait() {
            Ok(Some(_)) => return Ok(true),
            Ok(None) => {}
            Err(error) => {
                return Err(ComputerUseError::HelperUnavailable(format!(
                    "could not inspect helper before termination: {error}"
                )))
            }
        }
        child.kill().map_err(|error| {
            ComputerUseError::HelperUnavailable(format!(
                "could not terminate helper after cancellation failure: {error}"
            ))
        })?;
        child.wait().map_err(|error| {
            ComputerUseError::HelperUnavailable(format!(
                "could not wait for helper termination: {error}"
            ))
        })?;
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

fn handshake(stream: &mut UnixStream) -> Result<(String, u64), ComputerUseError> {
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
        Ok(Response::Hello { helper_pid }) if helper_pid > 1 => Ok((session_id, 1)),
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
        if let Ok(child) = self.child.get_mut() {
            terminate_child(child);
        }
    }
}

fn terminate_child(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

enum CallError {
    Remote(String),
    Transport(ComputerUseError),
}

fn spawn_helper() -> Result<RawConnection, ComputerUseError> {
    let path = helper_path()?;
    let data_dir = crate::default_approval_store()?.root().to_path_buf();
    super::auth::verify_helper_at_path(&path).map_err(ComputerUseError::HelperRejected)?;
    let (parent, child_stream) = UnixStream::pair().map_err(transport_error)?;
    let (control_parent, control_child) = UnixStream::pair().map_err(transport_error)?;
    let child_fd = child_stream.as_raw_fd();
    let control_fd = control_child.as_raw_fd();
    let mut command = Command::new(&path);
    command
        .arg("--ipc-fd")
        .arg(CHILD_IPC_FD.to_string())
        .arg("--control-fd")
        .arg(CHILD_CONTROL_FD.to_string())
        .arg("--data-dir")
        .arg(data_dir)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || remap_child_descriptors(child_fd, control_fd));
    }
    let child = command.spawn().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not launch signed helper at {}: {error}",
            path.display()
        ))
    })?;
    drop(child_stream);
    drop(control_child);
    Ok(RawConnection {
        stream: parent,
        control_stream: Some(control_parent),
        child: Some(child),
    })
}

unsafe fn remap_child_descriptors(
    ipc_source: libc::c_int,
    control_source: libc::c_int,
) -> io::Result<()> {
    let mut ipc_source = ipc_source;
    let mut control_source = control_source;
    if ipc_source == CHILD_CONTROL_FD && control_source != CHILD_CONTROL_FD {
        ipc_source = libc::fcntl(ipc_source, libc::F_DUPFD_CLOEXEC, 5);
        if ipc_source == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    if control_source == CHILD_IPC_FD && ipc_source != CHILD_IPC_FD {
        control_source = libc::fcntl(control_source, libc::F_DUPFD_CLOEXEC, 5);
        if control_source == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    for (source, target) in [
        (ipc_source, CHILD_IPC_FD),
        (control_source, CHILD_CONTROL_FD),
    ] {
        if source != target && libc::dup2(source, target) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    for source in [ipc_source, control_source] {
        if source != CHILD_IPC_FD && source != CHILD_CONTROL_FD {
            libc::close(source);
        }
    }
    for target in [CHILD_IPC_FD, CHILD_CONTROL_FD] {
        if libc::fcntl(target, libc::F_SETFD, 0) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn helper_path() -> Result<PathBuf, ComputerUseError> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("CLARK_COMPUTER_USE_HELPER_PATH") {
        return validate_helper_path(PathBuf::from(path), None);
    }
    let executable = std::env::current_exe().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!("could not locate Clark executable: {error}"))
    })?;
    let directory = executable.parent().ok_or_else(|| {
        ComputerUseError::HelperUnavailable("Clark executable has no parent directory".to_string())
    })?;
    let expected_directory = directory.canonicalize().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not resolve Clark executable directory {}: {error}",
            directory.display()
        ))
    })?;
    validate_helper_path(directory.join(HELPER_EXECUTABLE), Some(&expected_directory))
}

fn validate_helper_path(
    path: PathBuf,
    expected_directory: Option<&std::path::Path>,
) -> Result<PathBuf, ComputerUseError> {
    let canonical = path.canonicalize().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "helper is missing at {}: {error}",
            path.display()
        ))
    })?;
    if canonical.file_name().and_then(|name| name.to_str()) != Some(HELPER_EXECUTABLE) {
        return Err(ComputerUseError::HelperRejected(format!(
            "helper path must end with {HELPER_EXECUTABLE}"
        )));
    }
    if !canonical.is_file() {
        return Err(ComputerUseError::HelperRejected(format!(
            "helper path is not a regular file: {}",
            canonical.display()
        )));
    }
    if expected_directory.is_some_and(|directory| canonical.parent() != Some(directory)) {
        return Err(ComputerUseError::HelperRejected(
            "release helper must be a real file beside the Clark executable".to_string(),
        ));
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
