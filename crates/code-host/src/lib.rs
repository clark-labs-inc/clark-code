//! Reusable control plane for unattended coding-agent runtimes.
//!
//! `code-host` deliberately knows nothing about a model, shell, filesystem
//! executor, or desktop UI. It owns the stable boundaries that every
//! headless worker needs: strict JSONL control messages, registered projects,
//! plugin discovery/dispatch, cancellation, and a small durable trajectory.
//! A worker composes concrete providers and plugins on top of this crate.

mod contract;
mod host;
mod idempotency;
mod plugin;
mod protocol;
mod trajectory;

pub use contract::{
    CodingSessionRecipe, ProjectRegistration, ProjectRegistry, RegistryError,
    ScoutCartographyRecipe,
};
pub use host::{HeadlessHost, HostError};
pub use plugin::{
    HeadlessPlugin, PluginContext, PluginError, PluginManifest, PluginRegistry, ProgressReporter,
};
pub use protocol::{Request, RequestCommand, Response};
pub use trajectory::{TrajectoryRecord, TrajectoryStatus};

/// Wire version shared by the standalone worker and host adapters.
pub const PROTOCOL_VERSION: u32 = 2;
