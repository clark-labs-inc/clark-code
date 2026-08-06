use thiserror::Error;

pub(crate) type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum RuntimeError {
    #[error("invalid request")]
    InvalidRequest,
    #[error("target mismatch")]
    TargetMismatch,
    #[error("candidate not found")]
    CandidateNotFound,
    #[error("authorization not found")]
    AuthNotFound,
    #[error("authorization stale")]
    AuthStale,
    #[error("access denied")]
    AccessDenied,
    #[error("rate limited")]
    RateLimited,
    #[error("provider unavailable")]
    ProviderUnavailable,
    #[error("provider protocol failure")]
    ProviderProtocol,
    #[error("unsupported adapter")]
    UnsupportedAdapter,
    #[error("execution bound exceeded")]
    BoundExceeded,
    #[error("private vault unavailable")]
    Vault,
    #[error("adapter protocol validation failed")]
    Protocol,
}

impl From<scout_adapter_protocol::ProtocolError> for RuntimeError {
    fn from(_: scout_adapter_protocol::ProtocolError) -> Self {
        Self::Protocol
    }
}
