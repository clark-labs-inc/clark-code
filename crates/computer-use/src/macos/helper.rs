use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use crate::{ComputerBackend, ComputerUseError};

use super::protocol::{
    read_control_request, read_request, write_control_response, write_response, ControlRequest,
    ControlResponse, ControlResponseFrame, RemoteError, Request, RequestFrame, Response,
    ResponseFrame, PROTOCOL_VERSION,
};
use super::service::MacServiceBackend;

const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_SESSION_ID_CHARS: usize = 128;
const MAX_BUNDLE_ID_CHARS: usize = 255;
const MAX_TITLE_FILTER_CHARS: usize = 500;
const MAX_OBSERVATION_ID_CHARS: usize = 128;
const MAX_ELEMENT_ID_CHARS: usize = 128;
const MAX_TEXT_INPUT_CHARS: usize = 20_000;
const MAX_ACTION_ID_CHARS: usize = 128;
const MAX_SECONDARY_ACTION_CHARS: usize = 128;

pub fn run(socket_path: PathBuf, data_dir: PathBuf) -> Result<(), ComputerUseError> {
    if !socket_path.is_absolute() {
        return Err(ComputerUseError::HelperProtocol(
            "computer-use service socket path must be absolute".to_string(),
        ));
    }
    if !data_dir.is_absolute() {
        return Err(ComputerUseError::HelperProtocol(
            "computer-use data directory must be absolute".to_string(),
        ));
    }
    super::auth::verify_service_signature().map_err(ComputerUseError::HelperRejected)?;
    if socket_path.exists() {
        fs::remove_file(&socket_path).map_err(|error| {
            ComputerUseError::HelperUnavailable(format!(
                "could not remove stale service socket {}: {error}",
                socket_path.display()
            ))
        })?;
    }
    let listener = UnixListener::bind(&socket_path).map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not bind service socket {}: {error}",
            socket_path.display()
        ))
    })?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not restrict service socket {}: {error}",
            socket_path.display()
        ))
    })?;
    let _cleanup = SocketCleanup(socket_path);
    let (stream, _) = listener.accept().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not accept primary service connection: {error}"
        ))
    })?;
    let (control, _) = listener.accept().map_err(|error| {
        ComputerUseError::HelperUnavailable(format!(
            "could not accept control service connection: {error}"
        ))
    })?;
    run_session(stream, control, data_dir)
}

fn run_session(
    mut stream: UnixStream,
    control: UnixStream,
    data_dir: PathBuf,
) -> Result<(), ComputerUseError> {
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .map_err(|error| ComputerUseError::HelperProtocol(error.to_string()))?;

    let hello = read_request(&mut stream).map_err(protocol_error)?;
    validate_envelope(&hello, None, 0)?;
    let Request::Hello { parent_pid } = hello.body else {
        return Err(ComputerUseError::HelperProtocol(
            "the first IPC request must be Hello".to_string(),
        ));
    };
    let session_id = hello.session_id.clone();
    if let Err(message) = super::auth::verify_client_peer(parent_pid, stream.as_raw_fd()) {
        let _ = write_response(
            &mut stream,
            &ResponseFrame {
                protocol_version: PROTOCOL_VERSION,
                session_id,
                request_id: hello.request_id,
                body: Err(RemoteError {
                    message: message.clone(),
                }),
            },
        );
        return Err(ComputerUseError::HelperRejected(message));
    }
    let backend = MacServiceBackend::new(crate::ApprovalStore::new(data_dir));
    let control_backend = backend.clone();
    let control_session = session_id.clone();
    std::thread::Builder::new()
        .name("clark-computer-use-control".to_string())
        .spawn(move || {
            let _ = run_control(control, control_backend, control_session, parent_pid);
        })
        .map_err(|error| {
            ComputerUseError::HelperUnavailable(format!(
                "could not start cancellation control channel: {error}"
            ))
        })?;

    write_response(
        &mut stream,
        &ResponseFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: session_id.clone(),
            request_id: hello.request_id,
            body: Ok(Response::Hello {
                helper_pid: std::process::id(),
            }),
        },
    )
    .map_err(protocol_error)?;

    let mut previous_request_id = hello.request_id;
    loop {
        let frame = match read_request(&mut stream) {
            Ok(frame) => frame,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(protocol_error(error)),
        };
        validate_envelope(&frame, Some(&session_id), previous_request_id + 1)?;
        previous_request_id = frame.request_id;
        let body = validate_request(&frame.body)
            .and_then(|()| execute(&backend, frame.body))
            .map_err(|error| RemoteError {
                message: error.to_string(),
            });
        write_response(
            &mut stream,
            &ResponseFrame {
                protocol_version: PROTOCOL_VERSION,
                session_id: session_id.clone(),
                request_id: frame.request_id,
                body,
            },
        )
        .map_err(protocol_error)?;
    }
}

struct SocketCleanup(PathBuf);

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn validate_envelope(
    frame: &RequestFrame,
    expected_session: Option<&str>,
    expected_request_id: u64,
) -> Result<(), ComputerUseError> {
    if frame.protocol_version != PROTOCOL_VERSION {
        return Err(ComputerUseError::HelperProtocol(format!(
            "unsupported protocol version {}; expected {PROTOCOL_VERSION}",
            frame.protocol_version
        )));
    }
    if frame.session_id.is_empty() || frame.session_id.chars().count() > MAX_SESSION_ID_CHARS {
        return Err(ComputerUseError::HelperProtocol(
            "session id is empty or oversized".to_string(),
        ));
    }
    if expected_session.is_some_and(|expected| frame.session_id != expected) {
        return Err(ComputerUseError::HelperProtocol(
            "session id changed within one IPC connection".to_string(),
        ));
    }
    if frame.request_id != expected_request_id {
        return Err(ComputerUseError::HelperProtocol(format!(
            "request id {} is not the expected monotonic id {expected_request_id}",
            frame.request_id
        )));
    }
    Ok(())
}

fn validate_request(request: &Request) -> Result<(), ComputerUseError> {
    match request {
        Request::Hello { .. } => {
            return Err(ComputerUseError::HelperProtocol(
                "Hello may only appear as the first request".to_string(),
            ))
        }
        Request::Permissions => {}
        Request::PromptPermissions(request) => {
            if !request.accessibility && !request.screen_recording {
                return Err(ComputerUseError::HelperProtocol(
                    "a permission request must select Accessibility or Screen Recording"
                        .to_string(),
                ));
            }
        }
        Request::ListWindows(filter) => {
            if let Some(bundle_id) = filter.bundle_id.as_deref() {
                validate_bundle(bundle_id)?;
            }
            if filter
                .title_contains
                .as_deref()
                .is_some_and(|title| title.chars().count() > MAX_TITLE_FILTER_CHARS)
            {
                return Err(ComputerUseError::HelperProtocol(
                    "window title filter is oversized".to_string(),
                ));
            }
        }
        Request::LaunchApplication { bundle_id } => validate_bundle(bundle_id)?,
        Request::Observe(target) => validate_target(target)?,
        Request::PrepareAction(request) => validate_prepare_action(request)?,
        Request::PreparedAction { id }
        | Request::AuthorizeAction { id, .. }
        | Request::CommitAction { id } => validate_action_id(id)?,
        Request::Click(request) => {
            validate_target(&request.window)?;
            validate_observation_id(&request.observation_id)?;
            crate::validate_intent_shape(&request.intent)?;
            match (&request.element_id, request.point) {
                (Some(element_id), None) => validate_element_id(element_id)?,
                (None, Some(point)) if point.x.is_finite() && point.y.is_finite() => {}
                _ => {
                    return Err(ComputerUseError::HelperProtocol(
                        "click must provide exactly one element id or finite point".to_string(),
                    ))
                }
            }
        }
        Request::TypeText(request) => {
            validate_target(&request.window)?;
            validate_observation_id(&request.observation_id)?;
            validate_element_id(&request.element_id)?;
            crate::validate_intent_shape(&request.intent)?;
            if request.text.chars().count() > MAX_TEXT_INPUT_CHARS {
                return Err(ComputerUseError::HelperProtocol(
                    "text input is oversized".to_string(),
                ));
            }
        }
        Request::KeyPress(request) => {
            validate_target(&request.window)?;
            validate_observation_id(&request.observation_id)?;
            crate::validate_intent_shape(&request.intent)?;
            if request.modifiers.len() > 4 {
                return Err(ComputerUseError::HelperProtocol(
                    "keypress has too many modifiers".to_string(),
                ));
            }
            for (index, modifier) in request.modifiers.iter().enumerate() {
                if request.modifiers[..index].contains(modifier) {
                    return Err(ComputerUseError::HelperProtocol(
                        "keypress contains duplicate modifiers".to_string(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_prepare_action(request: &crate::PrepareActionRequest) -> Result<(), ComputerUseError> {
    validate_target(&request.window)?;
    validate_observation_id(&request.observation_id)?;
    crate::validate_intent_shape(&request.intent)?;
    match &request.action {
        crate::ComputerAction::Click {
            element_id, point, ..
        } => match (element_id, point) {
            (Some(element_id), None) => validate_element_id(element_id)?,
            (None, Some(point)) if point.x.is_finite() && point.y.is_finite() => {}
            _ => {
                return Err(ComputerUseError::HelperProtocol(
                    "click must provide exactly one element id or finite point".to_string(),
                ))
            }
        },
        crate::ComputerAction::TypeText {
            element_id, text, ..
        } => {
            validate_element_id(element_id)?;
            if text.chars().count() > MAX_TEXT_INPUT_CHARS {
                return Err(ComputerUseError::HelperProtocol(
                    "text input is oversized".to_string(),
                ));
            }
        }
        crate::ComputerAction::Keypress { modifiers, .. } => {
            validate_modifiers(modifiers)?;
        }
        crate::ComputerAction::Scroll {
            element_id,
            delta_x,
            delta_y,
        } => {
            if let Some(element_id) = element_id {
                validate_element_id(element_id)?;
            }
            if *delta_x == 0 && *delta_y == 0 {
                return Err(ComputerUseError::HelperProtocol(
                    "scroll delta must not be zero".to_string(),
                ));
            }
        }
        crate::ComputerAction::Drag {
            start,
            end,
            duration_ms,
            ..
        } => {
            validate_location(start)?;
            validate_location(end)?;
            if *duration_ms == 0 {
                return Err(ComputerUseError::HelperProtocol(
                    "drag duration must be positive".to_string(),
                ));
            }
        }
        crate::ComputerAction::SecondaryAction { element_id, action } => {
            validate_element_id(element_id)?;
            if action.is_empty() || action.chars().count() > MAX_SECONDARY_ACTION_CHARS {
                return Err(ComputerUseError::HelperProtocol(
                    "secondary Accessibility action is empty or oversized".to_string(),
                ));
            }
        }
        crate::ComputerAction::SelectText {
            element_id,
            start,
            end,
        } => {
            validate_element_id(element_id)?;
            if start > end {
                return Err(ComputerUseError::HelperProtocol(
                    "text selection start must not exceed end".to_string(),
                ));
            }
        }
        crate::ComputerAction::SetValue { element_id, value } => {
            validate_element_id(element_id)?;
            if !value.is_finite() {
                return Err(ComputerUseError::HelperProtocol(
                    "numeric value must be finite".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_location(location: &crate::ActionLocation) -> Result<(), ComputerUseError> {
    match (&location.element_id, location.point) {
        (Some(element_id), None) => validate_element_id(element_id),
        (None, Some(point)) if point.x.is_finite() && point.y.is_finite() => Ok(()),
        _ => Err(ComputerUseError::HelperProtocol(
            "action location must provide exactly one element id or finite point".to_string(),
        )),
    }
}

fn validate_modifiers(modifiers: &[crate::Modifier]) -> Result<(), ComputerUseError> {
    if modifiers.len() > 4 {
        return Err(ComputerUseError::HelperProtocol(
            "keypress has too many modifiers".to_string(),
        ));
    }
    for (index, modifier) in modifiers.iter().enumerate() {
        if modifiers[..index].contains(modifier) {
            return Err(ComputerUseError::HelperProtocol(
                "keypress contains duplicate modifiers".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_action_id(id: &str) -> Result<(), ComputerUseError> {
    if id.is_empty() || id.chars().count() > MAX_ACTION_ID_CHARS {
        return Err(ComputerUseError::HelperProtocol(
            "prepared action id is empty or oversized".to_string(),
        ));
    }
    Ok(())
}

fn validate_target(target: &crate::WindowTarget) -> Result<(), ComputerUseError> {
    if target.pid <= 0 || target.window_id == 0 {
        return Err(ComputerUseError::HelperProtocol(
            "target pid and window id must both be positive".to_string(),
        ));
    }
    validate_bundle(&target.bundle_id)
}

fn validate_bundle(bundle_id: &str) -> Result<(), ComputerUseError> {
    if bundle_id.is_empty() || bundle_id.chars().count() > MAX_BUNDLE_ID_CHARS {
        return Err(ComputerUseError::HelperProtocol(
            "bundle id is empty or oversized".to_string(),
        ));
    }
    crate::ensure_bundle_allowed(bundle_id)
}

fn validate_observation_id(observation_id: &str) -> Result<(), ComputerUseError> {
    if observation_id.is_empty() || observation_id.chars().count() > MAX_OBSERVATION_ID_CHARS {
        return Err(ComputerUseError::HelperProtocol(
            "observation id is empty or oversized".to_string(),
        ));
    }
    Ok(())
}

fn validate_element_id(element_id: &str) -> Result<(), ComputerUseError> {
    if element_id.is_empty() || element_id.chars().count() > MAX_ELEMENT_ID_CHARS {
        return Err(ComputerUseError::HelperProtocol(
            "element id is empty or oversized".to_string(),
        ));
    }
    Ok(())
}

fn execute(backend: &MacServiceBackend, request: Request) -> Result<Response, ComputerUseError> {
    Ok(match request {
        Request::Hello { .. } => unreachable!("validated above"),
        Request::Permissions => Response::Permissions(backend.permissions()?),
        Request::PromptPermissions(request) => {
            Response::Permissions(backend.request_permissions(request)?)
        }
        Request::ListWindows(filter) => Response::Windows(backend.list_windows(filter)?),
        Request::LaunchApplication { bundle_id } => {
            backend.launch_application(&bundle_id)?;
            Response::Unit
        }
        Request::Observe(target) => Response::Observation(backend.observe(&target)?),
        Request::PrepareAction(request) => {
            Response::PreparedAction(backend.prepare_action(request)?)
        }
        Request::PreparedAction { id } => Response::PreparedAction(backend.prepared_action(&id)?),
        Request::AuthorizeAction { id, authorization } => {
            backend.authorize_action(&id, authorization)?;
            Response::Unit
        }
        Request::CommitAction { id } => Response::ActionReceipt(backend.commit_action(&id)?),
        Request::Click(request) => {
            backend.click(request)?;
            Response::Unit
        }
        Request::TypeText(request) => {
            backend.type_text(request)?;
            Response::Unit
        }
        Request::KeyPress(request) => {
            backend.keypress(request)?;
            Response::Unit
        }
    })
}

fn run_control(
    mut stream: UnixStream,
    backend: MacServiceBackend,
    expected_session: String,
    expected_parent_pid: u32,
) -> Result<(), ComputerUseError> {
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .map_err(protocol_error)?;
    let hello = read_control_request(&mut stream).map_err(protocol_error)?;
    validate_control_envelope(&hello, &expected_session, 0)?;
    let ControlRequest::Hello { parent_pid } = hello.body else {
        return Err(ComputerUseError::HelperProtocol(
            "the first control request must be Hello".to_string(),
        ));
    };
    if parent_pid != expected_parent_pid {
        return Err(ComputerUseError::HelperRejected(
            "control channel parent identity changed".to_string(),
        ));
    }
    super::auth::verify_client_peer(parent_pid, stream.as_raw_fd())
        .map_err(ComputerUseError::HelperRejected)?;
    write_control_response(
        &mut stream,
        &ControlResponseFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: expected_session.clone(),
            request_id: hello.request_id,
            body: Ok(ControlResponse::Hello),
        },
    )
    .map_err(protocol_error)?;

    let mut previous_request_id = hello.request_id;
    loop {
        let frame = match read_control_request(&mut stream) {
            Ok(frame) => frame,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(protocol_error(error)),
        };
        validate_control_envelope(
            &frame,
            &expected_session,
            previous_request_id.saturating_add(1),
        )?;
        previous_request_id = frame.request_id;
        let body = match frame.body {
            ControlRequest::Hello { .. } => Err(RemoteError {
                message: "Hello may only appear as the first control request".to_string(),
            }),
            ControlRequest::CancelActive => backend
                .cancel_active()
                .map(ControlResponse::CancelAck)
                .map_err(|error| RemoteError {
                    message: error.to_string(),
                }),
        };
        write_control_response(
            &mut stream,
            &ControlResponseFrame {
                protocol_version: PROTOCOL_VERSION,
                session_id: expected_session.clone(),
                request_id: frame.request_id,
                body,
            },
        )
        .map_err(protocol_error)?;
    }
}

fn validate_control_envelope(
    frame: &super::protocol::ControlRequestFrame,
    expected_session: &str,
    expected_request_id: u64,
) -> Result<(), ComputerUseError> {
    if frame.protocol_version != PROTOCOL_VERSION
        || frame.session_id != expected_session
        || frame.request_id != expected_request_id
    {
        return Err(ComputerUseError::HelperProtocol(
            "control envelope version, session, or request identity is invalid".to_string(),
        ));
    }
    Ok(())
}

fn protocol_error(error: std::io::Error) -> ComputerUseError {
    ComputerUseError::HelperProtocol(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionIntent, ActionRisk, MouseButton, Point, WindowTarget};

    fn target(bundle_id: &str) -> WindowTarget {
        WindowTarget {
            pid: 42,
            window_id: 7,
            bundle_id: bundle_id.to_string(),
        }
    }

    #[test]
    fn malformed_and_forbidden_requests_are_rejected_before_native_apis() {
        assert!(validate_request(&Request::Observe(target("com.apple.Terminal"))).is_err());
        assert!(validate_request(&Request::Observe(WindowTarget {
            pid: 0,
            ..target("com.apple.Safari")
        }))
        .is_err());
        assert!(validate_request(&Request::Click(crate::ClickRequest {
            intent: ActionIntent {
                risk: ActionRisk::Ambiguous,
                reason: "test".to_string(),
            },
            window: target("com.apple.Safari"),
            observation_id: "obs".to_string(),
            element_id: Some("ax-1".to_string()),
            point: Some(Point { x: 1.0, y: 1.0 }),
            button: MouseButton::Left,
            dry_run: true,
        }))
        .is_err());
        assert!(validate_request(&Request::PromptPermissions(
            crate::PermissionRequest::default()
        ))
        .is_err());
        assert!(validate_request(&Request::Click(crate::ClickRequest {
            intent: ActionIntent {
                risk: ActionRisk::Ambiguous,
                reason: String::new(),
            },
            window: target("com.apple.Safari"),
            observation_id: "obs".to_string(),
            element_id: Some("ax-1".to_string()),
            point: None,
            button: MouseButton::Left,
            dry_run: true,
        }))
        .is_err());
    }

    #[test]
    fn envelope_requires_exact_version_session_and_monotonic_ids() {
        let valid = RequestFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: "session".to_string(),
            request_id: 1,
            body: Request::Permissions,
        };
        validate_envelope(&valid, Some("session"), 1).unwrap();

        let wrong_version = RequestFrame {
            protocol_version: PROTOCOL_VERSION + 1,
            ..valid
        };
        assert!(validate_envelope(&wrong_version, Some("session"), 1).is_err());
    }
}
