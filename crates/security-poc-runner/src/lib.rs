//! Bounded, network-denied, disposable-workspace security PoC runner.
//!
//! This crate runs one positive/negative proof-of-concept control in a fresh
//! repository copy on a single host and seals a `managed_disposable` receipt.
//! It is used two ways with identical semantics:
//!
//! - **Local tool path** — `provider-local`'s `security_poc_execute` calls
//!   [`run`] directly against the local checkout.
//! - **Remote target service** — `clark-exec-server` routes
//!   [`SERVICE_NAME`] to [`dispatch`], which deserializes a
//!   [`SecurityPocRunRequest`], runs it on the target, and returns a
//!   [`SecurityPocRunResponse`]. Because the receipt is constructed here with
//!   the same digests and `managed_disposable` containment, the scan contract's
//!   acceptance check holds regardless of which host produced it.

mod runner;
#[cfg(test)]
mod tests;
mod types;

pub use runner::run;
pub use types::{
    sha256_hex, validate_id, PocControl, PocExecutionMetadata, PocInventoryFile, PocLanguage,
    SecurityPocReceipt, SecurityPocRunRequest, SecurityPocRunResponse, SERVICE_NAME,
};

use std::path::Path;

/// Target-service entry point. The PoC runner awaits subprocess output, so it
/// is dispatched asynchronously (like `scout_adapter_runtime`), not through the
/// blocking lane used by the pure CPU-bound services.
pub async fn dispatch(service: &str, root: &Path, request: &[u8]) -> Result<Vec<u8>, String> {
    if service != SERVICE_NAME {
        return Err(format!("unsupported target service: {service}"));
    }
    let request: SecurityPocRunRequest = serde_json::from_slice(request)
        .map_err(|error| format!("security PoC request: {error}"))?;
    let outcome = run(root, &request).await?;
    let response = SecurityPocRunResponse {
        receipt: outcome.receipt,
        stdout: outcome.stdout,
        stderr: outcome.stderr,
    };
    serde_json::to_vec(&response).map_err(|error| format!("security PoC response: {error}"))
}
