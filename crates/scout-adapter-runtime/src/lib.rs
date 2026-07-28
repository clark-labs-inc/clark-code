//! Bounded target-side execution for enterprise Scout adapters.
//!
//! Public request and response types contain only target-bound handles and safe
//! normalized evidence. Credential values and provider continuation tokens stay
//! inside the private runtime boundary.

mod aws;
mod error;
mod gcp;
mod github;
mod gitlab;
mod process;
mod process_support;
mod route_registry;
mod service;
mod service_support;
mod types;
mod vault;
mod vault_io;
mod wire;

pub use route_registry::{
    adapter_coverage_manifest, AdapterCoverageManifest, AdapterRouteManifest,
};
pub use service::{RuntimeConfig, ScoutAdapterService};
pub use types::{
    AuthCandidate, AuthCandidateHandle, AuthCandidateSource, CensusRequest, CensusResponse,
    FetchPageResponse, SafeFailure, SafeFailureCode, ToolCapability, ToolKind, VerifyAuthRequest,
    VerifyAuthResponse, RUNTIME_PROTOCOL_VERSION,
};
pub use wire::{
    dispatch, ScoutAdapterRequest, ScoutAdapterResponse, MAX_ADAPTER_REQUEST_BYTES, SERVICE_NAME,
};

#[cfg(test)]
mod tests;
