use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{
    ActionAuthorization, ActionReceipt, CancelAck, ClickRequest, KeyPressRequest, Observation,
    PermissionRequest, PermissionStatus, PrepareActionRequest, PreparedAction, TypeTextRequest,
    WindowFilter, WindowInfo, WindowTarget,
};

pub const PROTOCOL_VERSION: u16 = 3;
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
        parent_pid: u32,
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
    Hello { helper_pid: u32 },
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
    Hello { parent_pid: u32 },
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
    pub message: String,
}

pub fn write_request(stream: &mut UnixStream, frame: &RequestFrame) -> io::Result<()> {
    write_frame(stream, frame, MAX_REQUEST_BYTES)
}

#[cfg(any(feature = "helper-service", test))]
pub fn read_request(stream: &mut UnixStream) -> io::Result<RequestFrame> {
    read_frame(stream, MAX_REQUEST_BYTES)
}

#[cfg(any(feature = "helper-service", test))]
pub fn write_response(stream: &mut UnixStream, frame: &ResponseFrame) -> io::Result<()> {
    write_frame(stream, frame, MAX_RESPONSE_BYTES)
}

pub fn read_response(stream: &mut UnixStream) -> io::Result<ResponseFrame> {
    read_frame(stream, MAX_RESPONSE_BYTES)
}

pub fn write_control_request(
    stream: &mut UnixStream,
    frame: &ControlRequestFrame,
) -> io::Result<()> {
    write_frame(stream, frame, MAX_REQUEST_BYTES)
}

#[cfg(any(feature = "helper-service", test))]
pub fn read_control_request(stream: &mut UnixStream) -> io::Result<ControlRequestFrame> {
    read_frame(stream, MAX_REQUEST_BYTES)
}

#[cfg(any(feature = "helper-service", test))]
pub fn write_control_response(
    stream: &mut UnixStream,
    frame: &ControlResponseFrame,
) -> io::Result<()> {
    write_frame(stream, frame, MAX_RESPONSE_BYTES)
}

pub fn read_control_response(stream: &mut UnixStream) -> io::Result<ControlResponseFrame> {
    read_frame(stream, MAX_RESPONSE_BYTES)
}

fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T, maximum: usize) -> io::Result<()> {
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

fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream, maximum: usize) -> io::Result<T> {
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
    use super::*;

    #[test]
    fn frame_round_trip_preserves_session_and_request_identity() {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        let frame = RequestFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: "session-a".to_string(),
            request_id: 17,
            body: Request::Permissions,
        };
        write_request(&mut writer, &frame).unwrap();
        let decoded = read_request(&mut reader).unwrap();
        assert_eq!(decoded.protocol_version, PROTOCOL_VERSION);
        assert_eq!(decoded.session_id, "session-a");
        assert_eq!(decoded.request_id, 17);
        assert!(matches!(decoded.body, Request::Permissions));
    }

    #[test]
    fn oversized_and_zero_length_frames_fail_before_allocation() {
        for length in [0_u32, (MAX_REQUEST_BYTES + 1) as u32] {
            let (mut writer, mut reader) = UnixStream::pair().unwrap();
            writer.write_all(&length.to_be_bytes()).unwrap();
            let error = read_request(&mut reader).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer
            .write_all(&((MAX_RESPONSE_BYTES + 1) as u32).to_be_bytes())
            .unwrap();
        let error = read_response(&mut reader).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn malformed_messagepack_is_rejected() {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer.write_all(&1_u32.to_be_bytes()).unwrap();
        writer.write_all(&[0xc1]).unwrap();
        let error = read_request(&mut reader).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn control_frames_round_trip_independently() {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        let frame = ControlRequestFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: "session-control".to_string(),
            request_id: 1,
            body: ControlRequest::CancelActive,
        };
        write_control_request(&mut writer, &frame).unwrap();
        let decoded = read_control_request(&mut reader).unwrap();
        assert_eq!(decoded.session_id, "session-control");
        assert_eq!(decoded.request_id, 1);
        assert!(matches!(decoded.body, ControlRequest::CancelActive));

        let response = ControlResponseFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: "session-control".to_string(),
            request_id: 1,
            body: Ok(ControlResponse::CancelAck(CancelAck {
                lease_id: Some("lease-1".to_string()),
                quiesced: true,
                helper_terminated: false,
            })),
        };
        write_control_response(&mut reader, &response).unwrap();
        let decoded = read_control_response(&mut writer).unwrap();
        assert!(matches!(
            decoded.body,
            Ok(ControlResponse::CancelAck(CancelAck { quiesced: true, .. }))
        ));
    }
}
