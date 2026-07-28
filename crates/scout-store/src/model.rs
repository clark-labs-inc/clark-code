use agent_orchestration::{
    EnterpriseBatchId, EnterpriseClassification, EnterpriseConflict, EnterpriseEdgeKind,
    EnterpriseEntityId, EnterpriseEntityKind, EnterpriseId, EnterpriseSignedBatch,
    EnterpriseSnapshotCommitment, EnterpriseSnapshotCommitmentV2, MaterializedCharter,
    MaterializedEdge, MaterializedEntity,
};
use serde::{Deserialize, Serialize};

use crate::checkpoint::CheckpointExchangeBundle;
use crate::ledger_authority::LedgerAuthorityWork;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScoutStoreRequest {
    Ingest {
        enterprise_id: EnterpriseId,
        envelope: Box<EnterpriseSignedBatch>,
    },
    IssueCheckpoint {
        enterprise_id: EnterpriseId,
        now_ms: u64,
    },
    CheckpointStatus {
        enterprise_id: EnterpriseId,
    },
    ExportCheckpoint {
        enterprise_id: EnterpriseId,
        sequence: u64,
    },
    ObserveCheckpoint {
        enterprise_id: EnterpriseId,
        exchange: Box<CheckpointExchangeBundle>,
    },
    EnqueueOutbox {
        enterprise_id: EnterpriseId,
        batch_id: EnterpriseBatchId,
    },
    BeginOutboxDelivery {
        enterprise_id: EnterpriseId,
        batch_id: EnterpriseBatchId,
        attempt_id: String,
        previous_attempt_id: Option<String>,
    },
    ResolveOutboxDelivery {
        enterprise_id: EnterpriseId,
        batch_id: EnterpriseBatchId,
        attempt_id: String,
        resolution: OutboxResolution,
        resolution_id: String,
    },
    OutboxStatus {
        enterprise_id: EnterpriseId,
        batch_id: EnterpriseBatchId,
    },
    ListOutbox {
        enterprise_id: EnterpriseId,
        filter: OutboxStateFilter,
        cursor: Option<String>,
        limit: usize,
    },
    Rebuild {
        enterprise_id: EnterpriseId,
    },
    Status {
        enterprise_id: EnterpriseId,
    },
    Entities {
        enterprise_id: EnterpriseId,
        query: EntityQuery,
    },
    QualifiedEntities {
        enterprise_id: EnterpriseId,
        query: QualifiedEntityQuery,
    },
    Edges {
        enterprise_id: EnterpriseId,
        query: EdgeQuery,
    },
    QualifiedEdges {
        enterprise_id: EnterpriseId,
        query: QualifiedEdgeQuery,
    },
    Neighborhood {
        enterprise_id: EnterpriseId,
        seed: EnterpriseEntityId,
        depth: u8,
        limit: usize,
    },
    QualifiedNeighborhood {
        enterprise_id: EnterpriseId,
        query: NeighborhoodQuery,
    },
    Batches {
        enterprise_id: EnterpriseId,
        cursor: Option<String>,
        limit: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScoutStoreResponse {
    Ingested {
        outcome: IngestOutcome,
        receipt: IndexReceipt,
    },
    CheckpointIssued {
        status: AuthenticatedCheckpointStatus,
        idempotent: bool,
    },
    CheckpointStatus {
        status: Option<AuthenticatedCheckpointStatus>,
    },
    CheckpointExported {
        exchange: Box<CheckpointExchangeBundle>,
    },
    CheckpointObserved {
        status: ObservedCheckpointStatus,
        idempotent: bool,
    },
    OutboxUpdated {
        entry: OutboxEntry,
        idempotent: bool,
    },
    OutboxStatus {
        entry: Option<OutboxEntry>,
    },
    OutboxListed {
        page: OutboxPage,
    },
    Rebuilt(IndexReceipt),
    Status {
        status: Box<IndexedStatus>,
        receipt: IndexReceipt,
    },
    Entities {
        page: EntityPage,
        receipt: IndexReceipt,
    },
    Edges {
        page: EdgePage,
        receipt: IndexReceipt,
    },
    Neighborhood {
        page: NeighborhoodPage,
        receipt: IndexReceipt,
    },
    Batches {
        page: BatchPage,
        receipt: IndexReceipt,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestOutcome {
    Inserted,
    AlreadyPresent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxResolution {
    Acked,
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStateFilter {
    Pending,
    InFlight,
    PendingOrInFlight,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutboxState {
    Pending,
    InFlight {
        attempt_id: String,
    },
    Acked {
        attempt_id: String,
        resolution_id: String,
    },
    Rejected {
        attempt_id: String,
        resolution_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxEntry {
    pub enterprise_id: EnterpriseId,
    pub batch_id: EnterpriseBatchId,
    pub revision: u64,
    pub state: OutboxState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxPage {
    pub entries: Vec<OutboxEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedCheckpointStatus {
    pub checkpoint_id: String,
    pub manifest_id: String,
    pub sequence: u64,
    pub issued_at_ms: u64,
    pub batch_root: String,
    pub event_root: String,
    pub batch_count: u64,
    pub event_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_commitment: Option<EnterpriseSnapshotCommitment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_commitment_v2: Option<EnterpriseSnapshotCommitmentV2>,
    pub checkpoint_covers_current_ledger: bool,
    pub checkpoint_covers_current_projection: bool,
    pub uncheckpointed_batch_count: u64,
    pub uncheckpointed_event_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedCheckpointStatus {
    pub coordinator_id: String,
    pub anchor_manifest_id: String,
    pub checkpoint_id: String,
    pub sequence: u64,
    pub manifest_id: String,
    pub issued_at_ms: u64,
    pub batch_root: String,
    pub event_root: String,
    pub batch_count: u64,
    pub event_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_commitment: Option<EnterpriseSnapshotCommitment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_commitment_v2: Option<EnterpriseSnapshotCommitmentV2>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexReceipt {
    pub event_root: String,
    pub graph_digest: String,
    /// Supplemental partitioned event-set commitment during the dual-root migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_set_root_v1: Option<String>,
    /// Supplemental authenticated materialized-projection commitment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_map_root_v2: Option<String>,
    /// Enterprise-bound commitment to the graph digest and both supplemental roots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enterprise_snapshot_root_v2: Option<String>,
    pub batch_set_root: String,
    /// Exact authenticated-ledger storage work performed by this operation.
    #[serde(default)]
    pub ledger_authority_work: LedgerAuthorityWork,
    pub rebuilt: bool,
    #[serde(default)]
    pub derived_batches_read: usize,
    #[serde(default)]
    pub events_replayed: usize,
    /// Existing cached event identifiers scanned to update this receipt.
    #[serde(default)]
    pub event_ids_scanned: usize,
    /// Authenticated current entity rows read for this operation.
    #[serde(default)]
    pub entity_rows_read: usize,
    /// Authenticated current edge rows read for this operation.
    #[serde(default)]
    pub edge_rows_read: usize,
    /// Authenticated historical rows read for this operation.
    #[serde(default)]
    pub history_rows_read: usize,
    /// Authenticated coverage, frontier, or simulation rows read for this operation.
    #[serde(default)]
    pub auxiliary_rows_read: usize,
    /// Authenticated normalized conflict rows read for this operation.
    #[serde(default)]
    pub conflict_rows_read: usize,
    /// Normalized conflict rows inserted or updated by this operation.
    #[serde(default)]
    pub conflict_rows_written: usize,
    /// Normalized conflict rows deleted by this operation.
    #[serde(default)]
    pub conflict_rows_deleted: usize,
    /// Incident edges reconsidered because an endpoint changed.
    #[serde(default)]
    pub incident_edges_reclassified: usize,
    #[serde(default)]
    pub affected_projection_rows: usize,
    #[serde(default)]
    pub full_projection_fallback: bool,
    #[serde(default)]
    pub projection_rows_written: usize,
    #[serde(default)]
    pub projection_rows_deleted: usize,
    /// Compact authenticated commitment entries written by this operation.
    #[serde(default)]
    pub supplemental_rows_written: usize,
    /// Superseded compact commitment entries deleted by this operation.
    #[serde(default)]
    pub supplemental_rows_deleted: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedStatus {
    pub enterprise_id: EnterpriseId,
    /// Maximum classification represented by topology counts and conflict samples.
    #[serde(default)]
    pub max_classification: EnterpriseClassification,
    pub event_root: String,
    pub graph_digest: String,
    /// Supplemental partitioned event-set commitment during the dual-root migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_set_root_v1: Option<String>,
    /// Supplemental authenticated materialized-projection commitment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_map_root_v2: Option<String>,
    /// Enterprise-bound commitment to the graph digest and both supplemental roots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enterprise_snapshot_root_v2: Option<String>,
    pub batches: usize,
    pub events: usize,
    pub entities: usize,
    pub edges: usize,
    pub coverage_cells: usize,
    pub frontier_tasks: usize,
    pub simulation_contracts: usize,
    pub charter: Option<MaterializedCharter>,
    pub discovery_passes: usize,
    pub current_pass_id: Option<String>,
    pub current_pass_sealed_at_ms: Option<u64>,
    pub fixed_point: bool,
    pub base_completion_blockers: Vec<String>,
    pub conflict_count: usize,
    pub conflicts: Vec<EnterpriseConflict>,
}

impl IndexedStatus {
    pub fn completion_blockers_at(&self, now_ms: u64) -> Vec<String> {
        let mut blockers = self.base_completion_blockers.clone();
        if let (Some(charter), Some(sealed_at_ms)) = (&self.charter, self.current_pass_sealed_at_ms)
        {
            if now_ms.saturating_sub(sealed_at_ms) > charter.max_age_ms {
                blockers.push("current discovery pass exceeds the charter freshness policy".into());
            }
        }
        blockers
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityQuery {
    pub kind: Option<EnterpriseEntityKind>,
    pub provider_namespace: Option<String>,
    pub authority_scope: Option<String>,
    pub label_contains: Option<String>,
    pub critical: Option<bool>,
    pub cursor: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeQuery {
    pub kind: Option<EnterpriseEdgeKind>,
    pub from: Option<EnterpriseEntityId>,
    pub to: Option<EnterpriseEntityId>,
    pub cursor: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualifiedEntityQuery {
    pub kind: Option<EnterpriseEntityKind>,
    pub provider_namespace: Option<String>,
    pub authority_scope: Option<String>,
    pub label_contains: Option<String>,
    pub critical: Option<bool>,
    pub as_of_ms: Option<u64>,
    #[serde(default)]
    pub include_retired: bool,
    #[serde(default)]
    pub max_classification: EnterpriseClassification,
    pub cursor: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualifiedEdgeQuery {
    pub kind: Option<EnterpriseEdgeKind>,
    pub from: Option<EnterpriseEntityId>,
    pub to: Option<EnterpriseEntityId>,
    pub as_of_ms: Option<u64>,
    #[serde(default)]
    pub include_retired: bool,
    #[serde(default)]
    pub max_classification: EnterpriseClassification,
    pub cursor: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeighborhoodQuery {
    pub seed: EnterpriseEntityId,
    pub depth: u8,
    pub limit: usize,
    pub as_of_ms: Option<u64>,
    #[serde(default)]
    pub include_retired: bool,
    #[serde(default)]
    pub max_classification: EnterpriseClassification,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityPage {
    pub entities: Vec<MaterializedEntity>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgePage {
    pub edges: Vec<MaterializedEdge>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodPage {
    pub entities: Vec<MaterializedEntity>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedBatch {
    pub batch_id: String,
    pub event_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchPage {
    pub batches: Vec<IndexedBatch>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PageCursor {
    pub enterprise_id: EnterpriseId,
    pub event_root: String,
    pub graph_digest: String,
    pub projection_version: u16,
    pub filter_digest: String,
    pub last_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthenticatedPageCursor {
    pub payload: String,
    pub mac: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OutboxPageCursor {
    pub version: u16,
    pub enterprise_id: EnterpriseId,
    pub filter: OutboxStateFilter,
    pub last_batch_id: String,
}
