use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{BudgetSnapshot, HarnessKind, TaskId, UsageCharge};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryId(pub(super) String);

impl RepositoryId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.len() > 64 {
            return Err("repository ids must contain 1 to 64 characters".to_string());
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        }) {
            return Err(
                "repository ids may contain lowercase letters, digits, _ and -".to_string(),
            );
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RepositoryId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutKind {
    Main,
    LinkedWorktree,
    DetachedWorktree,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryBaseline {
    pub repository_id: RepositoryId,
    pub repository_fingerprint: String,
    pub checkout_root: String,
    pub checkout_kind: CheckoutKind,
    pub head_oid: String,
    pub current_branch: Option<String>,
    pub dirty_tree_sha256: String,
    pub allowed_changed_paths: BTreeSet<String>,
    pub cloud_eligible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryContractEdge {
    pub id: String,
    pub producer: RepositoryId,
    pub consumers: BTreeSet<RepositoryId>,
    pub artifact: String,
    pub compatibility_rule: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractDecision {
    pub edge_id: String,
    pub decided_by: TaskId,
    pub artifact_sha256: String,
    pub compatibility_rule: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiRepoTaskRole {
    Planner,
    Reader,
    Writer,
    Reviewer,
    Integrator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Cheap,
    Strong,
    Reviewer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiRepoTask {
    pub id: TaskId,
    pub role: MultiRepoTaskRole,
    pub repository_id: Option<RepositoryId>,
    pub dependencies: BTreeSet<TaskId>,
    pub objective: String,
    pub harness: String,
    pub harness_kind: HarnessKind,
    pub model: String,
    pub model_tier: ModelTier,
    pub budget_reservation: u64,
    pub allowed_changed_paths: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiRepoPlan {
    pub repositories: BTreeMap<RepositoryId, RepositoryBaseline>,
    pub contracts: Vec<RepositoryContractEdge>,
    pub contract_decisions: Vec<ContractDecision>,
    pub tasks: Vec<MultiRepoTask>,
    pub integration_checks: Vec<IntegrationCheck>,
    pub max_parallel_writers: usize,
    pub requires_independent_review: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecompositionDecision {
    pub delegated: bool,
    pub parallel_writer_batches: Vec<Vec<TaskId>>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationKind {
    LocalEphemeralClone,
    DetachedWorktree,
    CloudEphemeralClone,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangePackageDescriptor {
    pub task_id: TaskId,
    pub repository_id: RepositoryId,
    pub base_head_oid: String,
    pub changed_paths: BTreeSet<String>,
    pub patch_sha256: String,
    pub result_tree_sha256: String,
    pub artifact_path: String,
    pub isolation: IsolationKind,
    pub checks_run: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReaderReport {
    pub task_id: TaskId,
    pub repository_id: RepositoryId,
    pub evidence_refs: Vec<String>,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Accept,
    Rework,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewReceipt {
    pub reviewer_task_id: TaskId,
    pub package_sha256: BTreeSet<String>,
    pub findings: Vec<String>,
    pub decision: ReviewDecision,
    pub rework_task_ids: BTreeSet<TaskId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReceipt {
    pub failed_task_id: TaskId,
    pub replacement_task_id: TaskId,
    pub preserved_package_sha256: BTreeSet<String>,
    pub reused_artifact_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationCheck {
    pub id: String,
    pub repository_id: RepositoryId,
    pub argv: Vec<String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationCheckReceipt {
    pub id: String,
    pub repository_id: RepositoryId,
    pub argv: Vec<String>,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub exit_code: Option<i32>,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub passed: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationReceipt {
    pub fresh_workspace: bool,
    pub repository_baselines: BTreeMap<RepositoryId, String>,
    pub repository_result_trees: BTreeMap<RepositoryId, String>,
    pub applied_patch_sha256: Vec<String>,
    pub checks_run: Vec<String>,
    pub check_receipts: Vec<IntegrationCheckReceipt>,
    pub passed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRunOutcome {
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskExecutionReceipt {
    pub task_id: TaskId,
    pub role: MultiRepoTaskRole,
    pub repository_id: Option<RepositoryId>,
    pub harness: String,
    pub model: String,
    pub model_tier: ModelTier,
    pub attempt: u32,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub outcome: TaskRunOutcome,
    pub usage: UsageCharge,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningReceipt {
    pub planner_task_id: TaskId,
    pub plan_sha256: String,
    pub repository_baselines: BTreeMap<RepositoryId, String>,
    pub delegated: bool,
    pub validated_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultiRepoRunResult {
    pub decomposition: DecompositionDecision,
    pub planning: PlanningReceipt,
    pub tasks: Vec<TaskExecutionReceipt>,
    pub reader_reports: Vec<ReaderReport>,
    pub change_packages: Vec<ChangePackageDescriptor>,
    pub recoveries: Vec<RecoveryReceipt>,
    pub review: Option<ReviewReceipt>,
    pub integration: Option<IntegrationReceipt>,
    pub budget: BudgetSnapshot,
    pub error: Option<String>,
}

impl MultiRepoRunResult {
    pub fn passed(&self) -> bool {
        self.decomposition.delegated
            && self.error.is_none()
            && self
                .review
                .as_ref()
                .is_none_or(|review| review.decision == ReviewDecision::Accept)
            && self
                .integration
                .as_ref()
                .is_some_and(|integration| integration.fresh_workspace && integration.passed)
    }
}
