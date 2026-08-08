//! Privilege-boundary protocol for Agent Desktop's Windows sandbox helpers.
//!
//! This crate deliberately contains no process spawning, ACL mutation, network
//! configuration, provider logic, or UI code. The unprivileged desktop, the
//! elevated setup helper, and the restricted command runner all share these
//! wire types and validation rules.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RUNNER_PROTOCOL_VERSION: u32 = 1;
pub const SETUP_PROTOCOL_VERSION: u32 = 3;
pub const SETUP_MARKER_FILE: &str = "setup-marker-v1.json";
/// Keep the base64 request plus executable path and switch comfortably below
/// Windows' 32,767 UTF-16-unit process command-line limit.
pub const MAX_ENCODED_REQUEST_CHARS: usize = 24 * 1024;
pub const MAX_REQUEST_BYTES: usize = MAX_ENCODED_REQUEST_CHARS / 4 * 3;

pub const EXIT_SETUP_REQUIRED: i32 = 121;
pub const EXIT_INVALID_REQUEST: i32 = 122;
pub const EXIT_CONTAINMENT_FAILED: i32 = 123;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireNetworkPolicy {
    Restricted,
    Enabled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireSandboxPolicy {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub deny_read: Vec<PathBuf>,
    pub deny_write: Vec<PathBuf>,
    pub network: WireNetworkPolicy,
    pub process_temp_root: Option<PathBuf>,
}

impl WireSandboxPolicy {
    pub fn validate(&self) -> Result<(), String> {
        for path in self
            .read_roots
            .iter()
            .chain(&self.write_roots)
            .chain(&self.deny_read)
            .chain(&self.deny_write)
            .chain(self.process_temp_root.iter())
        {
            if !is_absolute_boundary_path(path) {
                return Err(format!(
                    "sandbox policy path must be absolute: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String, String> {
        let mut normalized = self.clone();
        normalize_roots(&mut normalized.read_roots);
        normalize_roots(&mut normalized.write_roots);
        normalize_roots(&mut normalized.deny_read);
        normalize_roots(&mut normalized.deny_write);
        // The temp path is also present in write_roots. Excluding this duplicate
        // environment hint keeps attestation stable for per-session children
        // under an already-provisioned writable parent.
        normalized.process_temp_root = None;
        let encoded = serde_json::to_vec(&normalized)
            .map_err(|error| format!("serialize sandbox policy fingerprint: {error}"))?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }

    /// The restricted-token backend intentionally implements narrow
    /// host-wide reads. Windows WRITE_RESTRICTED tokens cannot make an
    /// arbitrary deny-read list authoritative, so reject such policies rather
    /// than reporting a weaker boundary as enforced.
    pub fn validate_windows_enforceable(&self) -> Result<(), String> {
        if !self.read_roots.is_empty() || !self.deny_read.is_empty() {
            return Err(
                "Windows sandbox supports host-wide reads only; narrowed or denied reads are not enforceable"
                    .to_string(),
            );
        }
        if self.network != WireNetworkPolicy::Restricted {
            return Err("Windows sandbox requires restricted child networking".to_string());
        }
        Ok(())
    }

    /// Restricting-token SIDs for exactly the active writable roots. ACLs from
    /// older sessions remain harmless because their root capability is absent
    /// from the new token even though the offline account itself is reused.
    pub fn write_capability_sids(&self) -> Vec<String> {
        let mut roots = self.write_roots.clone();
        normalize_roots(&mut roots);
        let mut capabilities = vec![Self::device_capability_sid()];
        capabilities.extend(roots.iter().map(|root| {
            capability_sid_for_key(&format!(
                "agent-sandbox-write-root-v1:{}",
                normalized_boundary_text(root)
            ))
        }));
        capabilities
    }

    /// Stable capability installed once on non-filesystem objects needed by
    /// every restricted process (currently Windows' NUL device). It is never
    /// granted on a workspace, so it cannot combine filesystem authority
    /// across independently enrolled projects.
    pub fn device_capability_sid() -> String {
        capability_sid_for_key("agent-sandbox-device-v1")
    }

    pub fn write_capability_sid_for_root(root: &Path) -> String {
        capability_sid_for_key(&format!(
            "agent-sandbox-write-root-v1:{}",
            normalized_boundary_text(root)
        ))
    }
}

fn capability_sid_for_key(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let authorities = digest[..16]
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("four-byte SID authority")))
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("-");
    format!("S-1-5-21-{authorities}")
}

fn normalize_roots(roots: &mut Vec<PathBuf>) {
    roots.sort_by_key(|path| normalized_boundary_text(path));
    roots.dedup_by(|left, right| normalized_boundary_text(left) == normalized_boundary_text(right));
    let original = roots.clone();
    roots.retain(|candidate| {
        !original
            .iter()
            .any(|parent| candidate != parent && boundary_contains(parent, candidate))
    });
}

fn boundary_contains(parent: &Path, child: &Path) -> bool {
    let parent = normalized_boundary_text(parent);
    let child = normalized_boundary_text(child);
    child == parent
        || child
            .strip_prefix(&parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn normalized_boundary_text(path: &Path) -> String {
    resolve_existing_ancestor(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn resolve_existing_ancestor(path: &Path) -> PathBuf {
    if let Ok(resolved) = path.canonicalize() {
        return resolved;
    }
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            return path.to_path_buf();
        };
        suffix.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            return path.to_path_buf();
        };
        ancestor = parent;
    }
    let Ok(mut resolved) = ancestor.canonicalize() else {
        return path.to_path_buf();
    };
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    resolved
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireOsString {
    /// Native Windows UTF-16 code units. This preserves values that JSON text
    /// cannot represent without forcing a lossy UTF-8 conversion.
    #[serde(with = "utf16_bytes")]
    pub utf16: Vec<u16>,
}

impl WireOsString {
    pub fn from_os(value: &OsStr) -> Self {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Self {
                utf16: value.encode_wide().collect(),
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                utf16: value.to_string_lossy().encode_utf16().collect(),
            }
        }
    }

    pub fn to_os_string(&self) -> OsString {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&self.utf16)
        }
        #[cfg(not(windows))]
        {
            OsString::from(String::from_utf16_lossy(&self.utf16))
        }
    }

    fn contains_nul(&self) -> bool {
        self.utf16.contains(&0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireProcess {
    pub program: WireOsString,
    pub args: Vec<WireOsString>,
    pub cwd: WireOsString,
    pub env: Vec<(WireOsString, WireOsString)>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WindowsRunnerRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub state_dir: PathBuf,
    pub policy: WireSandboxPolicy,
    pub process: WireProcess,
}

impl WindowsRunnerRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.protocol_version != RUNNER_PROTOCOL_VERSION {
            return Err(format!(
                "unsupported runner protocol {}; expected {}",
                self.protocol_version, RUNNER_PROTOCOL_VERSION
            ));
        }
        if self.request_id.trim().is_empty() {
            return Err("runner request id is empty".to_string());
        }
        if !is_absolute_boundary_path(&self.state_dir) {
            return Err("runner state directory must be absolute".to_string());
        }
        if self.process.program.utf16.is_empty() {
            return Err("runner command is empty".to_string());
        }
        if self.process.program.contains_nul()
            || self.process.args.iter().any(WireOsString::contains_nul)
        {
            return Err("runner command contains an embedded NUL".to_string());
        }
        if self.process.cwd.utf16.is_empty() {
            return Err("runner working directory is empty".to_string());
        }
        if self.process.cwd.contains_nul() {
            return Err("runner working directory contains an embedded NUL".to_string());
        }
        if !is_absolute_wire_path(&self.process.cwd) {
            return Err("runner working directory must be absolute".to_string());
        }
        for (name, value) in &self.process.env {
            if name.utf16.is_empty()
                || name.contains_nul()
                || value.contains_nul()
                || name.utf16.contains(&(b'=' as u16))
            {
                return Err("runner environment contains an invalid name or value".to_string());
            }
        }
        self.policy.validate()?;
        self.policy.validate_windows_enforceable()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WindowsRootProof {
    pub root: PathBuf,
    pub proof_path: PathBuf,
    pub nonce: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WindowsSetupRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub state_dir: PathBuf,
    pub runner_path: PathBuf,
    pub policy: WireSandboxPolicy,
    /// Files created by the unelevated caller inside every ACL grant root.
    /// The elevated helper consumes these before it changes any ACL, proving
    /// that elevation cannot broaden the caller's own filesystem authority.
    pub root_proofs: Vec<WindowsRootProof>,
}

impl WindowsSetupRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.protocol_version != SETUP_PROTOCOL_VERSION {
            return Err(format!(
                "unsupported setup protocol {}; expected {}",
                self.protocol_version, SETUP_PROTOCOL_VERSION
            ));
        }
        if self.request_id.trim().is_empty() {
            return Err("setup request id is empty".to_string());
        }
        if !is_absolute_boundary_path(&self.state_dir)
            || !is_absolute_boundary_path(&self.runner_path)
        {
            return Err("setup paths must be absolute".to_string());
        }
        self.policy.validate()?;
        self.policy.validate_windows_enforceable()?;
        if self.root_proofs.len() != self.policy.write_roots.len() {
            return Err("every Windows sandbox write root needs one ownership proof".to_string());
        }
        for root in &self.policy.write_roots {
            let matching = self
                .root_proofs
                .iter()
                .filter(|proof| &proof.root == root)
                .count();
            if matching != 1 {
                return Err(format!(
                    "Windows sandbox write root needs exactly one ownership proof: {}",
                    root.display()
                ));
            }
        }
        for proof in &self.root_proofs {
            if !is_absolute_boundary_path(&proof.root)
                || !is_absolute_boundary_path(&proof.proof_path)
            {
                return Err("Windows sandbox ownership proof paths must be absolute".to_string());
            }
            if proof.nonce.len() < 32 {
                return Err("Windows sandbox ownership proof nonce is too short".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsNetworkEnforcement {
    WindowsFilteringPlatform,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WindowsSetupMarker {
    pub setup_protocol_version: u32,
    pub runner_protocol_version: u32,
    pub runner_sha256: String,
    pub offline_identity_sid: String,
    pub network_enforcement: WindowsNetworkEnforcement,
    pub generation: u64,
    pub provisioned_policy_sha256: Vec<String>,
    /// Root-scoped write capabilities whose ACLs were installed by the
    /// elevated helper. This permits a read-only session to select a safe
    /// subset of an already consented workspace policy without another UAC
    /// prompt; capabilities for unconsented roots remain unavailable.
    pub provisioned_write_capability_sids: Vec<String>,
}

impl WindowsSetupMarker {
    pub fn validate_for_runner(&self, runner_path: &Path) -> Result<(), String> {
        if self.setup_protocol_version != SETUP_PROTOCOL_VERSION {
            return Err("Windows sandbox setup marker is out of date".to_string());
        }
        if self.runner_protocol_version != RUNNER_PROTOCOL_VERSION {
            return Err("Windows sandbox runner protocol is out of date".to_string());
        }
        if self.offline_identity_sid.trim().is_empty() {
            return Err("Windows sandbox identity is missing".to_string());
        }
        if self.generation == 0 {
            return Err("Windows sandbox setup generation is invalid".to_string());
        }
        let actual = sha256_file(runner_path)?;
        if !actual.eq_ignore_ascii_case(&self.runner_sha256) {
            return Err("Windows sandbox runner does not match its setup attestation".to_string());
        }
        Ok(())
    }

    pub fn validate_bootstrap(&self, runner_path: &Path) -> Result<(), String> {
        self.validate_for_runner(runner_path)?;
        let device = WireSandboxPolicy::device_capability_sid();
        if !self
            .provisioned_write_capability_sids
            .iter()
            .any(|provisioned| provisioned.eq_ignore_ascii_case(&device))
        {
            return Err("Windows sandbox device capability is not provisioned".to_string());
        }
        Ok(())
    }

    pub fn validate_for_policy(&self, policy: &WireSandboxPolicy) -> Result<(), String> {
        let fingerprint = policy.fingerprint()?;
        if self
            .provisioned_policy_sha256
            .iter()
            .any(|provisioned| provisioned.eq_ignore_ascii_case(&fingerprint))
        {
            return Ok(());
        }

        // Exact attestation remains mandatory for policies with deny rules or
        // narrowed reads because those constraints need their own native ACL
        // reconciliation. A default read-only session has neither: it merely
        // omits the project write capability while retaining already-consented
        // Agent Desktop document/temp roots.
        let subset_eligible = policy.network == WireNetworkPolicy::Restricted
            && policy.read_roots.is_empty()
            && policy.deny_read.is_empty()
            && policy.deny_write.is_empty();
        let capabilities_are_provisioned = policy.write_capability_sids().iter().all(|required| {
            self.provisioned_write_capability_sids
                .iter()
                .any(|provisioned| provisioned.eq_ignore_ascii_case(required))
        });
        if subset_eligible && capabilities_are_provisioned {
            Ok(())
        } else {
            Err("Windows sandbox ACLs are not provisioned for this policy".to_string())
        }
    }
}

pub fn setup_marker_path(state_dir: &Path) -> PathBuf {
    state_dir.join(SETUP_MARKER_FILE)
}

pub fn read_setup_marker(state_dir: &Path) -> Result<WindowsSetupMarker, String> {
    let path = setup_marker_path(state_dir);
    let bytes = std::fs::read(&path).map_err(|error| {
        format!(
            "read Windows sandbox setup marker {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "parse Windows sandbox setup marker {}: {error}",
            path.display()
        )
    })
}

pub fn encode_request<T: Serialize>(request: &T) -> Result<String, String> {
    let bytes = rmp_serde::to_vec_named(request)
        .map_err(|error| format!("serialize sandbox helper request: {error}"))?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(format!(
            "sandbox helper request is too large: {} bytes",
            bytes.len()
        ));
    }
    let encoded = URL_SAFE_NO_PAD.encode(bytes);
    if encoded.len() > MAX_ENCODED_REQUEST_CHARS {
        return Err(format!(
            "encoded sandbox helper request is too large: {} characters",
            encoded.len()
        ));
    }
    Ok(encoded)
}

pub fn decode_request<T: for<'de> Deserialize<'de>>(encoded: &str) -> Result<T, String> {
    if encoded.len() > MAX_ENCODED_REQUEST_CHARS {
        return Err(format!(
            "encoded sandbox helper request is too large: {} bytes",
            encoded.len()
        ));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|error| format!("decode sandbox helper request: {error}"))?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(format!(
            "sandbox helper request is too large: {} bytes",
            bytes.len()
        ));
    }
    rmp_serde::from_slice(&bytes).map_err(|error| format!("parse sandbox helper request: {error}"))
}

mod utf16_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &[u16], serializer: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::with_capacity(value.len() * 2);
        for unit in value {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        serializer.serialize_bytes(&bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u16>, D::Error> {
        let bytes = serde_bytes::ByteBuf::deserialize(deserializer)?;
        if bytes.len() % 2 != 0 {
            return Err(serde::de::Error::custom(
                "UTF-16 byte string has odd length",
            ));
        }
        Ok(bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect())
    }
}

fn is_absolute_wire_path(path: &WireOsString) -> bool {
    let value = String::from_utf16_lossy(&path.utf16);
    is_absolute_text_path(&value)
}

fn is_absolute_boundary_path(path: &Path) -> bool {
    path.is_absolute() || is_absolute_text_path(&path.to_string_lossy())
}

fn is_absolute_text_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with("\\\\")
        || value.starts_with('/')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("open sandbox runner {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("read sandbox runner {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
