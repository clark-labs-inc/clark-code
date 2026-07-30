use serde::{Deserialize, Serialize};

/// Target-service name the exec-server routes to this runner.
pub const SERVICE_NAME: &str = "security-poc-v1";

/// The PoC control under test.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PocControl {
    Positive,
    Negative,
}

/// Interpreter for the bounded PoC control script.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PocLanguage {
    Shell,
    Python,
    Javascript,
}

impl PocLanguage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Python => "python",
            Self::Javascript => "javascript",
        }
    }

    pub fn script_name(self) -> &'static str {
        match self {
            Self::Shell => {
                if cfg!(windows) {
                    "control.ps1"
                } else {
                    "control.sh"
                }
            }
            Self::Python => "control.py",
            Self::Javascript => "control.mjs",
        }
    }
}

/// One inventory file to stage into the disposable workspace. `path` is
/// workspace-relative; `bytes` is the file content.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PocInventoryFile {
    pub path: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

/// A request to run one positive/negative PoC control in a fresh disposable
/// workspace on this target.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SecurityPocRunRequest {
    pub scan_id: String,
    pub candidate_id: String,
    pub inventory_id: String,
    pub control: PocControl,
    pub language: PocLanguage,
    pub expected_observation: String,
    pub script: String,
    pub expected_exit_code: i32,
    pub timeout_seconds: u64,
    /// Repository-relative root for the run receipt (`artifact_path`), e.g.
    /// `.clark/security-scans/<scan>/poc/runs/<...>`. Joined under the service
    /// root; must be relative and escape-free.
    pub run_root: String,
    /// The inventory snapshot to stage. The runner recomputes `workspace_sha256`
    /// over these bytes so the seal reflects what actually ran.
    pub inventory: Vec<PocInventoryFile>,
}

/// Execution metadata recorded on the receipt.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PocExecutionMetadata {
    pub expected_observation: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub timeout_ms: u64,
    pub output_limit_bytes: u64,
    pub sandbox_provider: String,
    pub sandbox_profile_sha256: String,
    pub script_path: String,
    pub stdout_path: String,
    pub stderr_path: String,
}

/// The sealed proof-of-concept receipt. Serialized field-for-field identical to
/// the desktop's `SecurityPocReceipt` so the scan contract's
/// `managed_disposable` acceptance check holds regardless of which host ran it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityPocReceipt {
    pub contract_version: u32,
    pub receipt_id: String,
    pub scan_id: String,
    pub candidate_id: String,
    pub inventory_id: String,
    pub control: PocControl,
    pub language: String,
    pub script_sha256: String,
    pub expected_observation_sha256: String,
    pub workspace_sha256: String,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub expected_exit_code: i32,
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub containment: String,
    pub artifact_path: String,
    #[serde(default)]
    pub execution: Option<PocExecutionMetadata>,
}

/// What the runner returns to the caller. The receipt plus the raw captured
/// output (so the caller can persist `stdout.log` / `stderr.log` artifacts and
/// show previews) — the runner itself only writes the disposable workspace and
/// `receipt.json` on the target.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SecurityPocRunResponse {
    pub receipt: SecurityPocReceipt,
    #[serde(with = "serde_bytes")]
    pub stdout: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub stderr: Vec<u8>,
}

/// Bounded, validated id fragment (matches the desktop's `validate_id`).
pub fn validate_id(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Err(format!(
            "{name} must contain only letters, numbers, `.`, `_`, or `-`"
        ))
    } else {
        Ok(())
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
