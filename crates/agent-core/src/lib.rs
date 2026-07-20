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

pub mod codec;
pub mod domain;
pub mod error;
pub mod ids;
pub mod projection;
pub mod provider;

pub use domain::*;
pub use error::{Error, Result};
pub use ids::*;
pub use projection::{apply, reduce_all, Snapshot, TimelineItem};
pub use provider::{
    ClientResponse, CollaborationMode, PlanDecision, PlanImplementationContext, PromptInput,
    Provider, ProviderCapabilities, ProviderConfig, ResumeItem, ResumeTranscript, Session,
    SessionOptions,
};
