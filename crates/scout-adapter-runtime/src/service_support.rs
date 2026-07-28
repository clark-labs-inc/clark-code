use scout_adapter_protocol::{
    AdapterPageOutcome, AdapterPageReceipt, AdapterPageRequest, FailureReason, RedactionSummary,
    TruncationReason,
};
use sha2::{Digest, Sha256};

use crate::error::{RuntimeError, RuntimeResult};
use crate::types::{AuthCandidate, AuthCandidateHandle, AuthCandidateSource};
use crate::vault::StoredAuthRef;
use crate::{aws, gcp, github, gitlab};

pub(crate) fn candidate(handle: AuthCandidateHandle, reference: StoredAuthRef) -> AuthCandidate {
    let (adapter_id, provider, source) = match reference {
        StoredAuthRef::GithubEnvironment { .. } => (
            github::adapter_id(),
            "github",
            AuthCandidateSource::TargetEnvironment,
        ),
        StoredAuthRef::GithubCli => (
            github::adapter_id(),
            "github",
            AuthCandidateSource::TargetCli,
        ),
        StoredAuthRef::GitlabEnvironment { .. } => (
            gitlab::adapter_id(),
            "gitlab",
            AuthCandidateSource::TargetEnvironment,
        ),
        StoredAuthRef::AwsEnvironment => (
            aws::adapter_id(),
            "aws",
            AuthCandidateSource::TargetEnvironment,
        ),
        StoredAuthRef::AwsProfile { .. } => {
            (aws::adapter_id(), "aws", AuthCandidateSource::TargetProfile)
        }
        StoredAuthRef::AwsWorkload => (
            aws::adapter_id(),
            "aws",
            AuthCandidateSource::TargetWorkload,
        ),
        StoredAuthRef::GcpCli { .. } => (gcp::adapter_id(), "gcp", AuthCandidateSource::TargetCli),
    };
    AuthCandidate {
        handle,
        adapter_id,
        provider: provider.to_owned(),
        source,
    }
}

pub(crate) fn failure_receipt(
    request: AdapterPageRequest,
    target: scout_adapter_protocol::TargetIdentity,
    auth: scout_adapter_protocol::AuthContextDescriptor,
    error: RuntimeError,
    observed_at_ms: u64,
) -> RuntimeResult<AdapterPageReceipt> {
    let outcome = match error {
        RuntimeError::AccessDenied => AdapterPageOutcome::Denied {
            reason: FailureReason::AccessDenied,
        },
        RuntimeError::AuthStale => AdapterPageOutcome::Stale {
            reason: FailureReason::AuthenticationExpired,
        },
        RuntimeError::RateLimited => AdapterPageOutcome::Unreachable {
            reason: FailureReason::RateLimited,
        },
        RuntimeError::ProviderUnavailable => AdapterPageOutcome::Unreachable {
            reason: FailureReason::ServiceUnavailable,
        },
        RuntimeError::UnsupportedAdapter => AdapterPageOutcome::Unsupported {
            reason: FailureReason::InvalidScope,
        },
        RuntimeError::ProviderProtocol | RuntimeError::Protocol => AdapterPageOutcome::Unsafe {
            reason: FailureReason::ProtocolViolation,
        },
        RuntimeError::BoundExceeded => AdapterPageOutcome::Truncated {
            reason: TruncationReason::Deadline,
            continuation_available: false,
        },
        RuntimeError::InvalidRequest
        | RuntimeError::TargetMismatch
        | RuntimeError::CandidateNotFound
        | RuntimeError::AuthNotFound
        | RuntimeError::Vault => return Err(error),
    };
    AdapterPageReceipt::new(
        request,
        target,
        auth,
        adapter_build_sha256(),
        observed_at_ms,
        outcome,
        Vec::new(),
        None,
        RedactionSummary::default(),
    )
    .map_err(Into::into)
}

pub(crate) fn adapter_build_sha256() -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!(
                "scout-adapter-runtime@{}:{}:{}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            )
            .as_bytes()
        )
    )
}
