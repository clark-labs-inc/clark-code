use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    Scripted,
    External,
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneKind {
    Single,
    EqualBudgetSingle,
    MultiCheap,
    MultiStrong,
    MultiDiverseReview,
    CloudMixed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSpec {
    pub id: String,
    pub kind: LaneKind,
    pub root_model: String,
    pub worker_model: String,
    pub reviewer_model: Option<String>,
    pub token_budget: u64,
    pub max_parallel_writers: usize,
}

impl LaneSpec {
    pub fn catalog(strong: &str, cheap: &str, reviewer: &str) -> Vec<Self> {
        vec![
            Self::new("single", LaneKind::Single, strong, strong, None, 100_000, 1),
            Self::new(
                "equal-budget-single",
                LaneKind::EqualBudgetSingle,
                strong,
                strong,
                None,
                400_000,
                1,
            ),
            Self::new(
                "multi-cheap",
                LaneKind::MultiCheap,
                strong,
                cheap,
                None,
                400_000,
                4,
            ),
            Self::new(
                "multi-strong",
                LaneKind::MultiStrong,
                strong,
                strong,
                None,
                400_000,
                4,
            ),
            Self::new(
                "multi-diverse-review",
                LaneKind::MultiDiverseReview,
                strong,
                cheap,
                Some(reviewer),
                400_000,
                4,
            ),
            Self::new(
                "cloud-mixed",
                LaneKind::CloudMixed,
                strong,
                cheap,
                Some(reviewer),
                400_000,
                4,
            ),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        id: &str,
        kind: LaneKind,
        root: &str,
        worker: &str,
        reviewer: Option<&str>,
        token_budget: u64,
        max_parallel_writers: usize,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            root_model: root.into(),
            worker_model: worker.into(),
            reviewer_model: reviewer.map(str::to_string),
            token_budget,
            max_parallel_writers,
        }
    }

    pub fn is_multi(&self) -> bool {
        !matches!(self.kind, LaneKind::Single | LaneKind::EqualBudgetSingle)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    RepositoryGraph,
    AuthoritativePlanningReceipt,
    PinnedBaselines,
    ContractDecisionLedger,
    IsolatedWriterArtifacts,
    ParallelWriters,
    FreshIntegrationReplay,
    TargetedRecovery,
    CheapModelRouting,
    IndependentReview,
    CloudRepositoryWorker,
    TriggerDiscipline,
    NonTechnicalDefaultFlow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultInjection {
    None,
    ChildCrashAfterArtifact,
    BaselineDrift,
    StaleGeneratedClient,
    ReviewerFalseVeto,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositorySpec {
    pub id: String,
    pub initial_files: Vec<FileFixture>,
    pub dirty_user_files: Vec<FileFixture>,
    pub solution_files: Vec<FileFixture>,
    pub allowed_changed_paths: BTreeSet<String>,
    pub public_checks: Vec<String>,
    pub cloud_eligible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractEdge {
    pub id: String,
    pub producer_repo: String,
    pub consumer_repos: Vec<String>,
    pub artifact: String,
    pub compatibility_rule: String,
}

#[derive(Clone, Debug)]
pub enum HiddenCheck {
    FileContains {
        repo: String,
        path: String,
        needle: String,
    },
    FileEquals {
        repo: String,
        path: String,
        expected: String,
    },
    Python {
        name: String,
        script: String,
    },
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub id: String,
    pub family: String,
    pub title: String,
    pub prompt: String,
    pub repositories: Vec<RepositorySpec>,
    pub edges: Vec<ContractEdge>,
    pub hidden_checks: Vec<HiddenCheck>,
    pub required_capabilities: BTreeSet<Capability>,
    pub expected_delegate: bool,
    pub single_agent_trap: bool,
    pub fault: FaultInjection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositoryManifest {
    pub id: String,
    pub path: String,
    pub baseline_sha: String,
    pub allowed_changed_paths: BTreeSet<String>,
    pub public_checks: Vec<String>,
    pub cloud_eligible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicTaskManifest {
    pub schema_version: u32,
    pub scenario_id: String,
    pub title: String,
    pub prompt: String,
    pub repositories: Vec<RepositoryManifest>,
    pub contracts: Vec<ContractEdge>,
    pub lane: LaneSpec,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateControl {
    pub injected_fault: FaultInjection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateRequest {
    pub schema_version: u32,
    pub workspace_path: String,
    pub manifest_path: String,
    pub result_path: String,
    pub task: PublicTaskManifest,
    pub control: CandidateControl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRole {
    Planner,
    Reader,
    Writer,
    Integrator,
    Reviewer,
    Verifier,
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
    pub repo_id: Option<String>,
    pub role: TaskRole,
    pub dependencies: Vec<String>,
    pub model: String,
    pub model_tier: String,
    pub harness: String,
    pub isolated: bool,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub outcome: TaskOutcome,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangePackage {
    pub task_id: String,
    pub repo_id: String,
    pub base_sha: String,
    pub changed_paths: BTreeSet<String>,
    pub patch_path: String,
    pub patch_sha256: String,
    pub result_tree_sha256: String,
    pub isolation: String,
    pub tests: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractDecision {
    pub edge_id: String,
    pub producer_repo: String,
    pub consumer_repos: Vec<String>,
    pub artifact_sha256: String,
    pub compatibility_rule: String,
    pub approved_by: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryReceipt {
    pub failed_task_id: String,
    pub replacement_task_id: String,
    pub preserved_task_ids: Vec<String>,
    pub reused_artifact_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrationReceipt {
    pub fresh_workspace: bool,
    pub repo_baselines: BTreeMap<String, String>,
    pub repo_result_trees: BTreeMap<String, String>,
    pub applied_patch_sha256: Vec<String>,
    pub checks_run: Vec<String>,
    pub passed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageReceipt {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub useful_tokens: u64,
    pub duplicate_read_tokens: u64,
    pub cost_usd: f64,
    pub wall_ms: u64,
    pub agent_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SafetyReceipt {
    pub unauthorized_writes: Vec<String>,
    pub lost_user_changes: Vec<String>,
    pub permission_widenings: Vec<String>,
    pub destructive_actions: Vec<String>,
    pub baseline_moves: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanningReceipt {
    pub planner_task_id: String,
    pub plan_sha256: String,
    pub repository_baselines: BTreeMap<String, String>,
    pub delegated: bool,
    pub validated_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InteractionReceipt {
    /// True only when this trace came from the ordinary user-facing path,
    /// rather than an evaluator-only or advanced orchestration screen.
    pub default_flow: bool,
    /// User actions required before the agent can start. Selecting several
    /// projects in one picker counts as one action.
    pub setup_actions: u32,
    /// Separate Cloud approval prompts shown during one run.
    pub cloud_consent_prompts: u32,
    /// User actions required after the agent finishes to inspect and apply work.
    pub completion_actions: u32,
    pub model_choice_required: bool,
    pub agent_configuration_required: bool,
    pub version_control_knowledge_required: bool,
    pub advanced_details_collapsed: bool,
    pub plain_language_progress: bool,
    /// Internal terms visible on the default path, such as model names,
    /// worktrees, leases, patches, DAGs, or agent topology.
    pub exposed_internal_terms: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateResult {
    pub schema_version: u32,
    pub candidate_id: String,
    pub scenario_id: String,
    pub lane_id: String,
    pub delegated: bool,
    pub delegation_reason: String,
    #[serde(default)]
    pub planning: Option<PlanningReceipt>,
    pub tasks: Vec<TaskReceipt>,
    pub change_packages: Vec<ChangePackage>,
    pub contract_decisions: Vec<ContractDecision>,
    pub recoveries: Vec<RecoveryReceipt>,
    pub integration: Option<IntegrationReceipt>,
    pub usage: UsageReceipt,
    pub safety: SafetyReceipt,
    #[serde(default)]
    pub interaction: Option<InteractionReceipt>,
    pub claimed_complete: bool,
    pub error: Option<String>,
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
    AuthoritativePlanningReceiptMissing,
    BehavioralFailure,
    CloudWorkerMissing,
    ContractDecisionMissing,
    DirtyUserChangeLost,
    FreshIntegrationFailed,
    IndependentReviewMissing,
    InvalidChangePackage,
    ModelRoutingIncorrect,
    NonTechnicalDefaultFlowMissing,
    OutOfScopeWrite,
    ParallelWriterEvidenceMissing,
    PermissionOrDestructiveViolation,
    PinnedBaselineMissing,
    RepositoryGraphMissing,
    TargetedRecoveryMissing,
    TokenBudgetExceeded,
    TriggerIncorrect,
    WriterIsolationMissing,
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
    pub replay_correctness: f64,
    pub conformance_score: f64,
}

impl RunRecord {
    pub fn passed(&self) -> bool {
        let replay_passed = !self.result.delegated || self.replay_correctness >= 1.0;
        let conformance_passed = !self.lane.is_multi() || self.conformance_score >= 1.0;
        self.hard_failures.is_empty()
            && self.behavioral_correctness >= 1.0
            && replay_passed
            && conformance_passed
            && self.result.claimed_complete
            && self.result.error.is_none()
    }

    pub fn total_tokens(&self) -> u64 {
        self.result
            .usage
            .input_tokens
            .saturating_add(self.result.usage.output_tokens)
    }
}
