use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    CurrentAgent,
    Reference,
    External,
}

impl CandidateKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::CurrentAgent => "current-agent",
            Self::Reference => "reference",
            Self::External => "external",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    Simulation,
    ExternalTrace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneKind {
    Single,
    EqualBudgetSingle,
    NaiveParallel,
    WorkGraphStrong,
    WorkGraphCheapSupport,
    WorkGraphDiverseReview,
    WorkGraphCloud,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSpec {
    pub id: String,
    pub kind: LaneKind,
    pub root_model: String,
    pub support_model: String,
    pub reviewer_model: Option<String>,
    pub token_budget: u64,
    pub max_parallel_tasks: usize,
}

impl LaneSpec {
    pub fn catalog(strong: &str, cheap: &str, reviewer: &str) -> Vec<Self> {
        vec![
            Self::new("single", LaneKind::Single, strong, strong, None, 90_000, 1),
            Self::new(
                "equal-budget-single",
                LaneKind::EqualBudgetSingle,
                strong,
                strong,
                None,
                240_000,
                1,
            ),
            Self::new(
                "naive-parallel",
                LaneKind::NaiveParallel,
                strong,
                strong,
                None,
                240_000,
                4,
            ),
            Self::new(
                "work-graph-strong",
                LaneKind::WorkGraphStrong,
                strong,
                strong,
                None,
                240_000,
                4,
            ),
            Self::new(
                "work-graph-cheap-support",
                LaneKind::WorkGraphCheapSupport,
                strong,
                cheap,
                None,
                240_000,
                4,
            ),
            Self::new(
                "work-graph-diverse-review",
                LaneKind::WorkGraphDiverseReview,
                strong,
                cheap,
                Some(reviewer),
                240_000,
                4,
            ),
            Self::new(
                "work-graph-cloud",
                LaneKind::WorkGraphCloud,
                strong,
                cheap,
                Some(reviewer),
                240_000,
                4,
            ),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        id: &str,
        kind: LaneKind,
        root: &str,
        support: &str,
        reviewer: Option<&str>,
        token_budget: u64,
        max_parallel_tasks: usize,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            root_model: root.into(),
            support_model: support.into(),
            reviewer_model: reviewer.map(str::to_string),
            token_budget,
            max_parallel_tasks,
        }
    }

    pub fn is_work_graph(&self) -> bool {
        matches!(
            self.kind,
            LaneKind::WorkGraphStrong
                | LaneKind::WorkGraphCheapSupport
                | LaneKind::WorkGraphDiverseReview
                | LaneKind::WorkGraphCloud
        )
    }
}

#[derive(Clone, Debug)]
pub struct FileFixture {
    pub path: String,
    pub content: String,
}

impl FileFixture {
    pub fn new(path: &str, content: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProjectSpec {
    pub id: String,
    pub initial_files: Vec<FileFixture>,
    pub dirty_user_files: Vec<FileFixture>,
    pub solution_files: Vec<FileFixture>,
    pub allowed_changed_paths: BTreeSet<String>,
    pub cloud_eligible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRole {
    Inspect,
    Provision,
    Implement,
    Generate,
    Review,
    Verify,
}

impl TaskRole {
    pub fn writes(self) -> bool {
        matches!(self, Self::Implement | Self::Generate)
    }

    pub fn cheap_eligible(self) -> bool {
        matches!(self, Self::Inspect | Self::Provision)
    }
}

#[derive(Clone, Debug)]
pub struct TaskSpec {
    pub id: String,
    pub role: TaskRole,
    pub dependencies: Vec<String>,
    pub resources: Vec<String>,
    pub outputs: Vec<String>,
    pub write_scope: BTreeSet<String>,
    pub duration_ms: u64,
    pub token_estimate: u64,
    pub cloud_eligible: bool,
}

#[derive(Clone, Debug)]
pub struct ResourceSpec {
    pub id: String,
    pub kind: String,
    pub provision_ms: u64,
    pub ttl_ms: u64,
    pub reusable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultInjection {
    None,
    ResourceProvisionFailure,
    WorkerCrashAfterArtifact,
    SourceBaselineDrift,
    ResourceExpiry,
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub id: String,
    pub family: String,
    pub title: String,
    pub prompt: String,
    pub projects: Vec<ProjectSpec>,
    pub tasks: Vec<TaskSpec>,
    pub resources: Vec<ResourceSpec>,
    pub final_artifacts: BTreeSet<String>,
    pub expected_delegate: bool,
    pub requires_independent_review: bool,
    pub fault: FaultInjection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub id: String,
    pub path: String,
    pub baseline_sha: String,
    pub allowed_changed_paths: BTreeSet<String>,
    pub cloud_eligible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicTaskManifest {
    pub schema_version: u32,
    pub scenario_id: String,
    pub title: String,
    pub prompt: String,
    pub projects: Vec<ProjectManifest>,
    pub lane: LaneSpec,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateControl {
    pub fault: FaultInjection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateRequest {
    pub schema_version: u32,
    pub workspace_path: String,
    pub result_path: String,
    pub task: PublicTaskManifest,
    pub control: CandidateControl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceAuthority {
    SelfReported,
    HostSimulation,
    ProductionHost,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanReceipt {
    pub graph_id: String,
    pub authority: TraceAuthority,
    pub task_ids: Vec<String>,
    pub resource_ids: Vec<String>,
    pub validated_at_ms: u64,
    pub delegated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Completed,
    Failed,
    Blocked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskReceipt {
    pub id: String,
    pub attempt: u32,
    pub role: TaskRole,
    pub dependencies: Vec<String>,
    pub resources: Vec<String>,
    pub model: String,
    pub model_tier: String,
    pub harness: String,
    pub workspace_id: String,
    pub write_scope: BTreeSet<String>,
    pub reserved_tokens: u64,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub outcome: TaskOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOutcome {
    Ready,
    Failed,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceReceipt {
    pub resource_id: String,
    pub instance_id: String,
    pub attempt: u32,
    pub kind: String,
    pub requested_ms: u64,
    pub ready_ms: Option<u64>,
    pub expires_ms: Option<u64>,
    pub released_ms: Option<u64>,
    pub outcome: ResourceOutcome,
    pub used_by: Vec<String>,
    pub health_checks: u32,
    pub host_supervised: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactReceipt {
    pub artifact_id: String,
    pub producer_task: String,
    pub source_baselines: BTreeMap<String, String>,
    pub input_artifact_shas: Vec<String>,
    pub content_sha256: String,
    pub integrity_sha256: String,
    pub produced_ms: u64,
    pub consumed_by: Vec<String>,
    pub verified_by: Vec<String>,
    pub stale: bool,
    pub rejected: bool,
}

impl ArtifactReceipt {
    pub fn expected_integrity(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.artifact_id.as_bytes());
        hasher.update([0]);
        hasher.update(self.producer_task.as_bytes());
        hasher.update([0]);
        for (project, baseline) in &self.source_baselines {
            hasher.update(project.as_bytes());
            hasher.update([0]);
            hasher.update(baseline.as_bytes());
            hasher.update([0]);
        }
        for input in &self.input_artifact_shas {
            hasher.update(input.as_bytes());
            hasher.update([0]);
        }
        hasher.update(self.content_sha256.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DependencyWakeupReceipt {
    pub task_id: String,
    pub dependency_id: String,
    pub dependency_kind: String,
    pub at_ms: u64,
    pub host_generated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryReceipt {
    pub failed_subject: String,
    pub replacement_subject: String,
    pub reason: String,
    pub preserved_artifact_shas: Vec<String>,
    pub restarted_subjects: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationReceipt {
    pub verifier_task: String,
    pub fresh_workspace: bool,
    pub checked_artifact_shas: Vec<String>,
    pub checks: Vec<String>,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceEvent {
    pub sequence: u64,
    pub at_ms: u64,
    pub kind: String,
    pub subject: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageReceipt {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub useful_tokens: u64,
    pub model_polling_tokens: u64,
    pub duplicate_setup_tokens: u64,
    pub cost_usd: f64,
    pub wall_ms: u64,
    pub agent_ms: u64,
    pub peak_reserved_tokens: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SafetyReceipt {
    pub raw_process_handoffs: Vec<String>,
    pub unauthorized_writes: Vec<String>,
    pub lost_user_changes: Vec<String>,
    pub permission_widenings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InteractionReceipt {
    pub default_flow: bool,
    pub setup_actions: u32,
    pub completion_actions: u32,
    pub model_choice_required: bool,
    pub agent_configuration_required: bool,
    pub version_control_knowledge_required: bool,
    pub advanced_details_collapsed: bool,
    pub plain_language_progress: bool,
    pub exposed_internal_terms: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateResult {
    pub schema_version: u32,
    pub candidate_id: String,
    pub scenario_id: String,
    pub lane_id: String,
    pub production_trace_id: Option<String>,
    pub delegated: bool,
    pub delegation_reason: String,
    pub plan: Option<PlanReceipt>,
    pub tasks: Vec<TaskReceipt>,
    pub resources: Vec<ResourceReceipt>,
    pub artifacts: Vec<ArtifactReceipt>,
    pub wakeups: Vec<DependencyWakeupReceipt>,
    pub recoveries: Vec<RecoveryReceipt>,
    pub verification: Option<VerificationReceipt>,
    pub events: Vec<TraceEvent>,
    pub usage: UsageReceipt,
    pub safety: SafetyReceipt,
    pub interaction: Option<InteractionReceipt>,
    pub claimed_complete: bool,
    pub error: Option<String>,
}

impl CandidateResult {
    pub fn total_tokens(&self) -> u64 {
        self.usage
            .input_tokens
            .saturating_add(self.usage.output_tokens)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardFailure {
    ArtifactLineageInvalid,
    AuthoritativePlanMissing,
    BehavioralFailure,
    BudgetOversubscribed,
    CleanupMissing,
    DependencyOrderViolation,
    DuplicateResourceSetup,
    HostWakeupMissing,
    IndependentVerificationMissing,
    ModelPollingDuringWait,
    NonTechnicalDefaultFlowMissing,
    ParallelismLimitExceeded,
    ProductionTraceMissing,
    RawProcessHandoff,
    RecoveryDiscardedGoodWork,
    ResourceLifecycleViolation,
    StaleArtifactConsumed,
    UnsafeWriterSharing,
    UnnecessaryDelegation,
    UnverifiedCompletion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema_version: u32,
    pub run_id: String,
    pub evidence_level: EvidenceLevel,
    pub candidate: CandidateKind,
    pub scenario_id: String,
    pub scenario_family: String,
    pub repetition: u32,
    pub lane: LaneSpec,
    pub workspace_path: String,
    pub result: CandidateResult,
    pub checks: Vec<CheckResult>,
    pub hard_failures: BTreeSet<HardFailure>,
    pub behavioral_correctness: f64,
    pub lifecycle_conformance: f64,
    pub efficiency_score: f64,
}

impl RunRecord {
    pub fn passed(&self) -> bool {
        self.hard_failures.is_empty()
            && self.behavioral_correctness >= 1.0
            && (!self.lane.is_work_graph() || self.lifecycle_conformance >= 1.0)
            && self.result.claimed_complete
            && self.result.error.is_none()
    }

    pub fn total_tokens(&self) -> u64 {
        self.result.total_tokens()
    }
}
