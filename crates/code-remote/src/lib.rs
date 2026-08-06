//! Remote control for a standalone [`clark-code-worker`] over system SSH.
//!
//! The connection is intentionally separate from `code-host`: the host owns
//! protocol and plugin policy, while this crate owns artifact deployment and a
//! correlated JSONL stdio transport. Credentials may be relayed as one
//! encrypted SSH-stdin bootstrap line; they are never written to the worker
//! config or placed in an argv string.

mod artifact;
mod process;
mod spec;
mod transport;

pub use artifact::{RemoteArch, RemoteArtifact, RemoteArtifactError};
pub use process::{
    RemoteWorker, RemoteWorkerError, RemoteWorkerFrame, RemoteWorkerInfo, RemoteWorkerProgress,
    RemoteWorkerRequest, RemoteWorkerSlot,
};
pub use spec::{RemoteWorkerSpec, SpecError};
