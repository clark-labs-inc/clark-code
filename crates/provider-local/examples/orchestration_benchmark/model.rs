use std::collections::{BTreeMap, BTreeSet};

use agent_core::domain::{RunExecutionSummary, RunUsage};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    Scripted,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionCeiling {
    ReadOnly,
    WorkspaceWrite,
    Full,
}

impl PermissionCeiling {
    pub fn permits(self, requested: Self) -> bool {
        requested <= self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    QueueOnly,
    TriggerTurn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskMode {
    ReadOnly,
    Write,
    Review,
    Verify,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Proposed,
    Ready,
    Leased,
    Running,
    Reported,
    Review,
    Rework,
    Verified,
    Accepted,
    Failed,
    Blocked,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Pending,
    Running,
    Idle,
    Completed,
    Interrupted,
    Errored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneKind {
    Single,
    PlannedSingle,
    ReaderWriter,
    Reviewed,
    CheapSubagents,
    HomogeneousStrong,
    ClarkCloud,
    MixedHarness,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSpec {
    pub id: String,
    pub kind: LaneKind,
    pub root_model: String,
    pub subagent_model: Option<String>,
    pub provider: String,
    pub reasoning_effort: Option<String>,
    pub max_concurrency: usize,
    pub max_attempts: u32,
    pub token_budget: u64,
    pub reviewer: bool,
    pub verifier: bool,
    pub cloud_agents: bool,
}

impl LaneSpec {
    pub fn catalog(strong: &str, cheap: &str) -> Vec<Self> {
        let lane = |id: &str, kind, subagent: Option<&str>, provider: &str| Self {
            id: id.to_string(),
            kind,
            root_model: strong.to_string(),
            subagent_model: subagent.map(str::to_string),
            provider: provider.to_string(),
            reasoning_effort: None,
            max_concurrency: 4,
            max_attempts: 2,
            token_budget: 120_000,
            reviewer: matches!(kind, LaneKind::Reviewed),
            verifier: matches!(kind, LaneKind::Reviewed),
            cloud_agents: matches!(kind, LaneKind::ClarkCloud),
        };
        vec![
            lane("single", LaneKind::Single, None, "local"),
            lane("planned-single", LaneKind::PlannedSingle, None, "local"),
            lane(
                "reader-writer",
                LaneKind::ReaderWriter,
                Some(strong),
                "local",
            ),
            lane("reviewed", LaneKind::Reviewed, Some(strong), "local"),
            lane(
                "cheap-subagents",
                LaneKind::CheapSubagents,
                Some(cheap),
                "local",
            ),
            lane(
                "homogeneous-strong",
                LaneKind::HomogeneousStrong,
                Some(strong),
                "local",
            ),
            lane(
                "clark-cloud",
                LaneKind::ClarkCloud,
                Some(cheap),
                "clark-cloud",
            ),
            lane(
                "mixed-harness",
                LaneKind::MixedHarness,
                Some(cheap),
                "mixed",
            ),
        ]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskContract {
    pub id: String,
    pub logical_path: String,
    pub mode: TaskMode,
    pub instruction: String,
    pub dependencies: Vec<String>,
    pub scope: BTreeSet<String>,
    pub acceptance: Vec<String>,
    pub permission_ceiling: PermissionCeiling,
    pub preferred_model_tier: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandEvidence {
    pub command: String,
    pub exit_code: Option<i32>,
    pub output_artifact: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestEvidence {
    pub name: String,
    pub passed: bool,
    pub output_artifact: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimEvidence {
    pub claim: String,
    pub evidence_ref: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructuredHandoff {
    pub task_id: String,
    pub attempt_id: String,
    pub reported_status: TaskStatus,
    pub summary: String,
    pub changed_paths: BTreeSet<String>,
    pub baseline_checkpoint: Option<String>,
    pub result_checkpoint: Option<String>,
    pub commands: Vec<CommandEvidence>,
    pub tests: Vec<TestEvidence>,
    pub claims: Vec<ClaimEvidence>,
    pub unresolved: Vec<String>,
    pub artifact_refs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: String,
    pub path: Option<String>,
    pub message: String,
    pub evidence_ref: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewVerdict {
    pub task_id: String,
    pub accepted: bool,
    pub findings: Vec<ReviewFinding>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub attempt_id: String,
    pub task_id: String,
    pub agent_path: String,
    pub provider: String,
    pub model: String,
    pub role: String,
    pub permission_ceiling: PermissionCeiling,
    pub status: AgentStatus,
    pub duration_ms: u64,
    pub usage: RunUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<RunExecutionSummary>,
    #[serde(default)]
    pub lifecycle_trace_replayable: bool,
    #[serde(default)]
    pub duplicate_tool_receipts: u32,
    pub tool_calls: Vec<String>,
    pub final_message: String,
    pub handoff: Option<StructuredHandoff>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardFailure {
    AcceptedUnverifiableResult,
    CausalTraceMissing,
    ConcurrentWriterLease,
    DestructiveBehavior,
    DuplicateToolReceipt,
    LifecycleTraceInvalid,
    LostUserChange,
    OutOfScopeWrite,
    PermissionWidening,
    UnauthorizedWrite,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TriggerMetrics {
    pub expected_delegate: bool,
    pub actual_delegate: bool,
    pub false_positive: bool,
    pub false_negative: bool,
    pub boundary_score: f64,
    pub dependency_score: f64,
    pub cheap_model_assignment_score: f64,
    pub cloud_agent_assignment_score: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunMetrics {
    pub correctness: f64,
    pub changed_path_precision: f64,
    pub recovered_failures: u32,
    pub unrecovered_failures: u32,
    pub review_catches: u32,
    pub review_false_vetoes: u32,
    pub interventions: u32,
    pub duration_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub non_cached_input_tokens: u64,
    pub non_cached_input_available: bool,
    pub cost_usd: f64,
    pub agent_millis: u64,
    pub redundant_reads: u32,
    pub root_executions: u32,
    pub root_attempts: u32,
    pub root_recoveries: u32,
    pub lifecycle_trace_failures: u32,
    pub duplicate_tool_receipts: u32,
    pub cloud_agent_calls: u32,
    pub unmetered_external_calls: u32,
    pub max_parallel_agents: usize,
    pub utilization: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchmarkRecord {
    pub schema_version: u32,
    pub run_id: String,
    pub evidence_level: EvidenceLevel,
    pub scenario_id: String,
    pub scenario_family: String,
    pub variant: u32,
    pub repetition: u32,
    pub lane: LaneSpec,
    pub repository_path: String,
    pub baseline_checkpoint: Option<String>,
    pub result_checkpoint: Option<String>,
    pub started_at_unix_ms: i64,
    pub tasks: Vec<TaskContract>,
    pub task_statuses: BTreeMap<String, TaskStatus>,
    pub attempts: Vec<AttemptRecord>,
    pub handoffs: Vec<StructuredHandoff>,
    pub reviews: Vec<ReviewVerdict>,
    pub actual_changed_paths: BTreeSet<String>,
    pub checks: Vec<CheckResult>,
    pub hard_failures: BTreeSet<HardFailure>,
    pub trigger: TriggerMetrics,
    pub metrics: RunMetrics,
    pub orchestration_complete: bool,
    pub error: Option<String>,
}

impl BenchmarkRecord {
    pub fn passed(&self) -> bool {
        self.hard_failures.is_empty()
            && self.metrics.correctness >= 1.0
            && self.orchestration_complete
            && self.error.is_none()
    }
}
