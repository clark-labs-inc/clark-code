//! Deterministic scheduling contracts for exhaustive enterprise Scout runs.
//!
//! This crate owns no clock, network, filesystem, credential, or provider
//! access. Callers supply time and persist the serializable state. Opaque
//! authorization and cursor handles remain bound to an exact execution target.

mod manifest;
mod model;
mod state;

pub use manifest::{ExpansionRule, QuotaKey, QuotaPolicy, RouteKind, ScheduleManifest};
pub use model::{
    CompletionDisposition, LeaseClaim, PageCompletion, RetryClass, SchedulerReceipt,
    SchedulerTaskId, TaskOrigin, TaskSpec, TaskStatus, TerminalDisposition,
};
pub use state::Scheduler;

#[cfg(test)]
mod tests;
