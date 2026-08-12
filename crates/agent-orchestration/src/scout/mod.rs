//! Enterprise system-cartography contracts and pure graph projections.
//!
//! Runtime run, task, evidence, and graph authority lives in the hosted
//! cartography backend. This crate contains only its portable data contracts
//! and deterministic projection machinery.

mod enterprise;
pub use enterprise::{
    enterprise_event_root, project_event_slice, AuthorityRef, CoverageCellId, CoverageKey,
    CoverageObservation, CoverageStatus, DiscoveryCharterObservation, DiscoveryPassSealObservation,
    EnterpriseAffectedProjection, EnterpriseBatch, EnterpriseBatchBundle, EnterpriseBatchId,
    EnterpriseBatchInclusionReceipt, EnterpriseCheckpointCursor, EnterpriseCheckpointObservation,
    EnterpriseClassification, EnterpriseCompletion, EnterpriseConflict, EnterpriseEdgeId,
    EnterpriseEdgeKind, EnterpriseEntityId, EnterpriseEntityKind, EnterpriseEvent,
    EnterpriseEventId, EnterpriseFact, EnterpriseGrantBundle, EnterpriseGrantScope,
    EnterpriseGraph, EnterpriseId, EnterpriseLedgerCheckpoint, EnterpriseLedgerCommitment,
    EnterpriseLedgerSummary, EnterpriseMergeReport, EnterpriseProjectionCursor,
    EnterpriseProjectionSlice, EnterpriseProjectionWork, EnterpriseProvenance, EnterpriseQuery,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerProposal, EnterpriseSignerRole,
    EnterpriseSigningKey, EnterpriseSnapshot, EnterpriseSnapshotCommitment,
    EnterpriseSnapshotCommitmentV2, EnterpriseTrustChain, EnterpriseTrustManifest,
    EnterpriseTrustPolicy, FrontierKey, FrontierObservation, FrontierState, FrontierTaskId,
    GraphEdgeObservation, GraphEntityObservation, MaterializedCharter, MaterializedCoverage,
    MaterializedDiscoveryPass, MaterializedEdge, MaterializedEntity, MaterializedFrontier,
    MaterializedSimulationContract, QualifiedLifecycle, SimulationContractObservation,
    VerifiedEnterpriseBatch, VerifiedEnterpriseCheckpoint, VerifiedEnterpriseInclusion,
    ENTERPRISE_LEDGER_COMMITMENT_SCHEMA_VERSION, ENTERPRISE_SCHEMA_VERSION,
    ENTERPRISE_SIGNED_BATCH_SCHEMA_VERSION, ENTERPRISE_SNAPSHOT_COMMITMENT_SCHEMA_VERSION,
    ENTERPRISE_SNAPSHOT_COMMITMENT_V2_SCHEMA_VERSION, ENTERPRISE_TRUST_SCHEMA_VERSION,
    MAX_ENTERPRISE_EVENTS_PER_BATCH,
};
