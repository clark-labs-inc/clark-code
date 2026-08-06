use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};

use crate::{ComputerBackend, ComputerUseError};

use super::backend::PortableNativeBackend;
use super::protocol::{
    read_control_request, read_request, write_control_response, write_response, ControlRequest,
    ControlResponse, ControlResponseFrame, RemoteError, Request, RequestFrame, Response,
    ResponseFrame, PROTOCOL_VERSION,
};

#[cfg(unix)]
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ID_CHARS: usize = 128;
const MAX_BUNDLE_CHARS: usize = 255;
const MAX_TITLE_CHARS: usize = 500;
const MAX_TEXT_CHARS: usize = 20_000;

pub fn run(
    socket_name: String,
    data_dir: PathBuf,
    expected_client_pid: u32,
) -> Result<(), ComputerUseError> {
    if socket_name.is_empty() || socket_name.chars().count() > MAX_ID_CHARS {
        return Err(ComputerUseError::HelperProtocol(
            "service socket name is empty or oversized".to_string(),
        ));
    }
    if !data_dir.is_absolute() {
        return Err(ComputerUseError::HelperProtocol(
            "computer-use data directory must be absolute".to_string(),
        ));
    }
    super::auth::verify_own_executable()?;
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .map_err(protocol_error)?;
    let listener = ListenerOptions::new()
        .name(name)
        .create_sync()
        .map_err(protocol_error)?;
    let mut incoming = listener.incoming();
    let primary = incoming
        .next()
        .ok_or_else(|| {
            ComputerUseError::HelperUnavailable(
                "service listener closed before primary connection".to_string(),
            )
        })?
        .map_err(protocol_error)?;
    let control = incoming
        .next()
        .ok_or_else(|| {
            ComputerUseError::HelperUnavailable(
                "service listener closed before control connection".to_string(),
            )
        })?
        .map_err(protocol_error)?;
    super::auth::authenticate_client(&primary, expected_client_pid)?;
    super::auth::authenticate_client(&control, expected_client_pid)?;
    run_session(primary, control, data_dir, expected_client_pid)
}

fn run_session(
    mut primary: Stream,
    control: Stream,
    data_dir: PathBuf,
    expected_client_pid: u32,
) -> Result<(), ComputerUseError> {
    configure_timeouts(&primary)?;
    let hello = read_request(&mut primary).map_err(protocol_error)?;
    validate_envelope(&hello, None, 0)?;
    let Request::Hello { client_pid } = hello.body else {
        return Err(ComputerUseError::HelperProtocol(
            "the first IPC request must be Hello".to_string(),
        ));
    };
    if client_pid != expected_client_pid {
        return Err(ComputerUseError::HelperRejected(
            "client identity changed after transport authentication".to_string(),
        ));
    }
    let session_id = hello.session_id.clone();
    let backend = PortableNativeBackend::new(crate::ApprovalStore::new(data_dir));
    let control_backend = backend.clone();
    let control_session = session_id.clone();
    std::thread::Builder::new()
        .name("clark-computer-use-control".to_string())
        .spawn(move || {
            let _ = run_control(
                control,
                control_backend,
                control_session,
                expected_client_pid,
            );
        })
        .map_err(|error| ComputerUseError::HelperUnavailable(error.to_string()))?;

    write_response(
        &mut primary,
        &ResponseFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: session_id.clone(),
            request_id: hello.request_id,
            body: Ok(Response::Hello {
                service_pid: std::process::id(),
            }),
        },
    )
    .map_err(protocol_error)?;

    let mut previous_request_id = hello.request_id;
    loop {
        let frame = match read_request(&mut primary) {
            Ok(frame) => frame,
            Err(error) if disconnected(&error) => return Ok(()),
            Err(error) => return Err(protocol_error(error)),
        };
        validate_envelope(
            &frame,
            Some(&session_id),
            previous_request_id.saturating_add(1),
        )?;
        previous_request_id = frame.request_id;
        let body = validate_request(&frame.body)
            .and_then(|()| execute(&backend, frame.body))
            .map_err(RemoteError::from);
        write_response(
            &mut primary,
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

fn run_control(
    mut stream: Stream,
    backend: PortableNativeBackend,
    expected_session: String,
    expected_client_pid: u32,
) -> Result<(), ComputerUseError> {
    configure_timeouts(&stream)?;
    let hello = read_control_request(&mut stream).map_err(protocol_error)?;
    validate_control_envelope(&hello, &expected_session, 0)?;
    let ControlRequest::Hello { client_pid } = hello.body else {
        return Err(ComputerUseError::HelperProtocol(
            "the first control request must be Hello".to_string(),
        ));
    };
    if client_pid != expected_client_pid {
        return Err(ComputerUseError::HelperRejected(
            "control channel client identity changed".to_string(),
        ));
    }
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
            Err(error) if disconnected(&error) => return Ok(()),
            Err(error) => return Err(protocol_error(error)),
        };
        validate_control_envelope(
            &frame,
            &expected_session,
            previous_request_id.saturating_add(1),
        )?;
        previous_request_id = frame.request_id;
        let body = match frame.body {
            ControlRequest::Hello { .. } => Err(RemoteError::from(
                ComputerUseError::HelperProtocol("Hello may only appear first".to_string()),
            )),
            ControlRequest::CancelActive => backend
                .cancel_active()
                .map(ControlResponse::CancelAck)
                .map_err(RemoteError::from),
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

fn execute(
    backend: &PortableNativeBackend,
    request: Request,
) -> Result<Response, ComputerUseError> {
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

fn validate_request(request: &Request) -> Result<(), ComputerUseError> {
    match request {
        Request::Hello { .. } => {
            return Err(ComputerUseError::HelperProtocol(
                "Hello may only appear first".to_string(),
            ));
        }
        Request::Permissions => {}
        Request::PromptPermissions(request) => {
            if !request.accessibility && !request.screen_recording {
                return Err(ComputerUseError::HelperProtocol(
                    "permission request is empty".to_string(),
                ));
            }
        }
        Request::ListWindows(filter) => {
            if let Some(bundle) = filter.bundle_id.as_deref() {
                validate_bundle(bundle)?;
            }
            if filter
                .title_contains
                .as_ref()
                .is_some_and(|title| title.chars().count() > MAX_TITLE_CHARS)
            {
                return Err(ComputerUseError::HelperProtocol(
                    "title filter is oversized".to_string(),
                ));
            }
        }
        Request::LaunchApplication { bundle_id } => validate_bundle(bundle_id)?,
        Request::Observe(target) => validate_target(target)?,
        Request::PrepareAction(request) => {
            validate_target(&request.window)?;
            validate_id(&request.observation_id, "observation")?;
            crate::validate_intent_shape(&request.intent)?;
            validate_action(&request.action)?;
        }
        Request::PreparedAction { id }
        | Request::AuthorizeAction { id, .. }
        | Request::CommitAction { id } => validate_id(id, "prepared action")?,
        Request::Click(request) => {
            validate_target(&request.window)?;
            validate_id(&request.observation_id, "observation")?;
            crate::validate_intent_shape(&request.intent)?;
            validate_action(&crate::ComputerAction::Click {
                element_id: request.element_id.clone(),
                point: request.point,
                button: request.button,
            })?;
        }
        Request::TypeText(request) => {
            validate_target(&request.window)?;
            validate_id(&request.observation_id, "observation")?;
            validate_id(&request.element_id, "element")?;
            crate::validate_intent_shape(&request.intent)?;
            if request.text.chars().count() > MAX_TEXT_CHARS {
                return Err(ComputerUseError::HelperProtocol(
                    "text input is oversized".to_string(),
                ));
            }
        }
        Request::KeyPress(request) => {
            validate_target(&request.window)?;
            validate_id(&request.observation_id, "observation")?;
            crate::validate_intent_shape(&request.intent)?;
            validate_modifiers(&request.modifiers)?;
        }
    }
    Ok(())
}

fn validate_action(action: &crate::ComputerAction) -> Result<(), ComputerUseError> {
    match action {
        crate::ComputerAction::Click {
            element_id, point, ..
        } => match (element_id, point) {
            (Some(id), None) => validate_id(id, "element"),
            (None, Some(point)) if point.x.is_finite() && point.y.is_finite() => Ok(()),
            _ => Err(ComputerUseError::HelperProtocol(
                "click must provide exactly one element id or finite point".to_string(),
            )),
        },
        crate::ComputerAction::TypeText {
            element_id, text, ..
        } => {
            validate_id(element_id, "element")?;
            if text.chars().count() > MAX_TEXT_CHARS {
                return Err(ComputerUseError::HelperProtocol(
                    "text input is oversized".to_string(),
                ));
            }
            Ok(())
        }
        crate::ComputerAction::Keypress { modifiers, .. } => validate_modifiers(modifiers),
        crate::ComputerAction::Drag {
            start,
            end,
            duration_ms,
            ..
        } => {
            validate_point_location(start)?;
            validate_point_location(end)?;
            if !(50..=2_000).contains(duration_ms) {
                return Err(ComputerUseError::HelperProtocol(
                    "drag duration is outside 50..=2000 ms".to_string(),
                ));
            }
            Ok(())
        }
        _ => Err(ComputerUseError::HumanHandoffRequired(
            "the portable service currently supports bounded click, drag, and keypress actions"
                .to_string(),
        )),
    }
}

fn validate_point_location(location: &crate::ActionLocation) -> Result<(), ComputerUseError> {
    match (&location.element_id, location.point) {
        (None, Some(point)) if point.x.is_finite() && point.y.is_finite() => Ok(()),
        _ => Err(ComputerUseError::HelperProtocol(
            "portable drag locations require one finite screenshot point".to_string(),
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

fn validate_target(target: &crate::WindowTarget) -> Result<(), ComputerUseError> {
    if target.pid <= 0 || target.window_id == 0 {
        return Err(ComputerUseError::HelperProtocol(
            "target PID and window ID must be positive".to_string(),
        ));
    }
    validate_bundle(&target.bundle_id)
}

fn validate_bundle(bundle: &str) -> Result<(), ComputerUseError> {
    if bundle.is_empty() || bundle.chars().count() > MAX_BUNDLE_CHARS {
        return Err(ComputerUseError::HelperProtocol(
            "application identity is empty or oversized".to_string(),
        ));
    }
    crate::ensure_bundle_allowed(bundle)
}

fn validate_id(id: &str, label: &str) -> Result<(), ComputerUseError> {
    if id.is_empty() || id.chars().count() > MAX_ID_CHARS {
        return Err(ComputerUseError::HelperProtocol(format!(
            "{label} id is empty or oversized"
        )));
    }
    Ok(())
}

fn validate_envelope(
    frame: &RequestFrame,
    expected_session: Option<&str>,
    expected_request_id: u64,
) -> Result<(), ComputerUseError> {
    if frame.protocol_version != PROTOCOL_VERSION
        || frame.session_id.is_empty()
        || frame.session_id.chars().count() > MAX_ID_CHARS
        || expected_session.is_some_and(|session| frame.session_id != session)
        || frame.request_id != expected_request_id
    {
        return Err(ComputerUseError::HelperProtocol(
            "request envelope version, session, or monotonic identity is invalid".to_string(),
        ));
    }
    Ok(())
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
            "control envelope version, session, or monotonic identity is invalid".to_string(),
        ));
    }
    Ok(())
}

fn disconnected(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe
    )
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_requires_exact_version_session_and_monotonic_id() {
        let valid = RequestFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: "session".to_string(),
            request_id: 3,
            body: Request::Permissions,
        };
        assert!(validate_envelope(&valid, Some("session"), 3).is_ok());
        assert!(validate_envelope(&valid, Some("different"), 3).is_err());
        assert!(validate_envelope(&valid, Some("session"), 4).is_err());

        let wrong_version = RequestFrame {
            protocol_version: PROTOCOL_VERSION + 1,
            ..valid
        };
        assert!(validate_envelope(&wrong_version, Some("session"), 3).is_err());
    }

    #[test]
    fn request_validation_rejects_empty_permissions_and_unbounded_drag() {
        let empty = Request::PromptPermissions(crate::PermissionRequest {
            accessibility: false,
            screen_recording: false,
        });
        assert!(validate_request(&empty).is_err());

        let drag = Request::PrepareAction(crate::PrepareActionRequest {
            intent: crate::ActionIntent {
                risk: crate::ActionRisk::Ambiguous,
                reason: "test bounded drag".to_string(),
            },
            window: crate::WindowTarget {
                pid: 42,
                window_id: 7,
                bundle_id: "qa.fixture".to_string(),
            },
            observation_id: "observation".to_string(),
            action: crate::ComputerAction::Drag {
                start: crate::ActionLocation {
                    element_id: None,
                    point: Some(crate::Point { x: 1.0, y: 1.0 }),
                },
                end: crate::ActionLocation {
                    element_id: None,
                    point: Some(crate::Point { x: 2.0, y: 2.0 }),
                },
                button: crate::MouseButton::Left,
                duration_ms: 2_001,
            },
            dry_run: false,
        });
        assert!(validate_request(&drag).is_err());
    }
}
