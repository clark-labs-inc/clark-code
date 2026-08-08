use std::fmt;

use scout_adapter_protocol::{
    AdapterId, AdapterPageReceipt, AuthContextDescriptor, TargetId, TargetIdentity,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{RuntimeError, RuntimeResult};

pub const RUNTIME_PROTOCOL_VERSION: u16 = 3;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthCandidateHandle(String);

impl AuthCandidateHandle {
    pub(crate) fn for_target_ref(target_id: &TargetId, reference: &str) -> Self {
        let digest = Sha256::digest(format!("{target_id}\0{reference}").as_bytes());
        Self(format!("candidate:{digest:x}"))
    }

    pub(crate) fn validate(&self) -> RuntimeResult<()> {
        let Some(digest) = self.0.strip_prefix("candidate:") else {
            return Err(RuntimeError::InvalidRequest);
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(RuntimeError::InvalidRequest);
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AuthCandidateHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCandidateSource {
    TargetEnvironment,
    TargetCli,
    TargetProfile,
    TargetWorkload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthCandidate {
    pub handle: AuthCandidateHandle,
    pub adapter_id: AdapterId,
    pub provider: String,
    pub source: AuthCandidateSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    NativeGithubHttps,
    NativeGitlabHttps,
    GhCli,
    AwsCli,
    GcloudCli,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCapability {
    pub tool: ToolKind,
    pub available: bool,
    pub census_failure: Option<SafeFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeFailureCode {
    InvalidRequest,
    TargetMismatch,
    CandidateNotFound,
    AuthorizationStale,
    AccessDenied,
    RateLimited,
    ProviderUnavailable,
    ProviderProtocol,
    UnsupportedAdapter,
    BoundExceeded,
    VaultUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeFailure {
    pub code: SafeFailureCode,
    pub retryable: bool,
}

impl From<&RuntimeError> for SafeFailure {
    fn from(error: &RuntimeError) -> Self {
        let (code, retryable) = match error {
            RuntimeError::InvalidRequest | RuntimeError::Protocol => {
                (SafeFailureCode::InvalidRequest, false)
            }
            RuntimeError::TargetMismatch => (SafeFailureCode::TargetMismatch, false),
            RuntimeError::CandidateNotFound | RuntimeError::AuthNotFound => {
                (SafeFailureCode::CandidateNotFound, false)
            }
            RuntimeError::AuthStale => (SafeFailureCode::AuthorizationStale, false),
            RuntimeError::AccessDenied => (SafeFailureCode::AccessDenied, false),
            RuntimeError::RateLimited => (SafeFailureCode::RateLimited, true),
            RuntimeError::ProviderUnavailable => (SafeFailureCode::ProviderUnavailable, true),
            RuntimeError::ProviderProtocol => (SafeFailureCode::ProviderProtocol, false),
            RuntimeError::UnsupportedAdapter => (SafeFailureCode::UnsupportedAdapter, false),
            RuntimeError::BoundExceeded => (SafeFailureCode::BoundExceeded, true),
            RuntimeError::Vault => (SafeFailureCode::VaultUnavailable, true),
        };
        Self { code, retryable }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CensusRequest {
    pub runtime_protocol_version: u16,
}

impl Default for CensusRequest {
    fn default() -> Self {
        Self {
            runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CensusResponse {
    Succeeded {
        target: Box<TargetIdentity>,
        candidates: Vec<AuthCandidate>,
        tools: Vec<ToolCapability>,
        coverage_manifest: crate::AdapterCoverageManifest,
        observed_at_ms: u64,
    },
    Failed {
        failure: SafeFailure,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyAuthRequest {
    pub runtime_protocol_version: u16,
    pub target_id: TargetId,
    pub target_identity_sha256: String,
    pub candidate_handle: AuthCandidateHandle,
    pub adapter_id: AdapterId,
    pub requested_authority_scope: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerifyAuthResponse {
    Succeeded {
        target: Box<TargetIdentity>,
        auth_context: Box<AuthContextDescriptor>,
    },
    Failed {
        failure: SafeFailure,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum FetchPageResponse {
    Succeeded { receipt: Box<AdapterPageReceipt> },
    Failed { failure: SafeFailure },
}

pub(crate) fn random_auth_handle() -> scout_adapter_protocol::AuthContextHandle {
    scout_adapter_protocol::AuthContextHandle::new(format!("auth:{}", Uuid::new_v4()))
        .expect("UUID")
}
