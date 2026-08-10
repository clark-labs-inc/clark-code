#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use interprocess::local_socket::{prelude::*, GenericNamespaced, Stream};

use crate::{
    ActionAuthorization, ActionReceipt, CancelAck, ClickRequest, ComputerBackend, ComputerUseError,
    KeyPressRequest, Observation, PermissionRequest, PermissionStatus, PrepareActionRequest,
    PreparedAction, TypeTextRequest, WindowFilter, WindowInfo, WindowTarget,
};

use super::protocol::{
    read_control_response, read_response, write_control_request, write_request, ControlRequest,
    ControlRequestFrame, ControlResponse, Request, RequestFrame, Response, PROTOCOL_VERSION,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(unix)]
const IO_TIMEOUT: Duration = Duration::from_secs(30);

pub struct PortableServiceBackend {
    connection: Mutex<Option<Arc<Connection>>>,
}

impl PortableServiceBackend {
    pub fn new() -> Result<Self, ComputerUseError> {
        Ok(Self {
            connection: Mutex::new(None),
        })
    }

    fn connection(&self) -> Result<Arc<Connection>, ComputerUseError> {
        let mut slot = self.connection.lock().map_err(|_| {
            ComputerUseError::HelperUnavailable("service connection lock poisoned".to_string())
        })?;
        if let Some(connection) = slot.as_ref() {
            return Ok(connection.clone());
        }
        let connection = Arc::new(spawn_service()?);
        *slot = Some(connection.clone());
        Ok(connection)
    }

    fn call(&self, request: Request) -> Result<Response, ComputerUseError> {
        let connection = self.connection()?;
        match connection.call(request) {
            Ok(response) => Ok(response),
            Err(error) => {
                let _ = connection.terminate();
                if let Ok(mut slot) = self.connection.lock() {
                    if slot
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &connection))
                    {
                        *slot = None;
                    }
                }
                Err(error)
            }
        }
    }
}

impl ComputerBackend for PortableServiceBackend {
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

    fn observe(&self, target: &WindowTarget) -> Result<Observation, ComputerUseError> {
        match self.call(Request::Observe(target.clone()))? {
            Response::Observation(observation) => Ok(observation),
            response => Err(unexpected("Observation", response)),
        }
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> Result<PreparedAction, ComputerUseError> {
        match self.call(Request::PrepareAction(request))? {
            Response::PreparedAction(prepared) => Ok(prepared),
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
        match self.call(Request::CommitAction { id: id.to_string() })? {
            Response::ActionReceipt(receipt) => Ok(receipt),
            response => Err(unexpected("ActionReceipt", response)),
        }
    }

    fn cancel_active(&self) -> Result<CancelAck, ComputerUseError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| {
                ComputerUseError::HelperUnavailable("service connection lock poisoned".to_string())
            })?
            .clone();
        let Some(connection) = connection else {
            return Ok(CancelAck {
                lease_id: None,
                quiesced: true,
                helper_terminated: false,
            });
        };
        match connection.cancel() {
            Ok(ack) if ack.quiesced => Ok(ack),
            Ok(mut ack) => {
                ack.helper_terminated = connection.terminate()?;
                ack.quiesced = ack.helper_terminated;
                Ok(ack)
            }
            Err(error) => {
                if connection.terminate()? {
                    Ok(CancelAck {
                        lease_id: None,
                        quiesced: true,
                        helper_terminated: true,
                    })
                } else {
                    Err(error)
                }
            }
        }
    }

    fn click(&self, request: ClickRequest) -> Result<(), ComputerUseError> {
        expect_unit(self.call(Request::Click(request))?)
    }

    fn type_text(&self, request: TypeTextRequest) -> Result<(), ComputerUseError> {
        expect_unit(self.call(Request::TypeText(request))?)
    }

    fn keypress(&self, request: KeyPressRequest) -> Result<(), ComputerUseError> {
        expect_unit(self.call(Request::KeyPress(request))?)
    }
}

struct Connection {
    primary: Mutex<Stream>,
    control: Mutex<Stream>,
    next_request_id: Mutex<u64>,
    next_control_id: Mutex<u64>,
    session_id: String,
    service_pid: u32,
    service_path: PathBuf,
    child: Mutex<Option<Child>>,
}

impl Connection {
    fn call(&self, request: Request) -> Result<Response, ComputerUseError> {
        let mut request_id = self.next_request_id.lock().map_err(|_| {
            ComputerUseError::HelperProtocol("request counter poisoned".to_string())
        })?;
        let current = *request_id;
        *request_id = request_id.saturating_add(1);
        let mut stream = self
            .primary
            .lock()
            .map_err(|_| ComputerUseError::HelperProtocol("primary stream poisoned".to_string()))?;
        write_request(
            &mut *stream,
            &RequestFrame {
                protocol_version: PROTOCOL_VERSION,
                session_id: self.session_id.clone(),
                request_id: current,
                body: request,
            },
        )
        .map_err(protocol_error)?;
        let response = read_response(&mut *stream).map_err(protocol_error)?;
        validate_response(
            response.protocol_version,
            &response.session_id,
            response.request_id,
            &self.session_id,
            current,
        )?;
        response
            .body
            .map_err(super::protocol::RemoteError::into_local)
    }

    fn cancel(&self) -> Result<CancelAck, ComputerUseError> {
        let mut request_id = self.next_control_id.lock().map_err(|_| {
            ComputerUseError::HelperProtocol("control request counter poisoned".to_string())
        })?;
        let current = *request_id;
        *request_id = request_id.saturating_add(1);
        let mut stream = self
            .control
            .lock()
            .map_err(|_| ComputerUseError::HelperProtocol("control stream poisoned".to_string()))?;
        write_control_request(
            &mut *stream,
            &ControlRequestFrame {
                protocol_version: PROTOCOL_VERSION,
                session_id: self.session_id.clone(),
                request_id: current,
                body: ControlRequest::CancelActive,
            },
        )
        .map_err(protocol_error)?;
        let response = read_control_response(&mut *stream).map_err(protocol_error)?;
        validate_response(
            response.protocol_version,
            &response.session_id,
            response.request_id,
            &self.session_id,
            current,
        )?;
        match response
            .body
            .map_err(super::protocol::RemoteError::into_local)?
        {
            ControlResponse::CancelAck(ack) => Ok(ack),
            ControlResponse::Hello => Err(ComputerUseError::HelperProtocol(
                "unexpected control Hello response".to_string(),
            )),
        }
    }

    fn terminate(&self) -> Result<bool, ComputerUseError> {
        let primary = self
            .primary
            .lock()
            .map_err(|_| ComputerUseError::HelperProtocol("primary stream poisoned".to_string()))?;
        super::auth::verify_service_peer(&primary, self.service_pid, &self.service_path)?;
        drop(primary);
        let mut child = self.child.lock().map_err(|_| {
            ComputerUseError::HelperUnavailable("service process lock poisoned".to_string())
        })?;
        let Some(child) = child.as_mut() else {
            return Ok(false);
        };
        match child.try_wait() {
            Ok(Some(_)) => Ok(true),
            Ok(None) => {
                child
                    .kill()
                    .map_err(|error| ComputerUseError::HelperUnavailable(error.to_string()))?;
                let _ = child.wait();
                Ok(true)
            }
            Err(error) => Err(ComputerUseError::HelperUnavailable(error.to_string())),
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn spawn_service() -> Result<Connection, ComputerUseError> {
    let service_path = locate_service()?;
    let socket_name = format!(
        "agent-cua-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    );
    let data_dir = data_dir()?;
    std::fs::create_dir_all(&data_dir)
        .map_err(|error| ComputerUseError::HelperUnavailable(error.to_string()))?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| ComputerUseError::HelperUnavailable(error.to_string()))?;
    }
    let mut command = Command::new(&service_path);
    command
        .args([
            "--socket-name",
            &socket_name,
            "--data-dir",
            &data_dir.to_string_lossy(),
            "--client-pid",
            &std::process::id().to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::suppress_portable_console_window(&mut command);
    let mut child = command.spawn().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not launch {}: {error}",
            service_path.display()
        ))
    })?;
    let service_pid = child.id();
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .map_err(protocol_error)?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let primary = loop {
        match Stream::connect(name.clone()) {
            Ok(stream) => break stream,
            Err(error) => {
                if child.try_wait().ok().flatten().is_some() || Instant::now() >= deadline {
                    let _ = child.kill();
                    return Err(ComputerUseError::HelperUnavailable(format!(
                        "service did not open authenticated IPC: {}",
                        error
                    )));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };
    let control = Stream::connect(name).map_err(protocol_error)?;
    configure_timeouts(&primary)?;
    configure_timeouts(&control)?;
    super::auth::verify_service_peer(&primary, service_pid, &service_path)?;
    super::auth::verify_service_peer(&control, service_pid, &service_path)?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let mut primary = primary;
    write_request(
        &mut primary,
        &RequestFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: session_id.clone(),
            request_id: 0,
            body: Request::Hello {
                client_pid: std::process::id(),
            },
        },
    )
    .map_err(protocol_error)?;
    let hello = read_response(&mut primary).map_err(protocol_error)?;
    validate_response(
        hello.protocol_version,
        &hello.session_id,
        hello.request_id,
        &session_id,
        0,
    )?;
    match hello
        .body
        .map_err(super::protocol::RemoteError::into_local)?
    {
        Response::Hello {
            service_pid: claimed,
        } if claimed == service_pid => {}
        Response::Hello {
            service_pid: claimed,
        } => {
            return Err(ComputerUseError::HelperRejected(format!(
                "service claimed PID {claimed}, transport has PID {service_pid}"
            )));
        }
        response => return Err(unexpected("Hello", response)),
    }

    let mut control = control;
    write_control_request(
        &mut control,
        &ControlRequestFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: session_id.clone(),
            request_id: 0,
            body: ControlRequest::Hello {
                client_pid: std::process::id(),
            },
        },
    )
    .map_err(protocol_error)?;
    let control_hello = read_control_response(&mut control).map_err(protocol_error)?;
    validate_response(
        control_hello.protocol_version,
        &control_hello.session_id,
        control_hello.request_id,
        &session_id,
        0,
    )?;
    if !matches!(control_hello.body, Ok(ControlResponse::Hello)) {
        return Err(ComputerUseError::HelperRejected(
            "service rejected the control channel".to_string(),
        ));
    }

    Ok(Connection {
        primary: Mutex::new(primary),
        control: Mutex::new(control),
        next_request_id: Mutex::new(1),
        next_control_id: Mutex::new(1),
        session_id,
        service_pid,
        service_path,
        child: Mutex::new(Some(child)),
    })
}

fn locate_service() -> Result<PathBuf, ComputerUseError> {
    if let Some(path) = std::env::var_os("DESKTOP_COMPUTER_USE_SERVICE_PATH") {
        return verify_service_path(PathBuf::from(path));
    }
    let executable = std::env::current_exe()
        .map_err(|error| ComputerUseError::HelperUnavailable(error.to_string()))?;
    let directory = executable.parent().ok_or_else(|| {
        ComputerUseError::HelperUnavailable("current executable has no parent".to_string())
    })?;
    let filename = if cfg!(target_os = "windows") {
        "agent-computer-use-helper.exe"
    } else {
        "agent-computer-use-helper"
    };
    let candidates = [
        directory.join(filename),
        directory
            .join("agent-resources")
            .join("computer-use")
            .join(filename),
        directory
            .join("resources")
            .join("agent-resources")
            .join("computer-use")
            .join(filename),
        directory
            .join("..")
            .join("lib")
            .join("agent-desktop")
            .join("agent-resources")
            .join("computer-use")
            .join(filename),
        directory
            .join("..")
            .join("lib")
            .join("Clark Code")
            .join("agent-resources")
            .join("computer-use")
            .join(filename),
        directory
            .join("..")
            .join("lib")
            .join("agent-desktop-dev")
            .join("agent-resources")
            .join("computer-use")
            .join(filename),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return verify_service_path(candidate);
        }
    }
    if let Some(app_dir) = std::env::var_os("APPDIR") {
        for product in ["agent-desktop", "Clark Code", "agent-desktop-dev"] {
            let candidate = PathBuf::from(&app_dir)
                .join("usr")
                .join("lib")
                .join(product)
                .join("agent-resources")
                .join("computer-use")
                .join(filename);
            if candidate.is_file() {
                return verify_service_path(candidate);
            }
        }
    }
    Err(ComputerUseError::HelperUnavailable(format!(
        "could not find the separately packaged Computer Use service beside {}",
        executable.display()
    )))
}

fn verify_service_path(path: PathBuf) -> Result<PathBuf, ComputerUseError> {
    let canonical = std::fs::canonicalize(&path).map_err(|error| {
        ComputerUseError::HelperUnavailable(format!("{}: {error}", path.display()))
    })?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| ComputerUseError::HelperUnavailable(error.to_string()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(ComputerUseError::HelperUnavailable(format!(
            "{} is not a service executable",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn data_dir() -> Result<PathBuf, ComputerUseError> {
    if let Some(path) = std::env::var_os("DESKTOP_COMPUTER_USE_DATA_DIR") {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Ok(path);
        }
        return Err(ComputerUseError::HelperUnavailable(
            "DESKTOP_COMPUTER_USE_DATA_DIR must be absolute".to_string(),
        ));
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| Path::new(&home).join(".local").join("share"))
        });
    base.map(|path| path.join("Clark Code").join("Computer Use"))
        .ok_or_else(|| {
            ComputerUseError::HelperUnavailable(
                "could not resolve a per-user Computer Use data directory".to_string(),
            )
        })
}

fn validate_response(
    version: u16,
    session: &str,
    request_id: u64,
    expected_session: &str,
    expected_request_id: u64,
) -> Result<(), ComputerUseError> {
    if version != PROTOCOL_VERSION
        || session != expected_session
        || request_id != expected_request_id
    {
        return Err(ComputerUseError::HelperProtocol(
            "response envelope version, session, or request identity is invalid".to_string(),
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
    ComputerUseError::HelperProtocol(format!("expected {expected} response, got {response:?}"))
}

fn protocol_error(error: std::io::Error) -> ComputerUseError {
    ComputerUseError::HelperProtocol(error.to_string())
}

#[cfg(unix)]
fn configure_timeouts(stream: &Stream) -> Result<(), ComputerUseError> {
    stream
        .set_recv_timeout(Some(IO_TIMEOUT))
        .map_err(protocol_error)?;
    stream
        .set_send_timeout(Some(IO_TIMEOUT))
        .map_err(protocol_error)
}

#[cfg(windows)]
fn configure_timeouts(_stream: &Stream) -> Result<(), ComputerUseError> {
    // Windows named pipes do not expose socket-style I/O timeouts. The
    // dedicated service process remains the fail-closed cancellation boundary.
    Ok(())
}
