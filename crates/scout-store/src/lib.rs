mod checkpoint;
mod index;
pub mod ledger_authority;
mod model;
mod outbox;
mod query;

use std::path::Path;

use agent_orchestration::{EnterpriseBatchBundle, EnterpriseBatchId, EnterpriseId};

pub use checkpoint::{CheckpointExchangeBundle, StoredCheckpointBundle};
pub use model::{
    AuthenticatedCheckpointStatus, BatchPage, EdgePage, EdgeQuery, EntityPage, EntityQuery,
    IndexReceipt, IndexedBatch, IndexedStatus, IngestOutcome, NeighborhoodPage, NeighborhoodQuery,
    ObservedCheckpointStatus, OutboxEntry, OutboxPage, OutboxResolution, OutboxState,
    OutboxStateFilter, QualifiedEdgeQuery, QualifiedEntityQuery, ScoutStoreRequest,
    ScoutStoreResponse,
};

pub const SERVICE_NAME: &str = "scout-store-v1";

pub fn dispatch(service: &str, root: &Path, request: &[u8]) -> Result<Vec<u8>, String> {
    if service != SERVICE_NAME {
        return Err(format!("unsupported target service: {service}"));
    }
    let request: ScoutStoreRequest =
        serde_json::from_slice(request).map_err(|error| format!("Scout index request: {error}"))?;
    let response = index::handle(root, request)?;
    serde_json::to_vec(&response).map_err(|error| format!("Scout index response: {error}"))
}

pub fn request(root: &Path, request: ScoutStoreRequest) -> Result<ScoutStoreResponse, String> {
    index::handle(root, request)
}

pub fn outbox_delivery_bundle(
    root: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
) -> Result<EnterpriseBatchBundle, String> {
    outbox::delivery_bundle(root, enterprise_id, batch_id)
}

#[cfg(test)]
mod outbox_tests;
#[cfg(test)]
mod query_tests;
#[cfg(test)]
mod tests;
