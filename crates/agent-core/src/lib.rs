//! # agent-core
//!
//! Provider-agnostic engine for clark-desktop. Houses the normalized domain
//! model, the typed event-projection reducers, and the [`Provider`] abstraction
//! that lets the app talk to many agentic backends (ACP CLI agents, the Clark
//! runtime, …) through one interface.
//!
//! The crate is deliberately runtime-light: the domain model and projection are
//! pure and compile to both native (desktop/mobile via Tauri) and `wasm32`. Only
//! the transports and adapters pull in `tokio` (behind the `native` feature).
//!
//! Design rule mirrored from the product: **the agent decides, the client
//! renders.** Projection turns typed events into a [`Snapshot`]; it never infers
//! intent from natural-language text.

pub mod access_failure;
pub mod codec;
pub mod domain;
pub mod error;
pub mod ids;
pub mod projection;
pub mod provider;
pub mod recovery;

pub use access_failure::classify_provider_access_failure;
pub use domain::*;
pub use error::{Error, Result};
pub use ids::*;
pub use projection::{apply, normalize_snapshot_value, reduce_all, Snapshot, TimelineItem};
pub use provider::{
    AttachmentKind, BackgroundTask, BackgroundTaskState, ClientResponse, CollaborationMode,
    ExperimentCapability, ModelCapability, OutputStyleCapability, PlanDecision,
    PlanImplementationContext, PromptInput, Provider, ProviderCapabilities, ProviderConfig,
    ProviderConfiguration, ProviderConfigurationChange, ResumeItem, ResumeTranscript, Session,
    SessionOptions, SideQuestionFuture,
};
pub use recovery::{
    ExecutionBoundaryReceipt, ExecutionRecovery, ProviderFailureClass, ProviderIncident,
    ProviderIncidentCategory, ProviderIncidentScope, ProviderIncidentStatus,
    ProviderRequestDiagnostics, ProviderRetryCounts,
};
