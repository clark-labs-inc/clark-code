//! Evidence-first system cartography primitives.
//!
//! Scout workers are replaceable, read-only sensors. The ledger is the
//! canonical single-writer authority: workers may propose claims and attach
//! evidence, while only the root may adjudicate, retract, supersede, or seal.

mod contract;
mod enterprise;
mod ledger;
mod measurement;
mod report;
mod validation;

pub use contract::{
    Adjudication, AssignmentRecord, AssignmentStatus, ClaimId, ClaimProposal, ClaimRecord,
    ClaimStatus, ClaimUpdate, ConfidenceInterval, EvidenceArtifact, EvidenceCheck, EvidenceId,
    EvidenceKind, EvidenceProducer, EvidenceRecord, Measurement, OfflinePocControls, ProofTier,
    RunnerId, ScoutActor, ScoutAssignment, ScoutCapabilities, ScoutCharter, ScoutEvent,
    ScoutEventKind, ScoutLimits, ScoutPhase, ScoutRole, ScoutRunId, ScoutSnapshot, ScoutVerdict,
    SealDisposition, VerificationOutcome, WorkerAssignmentId, WorkerEnvelope,
};
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
pub use ledger::{CompletionCheck, ScoutLedger};
pub use measurement::{compute_measurement, MeasurementComputation, MeasurementMethod};

#[cfg(test)]
mod tests;
