//! Convergent enterprise cartography.
//!
//! Scout's run ledger remains the authority for claims and proof. This module
//! is the durable-data contract beneath many runs: immutable, content-addressed
//! observations from independent machines merge by set union and materialize
//! into one deterministic organization graph.

mod contract;
mod graph;

pub use contract::{
    AuthorityRef, CoverageCellId, CoverageKey, CoverageObservation, CoverageStatus,
    DiscoveryCharterObservation, DiscoveryPassSealObservation, EnterpriseBatch,
    EnterpriseBatchBundle, EnterpriseBatchId, EnterpriseBatchInclusionReceipt,
    EnterpriseCheckpointCursor, EnterpriseCheckpointObservation, EnterpriseClassification,
    EnterpriseEdgeId, EnterpriseEdgeKind, EnterpriseEntityId, EnterpriseEntityKind,
    EnterpriseEvent, EnterpriseEventId, EnterpriseFact, EnterpriseGrantBundle,
    EnterpriseGrantScope, EnterpriseId, EnterpriseLedgerCheckpoint, EnterpriseLedgerCommitment,
    EnterpriseLedgerSummary, EnterpriseProvenance, EnterpriseSignedBatch, EnterpriseSignerGrant,
    EnterpriseSignerProposal, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseSnapshotCommitment, EnterpriseSnapshotCommitmentV2, EnterpriseTrustChain,
    EnterpriseTrustManifest, EnterpriseTrustPolicy, FrontierKey, FrontierObservation,
    FrontierState, FrontierTaskId, GraphEdgeObservation, GraphEntityObservation,
    SimulationContractObservation, VerifiedEnterpriseBatch, VerifiedEnterpriseCheckpoint,
    VerifiedEnterpriseInclusion, ENTERPRISE_LEDGER_COMMITMENT_SCHEMA_VERSION,
    ENTERPRISE_SCHEMA_VERSION, ENTERPRISE_SIGNED_BATCH_SCHEMA_VERSION,
    ENTERPRISE_SNAPSHOT_COMMITMENT_SCHEMA_VERSION,
    ENTERPRISE_SNAPSHOT_COMMITMENT_V2_SCHEMA_VERSION, ENTERPRISE_TRUST_SCHEMA_VERSION,
    MAX_ENTERPRISE_EVENTS_PER_BATCH,
};
pub use graph::{
    enterprise_event_root, project_event_slice, EnterpriseAffectedProjection, EnterpriseCompletion,
    EnterpriseConflict, EnterpriseGraph, EnterpriseMergeReport, EnterpriseProjectionCursor,
    EnterpriseProjectionSlice, EnterpriseProjectionWork, EnterpriseQuery, EnterpriseSnapshot,
    MaterializedCharter, MaterializedCoverage, MaterializedDiscoveryPass, MaterializedEdge,
    MaterializedEntity, MaterializedFrontier, MaterializedSimulationContract, QualifiedLifecycle,
};

#[cfg(test)]
mod tests;
