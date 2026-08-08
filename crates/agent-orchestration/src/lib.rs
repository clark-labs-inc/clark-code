//! Safe execution and orchestration primitives for Agent Desktop.
//!
//! Every run can use the root execution ledger even when it has no children.
//! Delegated roles remain deliberately read-only: the root coding agent owns
//! the sole writer lease, and parallel writers require a different contract
//! with isolated worktrees and deterministic arbitration.

mod budget;
mod contract;
mod control;
mod execution;
mod harness;
mod multi_repo;
mod policy;
mod provider_harness;
mod runtime;
mod scout;

pub use budget::{BudgetConfig, BudgetReservation, BudgetSnapshot, SharedBudget, UsageCharge};
pub use contract::{
    AgentPath, AgentRecord, AgentRole, AgentStatus, ClaimEvidence, CommandEvidence, DeliveryMode,
    HarnessKind, Message, OrchestrationId, ReadOnlyTask, ReportDecision, ReportStatus,
    StructuredReport, TaskId, TestEvidence,
};
pub use control::{ControlPlane, ControlSnapshot, SpawnReservation};
pub use execution::{
    AttemptId, AttemptOutcome, ChildExecution, EvidenceReceipt, ExecutionAttempt, ExecutionEvent,
    ExecutionEventKind, ExecutionId, ExecutionLedger, ExecutionPolicy, ExecutionSnapshot,
    ExecutionState, ExecutionUsage, FailureClass, RecoveryDecision, ToolEvidence,
    ToolExecutionStatus,
};
pub use harness::{AttemptContext, HarnessAttempt, HarnessError, HarnessEvent, ReadOnlyHarness};
pub use multi_repo::{
    repository_result_tree_sha256, ChangePackageDescriptor, CheckoutKind, ContractDecision,
    DecompositionDecision, IntegrationCheck, IntegrationCheckReceipt, IntegrationHarnessAttempt,
    IntegrationReceipt, IsolationKind, ModelTier, MultiRepoCoordinator, MultiRepoCoordinatorEvent,
    MultiRepoEventSink, MultiRepoIntegrationHarness, MultiRepoPlan, MultiRepoReaderHarness,
    MultiRepoReviewHarness, MultiRepoRunResult, MultiRepoTask, MultiRepoTaskRole,
    MultiRepoWriterHarness, PlanningReceipt, ReaderFailure, ReaderHarnessAttempt, ReaderReport,
    RecoveryReceipt, RepositoryBaseline, RepositoryContractEdge, RepositoryId, ReviewDecision,
    ReviewHarnessAttempt, ReviewReceipt, TaskExecutionReceipt, TaskRunOutcome, WriterFailure,
    WriterHarnessAttempt,
};
pub use policy::{
    AdmissionDecision, AdmissionPolicy, AdmissionRequest, Authorization, ModelRate,
    OrchestrationPurpose, Rejection, RiskSignals, WorkstreamEstimate,
};
pub use provider_harness::{
    ProviderFactory, ProviderHarness, ProviderHarnessConfig, ReadOnlyEnforcement, WorkspaceGuard,
};
pub use runtime::{
    Coordinator, CoordinatorError, CoordinatorEvent, CoordinatorEventSink, FanOutRequest,
    FanOutResult,
};
pub use scout::{
    compute_measurement as compute_scout_measurement, enterprise_event_root, project_event_slice,
    Adjudication as ScoutAdjudication, AssignmentRecord as ScoutAssignmentRecord,
    AssignmentStatus as ScoutAssignmentStatus, AuthorityRef, ClaimId as ScoutClaimId,
    ClaimProposal as ScoutClaimProposal, ClaimRecord as ScoutClaimRecord,
    ClaimStatus as ScoutClaimStatus, ClaimUpdate as ScoutClaimUpdate,
    CompletionCheck as ScoutCompletionCheck, ConfidenceInterval as ScoutConfidenceInterval,
    CoverageCellId, CoverageKey, CoverageObservation, CoverageStatus, DiscoveryCharterObservation,
    DiscoveryPassSealObservation, EnterpriseAffectedProjection, EnterpriseBatch,
    EnterpriseBatchBundle, EnterpriseBatchId, EnterpriseBatchInclusionReceipt,
    EnterpriseCheckpointCursor, EnterpriseCheckpointObservation, EnterpriseClassification,
    EnterpriseCompletion, EnterpriseConflict, EnterpriseEdgeId, EnterpriseEdgeKind,
    EnterpriseEntityId, EnterpriseEntityKind, EnterpriseEvent, EnterpriseEventId, EnterpriseFact,
    EnterpriseGrantBundle, EnterpriseGrantScope, EnterpriseGraph, EnterpriseId,
    EnterpriseLedgerCheckpoint, EnterpriseLedgerCommitment, EnterpriseLedgerSummary,
    EnterpriseMergeReport, EnterpriseProjectionCursor, EnterpriseProjectionSlice,
    EnterpriseProjectionWork, EnterpriseProvenance, EnterpriseQuery, EnterpriseSignedBatch,
    EnterpriseSignerGrant, EnterpriseSignerProposal, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseSnapshot, EnterpriseSnapshotCommitment, EnterpriseSnapshotCommitmentV2,
    EnterpriseTrustChain, EnterpriseTrustManifest, EnterpriseTrustPolicy,
    EvidenceArtifact as ScoutEvidenceArtifact, EvidenceCheck as ScoutEvidenceCheck,
    EvidenceId as ScoutEvidenceId, EvidenceKind as ScoutEvidenceKind, EvidenceProducer,
    EvidenceRecord as ScoutEvidenceRecord, FrontierKey, FrontierObservation, FrontierState,
    FrontierTaskId, GraphEdgeObservation, GraphEntityObservation, MaterializedCharter,
    MaterializedCoverage, MaterializedDiscoveryPass, MaterializedEdge, MaterializedEntity,
    MaterializedFrontier, MaterializedSimulationContract, Measurement as ScoutMeasurement,
    MeasurementComputation as ScoutMeasurementComputation,
    MeasurementMethod as ScoutMeasurementMethod, OfflinePocControls, ProofTier, QualifiedLifecycle,
    RunnerId as ScoutRunnerId, ScoutActor, ScoutAssignment, ScoutCapabilities, ScoutCharter,
    ScoutEvent, ScoutEventKind, ScoutLedger, ScoutLimits, ScoutPhase, ScoutRole, ScoutRunId,
    ScoutSnapshot, ScoutVerdict, SealDisposition, SimulationContractObservation,
    VerificationOutcome, VerifiedEnterpriseBatch, VerifiedEnterpriseCheckpoint,
    VerifiedEnterpriseInclusion, WorkerAssignmentId as ScoutWorkerAssignmentId,
    WorkerEnvelope as ScoutWorkerEnvelope, ENTERPRISE_LEDGER_COMMITMENT_SCHEMA_VERSION,
    ENTERPRISE_SCHEMA_VERSION, ENTERPRISE_SIGNED_BATCH_SCHEMA_VERSION,
    ENTERPRISE_SNAPSHOT_COMMITMENT_SCHEMA_VERSION,
    ENTERPRISE_SNAPSHOT_COMMITMENT_V2_SCHEMA_VERSION, ENTERPRISE_TRUST_SCHEMA_VERSION,
    MAX_ENTERPRISE_EVENTS_PER_BATCH,
};
