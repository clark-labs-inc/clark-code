//! Typed filesystem and bounded-process payloads shared by the durable worker's
//! project plugin and Agent Desktop's native executor adapter. Correlation,
//! cancellation, ordering, and authentication belong to `code-host` and
//! `code-remote`; this crate contains no second transport protocol.

use serde::{Deserialize, Serialize};

/// Project-plugin method names. String constants keep dispatch compact while
/// the enclosing code-host request remains strictly typed and versioned.
pub mod method {
    pub const FS_READ: &str = "fs/read";
    pub const FS_WRITE: &str = "fs/write";
    pub const FS_WRITE_PRIVATE: &str = "fs/writePrivate";
    pub const FS_WRITE_PRIVATE_NEW: &str = "fs/writePrivateNew";
    pub const FS_SYNC_FILE: &str = "fs/syncFile";
    pub const FS_SYNC_DIRECTORY: &str = "fs/syncDirectory";
    pub const FS_CREATE_DIR: &str = "fs/createDir";
    pub const FS_REMOVE_FILE: &str = "fs/removeFile";
    pub const FS_REMOVE_DIR: &str = "fs/removeDir";
    pub const FS_RENAME: &str = "fs/rename";
    pub const FS_READ_DIR: &str = "fs/readDir";
    pub const FS_METADATA: &str = "fs/metadata";
    pub const FS_CANONICALIZE: &str = "fs/canonicalize";
    pub const FS_WALK: &str = "fs/walk";
    pub const ENV_HOME: &str = "environment/home";
    pub const ENV_CAPABILITY_CENSUS: &str = "environment/capabilityCensus";
    pub const PROCESS_START: &str = "process/start";
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
pub struct WriteNewResult {
    pub created: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenameParams {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadDirResult {
    pub entries: Vec<WireDirEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireDirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
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
pub struct CanonicalizeResult {
    /// Absolute path on the target machine after resolving symlinks.
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemCapabilityCensusResult {
    pub platform: String,
    pub architecture: String,
    pub executable_names: Vec<String>,
    pub environment_variable_names: Vec<String>,
    pub credential_surfaces: Vec<String>,
    pub executables_truncated: bool,
    pub environment_names_truncated: bool,
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
    /// Client-generated id used in cancellation and diagnostics.
    pub process_id: String,
    pub command: String,
    pub cwd: String,
    pub timeout_ms: u64,
    #[serde(default)]
    pub pty: bool,
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
    fn base64_roundtrips_binary() {
        let bytes = vec![0u8, 159, 146, 150, 255];
        assert_eq!(b64_decode(&b64_encode(&bytes)).unwrap(), bytes);
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
