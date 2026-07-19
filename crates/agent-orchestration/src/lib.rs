//! Safe execution and orchestration primitives for Clark Desktop.
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
