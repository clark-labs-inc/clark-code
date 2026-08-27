//! Error type shared across the engine and every provider adapter.

/// Engine-wide error. Adapters map transport/protocol failures into these so the
/// UI sees one error vocabulary regardless of provider.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transport error: {0}")]
    Transport(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("codec error: {0}")]
    Codec(String),

    #[error("provider not connected")]
    NotConnected,

    #[error("capability not supported by provider: {0}")]
    Unsupported(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    /// The addressed run has already left the provider's live execution set.
    /// Hosts may treat cancellation of this state as idempotent while waiting
    /// for the provider's terminal lifecycle event to finish projecting.
    #[error("run is not active: {0}")]
    RunNotActive(crate::RunId),

    #[error("io error: {0}")]
    Io(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Codec(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}
