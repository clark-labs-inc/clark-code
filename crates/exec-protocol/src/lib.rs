//! The wire contract between the desktop's `RemoteExecutor` (client) and the
//! `clark-exec-server` (server). JSON-RPC 2.0 over a single WebSocket:
//!
//! - **Requests** carry an `id`; the server answers with a matching response
//!   (`result` or `error`).
//! - **Notifications** have no `id`; the server pushes process output / exit.
//!
//! The connection opens with an [`method::AUTH`] request carrying the
//! per-session capability token and the [`PROTOCOL_VERSION`]; the server rejects
//! a mismatched token (defense-in-depth on the loopback port) or version
//! (prevents desktop ↔ server protocol drift) before serving anything else.
//!
//! Filesystem ops are request/response. A command runs as a small streaming
//! exchange: [`method::PROCESS_START`] → a sequence of
//! [`method::PROCESS_OUTPUT`] notifications → one [`method::PROCESS_EXIT`]. The
//! server buffers output by `seq` so a client that loses the socket mid-build
//! can reconnect, re-auth, and [`method::PROCESS_RESUME`] from the last `seq` it
//! saw — no lost output, no rerun.
//!
//! Both ends share these types, so the two can't drift.

use serde::{Deserialize, Serialize};

/// Bumped on any breaking change to the methods/params below. The server
/// refuses to serve a client advertising a different major value.
pub const PROTOCOL_VERSION: u32 = 2;

/// Method names. String constants (not an enum) so unknown methods round-trip to
/// a clean "method not found" error instead of a deserialize failure.
pub mod method {
    pub const AUTH: &str = "auth";
    pub const FS_READ: &str = "fs/read";
    pub const FS_WRITE: &str = "fs/write";
    pub const FS_CREATE_DIR: &str = "fs/createDir";
    pub const FS_REMOVE_FILE: &str = "fs/removeFile";
    pub const FS_REMOVE_DIR: &str = "fs/removeDir";
    pub const FS_READ_DIR: &str = "fs/readDir";
    pub const FS_METADATA: &str = "fs/metadata";
    pub const FS_WALK: &str = "fs/walk";
    pub const PROCESS_START: &str = "process/start";
    pub const PROCESS_RESUME: &str = "process/resume";
    pub const PROCESS_STATUS: &str = "process/status";
    pub const PROCESS_INPUT: &str = "process/input";
    pub const PROCESS_CANCEL: &str = "process/cancel";
    /// Notification: a chunk of process output.
    pub const PROCESS_OUTPUT: &str = "process/output";
    /// Notification: the process finished (or failed to run).
    pub const PROCESS_EXIT: &str = "process/exit";
}

/// JSON-RPC error codes we emit. Negative per the JSON-RPC convention; the
/// application-defined ones live in the reserved `-32000..` server range.
pub mod error_code {
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    /// Catch-all for an op that failed on the target (I/O error, spawn failure).
    pub const EXEC_FAILED: i64 = -32000;
    /// Auth must be the first request; token or protocol version mismatched.
    pub const UNAUTHORIZED: i64 = -32001;
    /// `process/resume` for a `process_id` the server no longer knows.
    pub const UNKNOWN_PROCESS: i64 = -32002;
}

/// A client→server request (always carries `id`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: JsonRpc,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Request {
    pub fn new(id: u64, method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: JsonRpc,
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// A server→client response to a [`Request`] with the same `id`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: JsonRpc,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: JsonRpc,
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JsonRpc,
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// A server→client notification (no `id`, no reply expected).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: JsonRpc,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Notification {
    pub fn new(method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: JsonRpc,
            method: method.to_string(),
            params,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// The literal `"2.0"` tag every JSON-RPC frame carries.
#[derive(Clone, Copy, Debug)]
pub struct JsonRpc;

impl Serialize for JsonRpc {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for JsonRpc {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = String::deserialize(d)?;
        if v == "2.0" {
            Ok(JsonRpc)
        } else {
            Err(serde::de::Error::custom("expected jsonrpc \"2.0\""))
        }
    }
}

// ---- Method params + results (the shared, typed contract) -------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthParams {
    pub token: String,
    pub protocol_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthResult {
    /// Echoed so the client can log/verify which server it reached.
    pub server_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathParams {
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadResult {
    /// File bytes, base64. Files may be binary, so never raw JSON strings.
    pub data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteParams {
    pub path: String,
    /// Bytes to write, base64.
    pub data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadDirResult {
    pub entries: Vec<WireDirEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireDirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Stat result. `modified_ms` is Unix-epoch millis, or `None` if unavailable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetaResult {
    pub modified_ms: Option<u64>,
    pub len: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalkResult {
    pub entries: Vec<WireWalkEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireWalkEntry {
    /// Absolute path on the target machine.
    pub path: String,
    pub modified_ms: Option<u64>,
    pub len: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessStartParams {
    /// Client-generated id; lets the client resume this exact run after a drop.
    pub process_id: String,
    pub command: String,
    pub cwd: String,
    pub timeout_ms: u64,
    #[serde(default)]
    pub pty: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessResumeParams {
    pub process_id: String,
    /// Replay buffered output with `seq` strictly greater than this.
    pub after_seq: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessIdParams {
    pub process_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessStatusParams {
    pub process_id: String,
    pub after_seq: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessStatusResult {
    pub output: Vec<ProcessOutputParams>,
    pub exit: Option<ProcessExitParams>,
    pub truncated_before_seq: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessInputParams {
    pub process_id: String,
    pub data: String,
    #[serde(default)]
    pub close: bool,
}

/// Which stream a [`method::PROCESS_OUTPUT`] chunk came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessOutputParams {
    pub process_id: String,
    /// Monotonic per-process sequence number; the resume cursor.
    pub seq: u64,
    pub stream: Stream,
    /// Chunk bytes, base64.
    pub data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessExitParams {
    pub process_id: String,
    /// One past the last output `seq`; the terminal marker for resume.
    pub seq: u64,
    /// Exit code, or `None` if signalled — mutually exclusive with `error`.
    pub code: Option<i32>,
    /// Set instead of `code` when the command never produced an exit status
    /// (spawn failure / timeout / cancellation): a model-readable message.
    pub error: Option<String>,
}

// ---- base64 helpers (one definition, so both ends agree) --------------------

/// Encode bytes for a wire `data` field.
pub fn b64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode a wire `data` field back to bytes.
pub fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("invalid base64: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrips_with_jsonrpc_tag() {
        let r = Request::new(7, method::FS_READ, serde_json::json!({"path": "/a"}));
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"jsonrpc\":\"2.0\""));
        assert!(s.contains("\"id\":7"));
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(back.method, method::FS_READ);
    }

    #[test]
    fn response_omits_absent_arm() {
        let ok = serde_json::to_string(&Response::ok(1, serde_json::json!({}))).unwrap();
        assert!(!ok.contains("error"));
        let err =
            serde_json::to_string(&Response::err(1, error_code::EXEC_FAILED, "boom")).unwrap();
        assert!(!err.contains("result"));
        assert!(err.contains("boom"));
    }

    #[test]
    fn rejects_wrong_jsonrpc_version() {
        let bad = r#"{"jsonrpc":"1.0","id":1,"method":"auth","params":{}}"#;
        assert!(serde_json::from_str::<Request>(bad).is_err());
    }

    #[test]
    fn base64_roundtrips_binary() {
        let bytes = vec![0u8, 159, 146, 150, 255];
        assert_eq!(b64_decode(&b64_encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn stream_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&Stream::Stdout).unwrap(),
            "\"stdout\""
        );
    }

    #[test]
    fn process_start_without_pty_field_defaults_to_pipes() {
        let params: ProcessStartParams = serde_json::from_value(serde_json::json!({
            "process_id": "p1",
            "command": "echo hi",
            "cwd": "/tmp",
            "timeout_ms": 1000
        }))
        .unwrap();
        assert!(!params.pty);
    }
}
