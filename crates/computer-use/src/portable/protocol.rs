use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{
    ActionAuthorization, ActionReceipt, CancelAck, ClickRequest, ComputerUseError, KeyPressRequest,
    Observation, PermissionRequest, PermissionStatus, PrepareActionRequest, PreparedAction,
    TypeTextRequest, WindowFilter, WindowInfo, WindowTarget,
};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_REQUEST_BYTES: usize = 1_048_576;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1_048_576;

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestFrame {
    pub protocol_version: u16,
    pub session_id: String,
    pub request_id: u64,
    pub body: Request,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    Hello {
        client_pid: u32,
    },
    Permissions,
    PromptPermissions(PermissionRequest),
    ListWindows(WindowFilter),
    LaunchApplication {
        bundle_id: String,
    },
    Observe(WindowTarget),
    PrepareAction(PrepareActionRequest),
    PreparedAction {
        id: String,
    },
    AuthorizeAction {
        id: String,
        authorization: ActionAuthorization,
    },
    CommitAction {
        id: String,
    },
    Click(ClickRequest),
    TypeText(TypeTextRequest),
    KeyPress(KeyPressRequest),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseFrame {
    pub protocol_version: u16,
    pub session_id: String,
    pub request_id: u64,
    pub body: Result<Response, RemoteError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Hello { service_pid: u32 },
    Permissions(PermissionStatus),
    Windows(Vec<WindowInfo>),
    Observation(Observation),
    PreparedAction(PreparedAction),
    ActionReceipt(ActionReceipt),
    Unit,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ControlRequestFrame {
    pub protocol_version: u16,
    pub session_id: String,
    pub request_id: u64,
    pub body: ControlRequest,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ControlRequest {
    Hello { client_pid: u32 },
    CancelActive,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ControlResponseFrame {
    pub protocol_version: u16,
    pub session_id: String,
    pub request_id: u64,
    pub body: Result<ControlResponse, RemoteError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ControlResponse {
    Hello,
    CancelAck(CancelAck),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteError {
    pub kind: RemoteErrorKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteErrorKind {
    UserTakeover,
    InputCancelled,
    TakeoverMonitorUnavailable,
    ObservationRequired,
    ObservationStale,
    ApprovalRequired,
    PreparedActionExpired,
    RateLimited,
    Other,
}

impl From<ComputerUseError> for RemoteError {
    fn from(error: ComputerUseError) -> Self {
        let message = error.to_string();
        let kind = match error {
            ComputerUseError::UserTakeover => RemoteErrorKind::UserTakeover,
            ComputerUseError::InputCancelled => RemoteErrorKind::InputCancelled,
            ComputerUseError::TakeoverMonitorUnavailable => {
                RemoteErrorKind::TakeoverMonitorUnavailable
            }
            ComputerUseError::ObservationRequired => RemoteErrorKind::ObservationRequired,
            ComputerUseError::ObservationStale => RemoteErrorKind::ObservationStale,
            ComputerUseError::ApprovalRequired => RemoteErrorKind::ApprovalRequired,
            ComputerUseError::PreparedActionExpired => RemoteErrorKind::PreparedActionExpired,
            ComputerUseError::RateLimited => RemoteErrorKind::RateLimited,
            _ => RemoteErrorKind::Other,
        };
        Self { kind, message }
    }
}

impl RemoteError {
    pub fn into_local(self) -> ComputerUseError {
        match self.kind {
            RemoteErrorKind::UserTakeover => ComputerUseError::UserTakeover,
            RemoteErrorKind::InputCancelled => ComputerUseError::InputCancelled,
            RemoteErrorKind::TakeoverMonitorUnavailable => {
                ComputerUseError::TakeoverMonitorUnavailable
            }
            RemoteErrorKind::ObservationRequired => ComputerUseError::ObservationRequired,
            RemoteErrorKind::ObservationStale => ComputerUseError::ObservationStale,
            RemoteErrorKind::ApprovalRequired => ComputerUseError::ApprovalRequired,
            RemoteErrorKind::PreparedActionExpired => ComputerUseError::PreparedActionExpired,
            RemoteErrorKind::RateLimited => ComputerUseError::RateLimited,
            RemoteErrorKind::Other => ComputerUseError::HelperRejected(self.message),
        }
    }
}

pub fn write_request<W: Write>(stream: &mut W, frame: &RequestFrame) -> io::Result<()> {
    write_frame(stream, frame, MAX_REQUEST_BYTES)
}

#[cfg(any(feature = "helper-service", test))]
pub fn read_request<R: Read>(stream: &mut R) -> io::Result<RequestFrame> {
    read_frame(stream, MAX_REQUEST_BYTES)
}

#[cfg(any(feature = "helper-service", test))]
pub fn write_response<W: Write>(stream: &mut W, frame: &ResponseFrame) -> io::Result<()> {
    write_frame(stream, frame, MAX_RESPONSE_BYTES)
}

pub fn read_response<R: Read>(stream: &mut R) -> io::Result<ResponseFrame> {
    read_frame(stream, MAX_RESPONSE_BYTES)
}

pub fn write_control_request<W: Write>(
    stream: &mut W,
    frame: &ControlRequestFrame,
) -> io::Result<()> {
    write_frame(stream, frame, MAX_REQUEST_BYTES)
}

#[cfg(any(feature = "helper-service", test))]
pub fn read_control_request<R: Read>(stream: &mut R) -> io::Result<ControlRequestFrame> {
    read_frame(stream, MAX_REQUEST_BYTES)
}

#[cfg(any(feature = "helper-service", test))]
pub fn write_control_response<W: Write>(
    stream: &mut W,
    frame: &ControlResponseFrame,
) -> io::Result<()> {
    write_frame(stream, frame, MAX_RESPONSE_BYTES)
}

pub fn read_control_response<R: Read>(stream: &mut R) -> io::Result<ControlResponseFrame> {
    read_frame(stream, MAX_RESPONSE_BYTES)
}

fn write_frame<T: Serialize, W: Write>(
    stream: &mut W,
    value: &T,
    maximum: usize,
) -> io::Result<()> {
    let payload = rmp_serde::to_vec_named(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if payload.is_empty() || payload.len() > maximum || payload.len() > u32::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("IPC frame size {} exceeds limit {maximum}", payload.len()),
        ));
    }
    stream.write_all(&(payload.len() as u32).to_be_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()
}

fn read_frame<T: DeserializeOwned, R: Read>(stream: &mut R, maximum: usize) -> io::Result<T> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("IPC frame size {length} is outside 1..={maximum}"),
        ));
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload)?;
    rmp_serde::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn request_frame_round_trips_with_bounded_framing() {
        let frame = RequestFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: "session-1".to_string(),
            request_id: 7,
            body: Request::Permissions,
        };
        let mut bytes = Vec::new();
        write_request(&mut bytes, &frame).unwrap();

        let decoded = read_request(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(decoded.protocol_version, PROTOCOL_VERSION);
        assert_eq!(decoded.session_id, "session-1");
        assert_eq!(decoded.request_id, 7);
        assert!(matches!(decoded.body, Request::Permissions));
    }

    #[test]
    fn frame_reader_rejects_zero_and_oversized_lengths_before_allocating() {
        let zero = read_request(&mut Cursor::new(0_u32.to_be_bytes())).unwrap_err();
        assert_eq!(zero.kind(), io::ErrorKind::InvalidData);

        let oversized = ((MAX_REQUEST_BYTES + 1) as u32).to_be_bytes();
        let oversized = read_request(&mut Cursor::new(oversized)).unwrap_err();
        assert_eq!(oversized.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn operational_remote_errors_keep_typed_semantics() {
        let takeover = RemoteError::from(ComputerUseError::UserTakeover).into_local();
        assert!(matches!(takeover, ComputerUseError::UserTakeover));

        let stale = RemoteError::from(ComputerUseError::ObservationStale).into_local();
        assert!(matches!(stale, ComputerUseError::ObservationStale));

        let other = RemoteError::from(ComputerUseError::Os("boom".to_string())).into_local();
        assert!(matches!(other, ComputerUseError::HelperRejected(message) if message == "boom"));
    }
}
