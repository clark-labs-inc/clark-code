use agent_core::domain::AgentEvent;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub type SeedFn = fn(&Path) -> Result<(), String>;
pub type VerifyFn = fn(&Path) -> Verification;
pub type ReferenceFn = fn(&Path) -> Result<(), String>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Project,
    Org,
    Scout,
    Oracle,
    Noise,
    Stale,
    Conflict,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    Required,
    Useful,
    Distractor,
    Stale,
    Conflict,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Evidence {
    pub id: &'static str,
    pub source: EvidenceSource,
    pub role: EvidenceRole,
    pub text: &'static str,
}

#[allow(dead_code)] // Legacy fixture hints remain readable but are not used by the LLM judge.
pub struct Scenario {
    pub id: &'static str,
    pub task: &'static str,
    pub required_plan_terms: &'static [&'static str],
    pub semantic_plan_checks: &'static [SemanticPlanCheck],
    pub required_evidence: &'static [&'static str],
    pub forbidden_evidence: &'static [&'static str],
    pub oracle_plan: &'static str,
    pub evidence: Vec<Evidence>,
    pub seed: SeedFn,
    pub verify: VerifyFn,
    pub reference_apply: ReferenceFn,
}

impl Scenario {
    pub fn domain(&self) -> &'static str {
        match self.id {
            "regional-audit-export" | "artifact-retention-legal-hold" => "compliance-data",
            "rolling-event-v2"
            | "tenant-policy-cache-invalidation"
            | "notification-template-versioning" => "distributed-systems",
            "permission-collaboration-split" | "feature-flag-tenant-scope" => {
                "product-configuration"
            }
            "oauth-key-rotation" => "security",
            "payment-webhook-idempotency" => "payments",
            "search-index-zero-downtime" | "database-shard-rebalance" => "data-platform",
            "mobile-offline-sync-v3" => "client-sync",
            _ => "unclassified",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // Keyword lists are retained only to deserialize/review older fixture design.
pub struct SemanticPlanCheck {
    pub id: &'static str,
    pub required_all: &'static [&'static str],
    pub required_any: &'static [&'static str],
    pub expectation: &'static str,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Check {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Verification {
    pub checks: Vec<Check>,
}

impl Verification {
    pub fn push(&mut self, id: &str, passed: bool, detail: impl Into<String>) {
        self.checks.push(Check {
            id: id.to_string(),
            passed,
            detail: detail.into(),
        });
    }

    pub fn passed(&self) -> usize {
        self.checks.iter().filter(|check| check.passed).count()
    }

    pub fn total(&self) -> usize {
        self.checks.len()
    }

    pub fn score(&self) -> f64 {
        if self.checks.is_empty() {
            0.0
        } else {
            self.passed() as f64 / self.total() as f64
        }
    }

    pub fn first_failure(&self) -> Option<&Check> {
        self.checks.iter().find(|check| !check.passed)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Lane {
    pub id: String,
    #[serde(default)]
    pub knowledge_delivery: KnowledgeDelivery,
    pub planner_sources: Vec<EvidenceSource>,
    pub executor_sources: Vec<EvidenceSource>,
    pub plan_origin: PlanOrigin,
    pub run_planner: bool,
    pub pass_plan_to_executor: bool,
    pub handoff: HandoffMode,
}

impl Lane {
    pub fn knowledge_delivery(&self) -> KnowledgeDelivery {
        self.knowledge_delivery
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeDelivery {
    #[default]
    ForcedPreflight,
    DeferredDiscovery,
    PreactivatedTools,
    PrefetchedCapsule,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanOrigin {
    #[default]
    None,
    Generated,
    Oracle,
    BankNone,
    BankAll,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffMode {
    #[default]
    None,
    MarkdownFresh,
    TypedCurrent,
    TypedFresh,
    TypedReplayFresh,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct HandoffReceipt {
    pub mode: HandoffMode,
    pub plan_bank_id: Option<String>,
    pub plan_id: Option<String>,
    pub plan_revision: Option<u32>,
    /// Hash of the full proposal stored in typed plan state.
    pub plan_sha256: Option<String>,
    /// Hash and size of the proposal bytes actually delivered to execution.
    pub delivered_plan_sha256: Option<String>,
    pub source_plan_chars: Option<usize>,
    pub delivered_plan_chars: Option<usize>,
    pub delivery_truncated: bool,
    pub typed_decision_sent: bool,
    pub executor_reused_provider: bool,
    pub executor_reused_session: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TrajectoryEventReceipt {
    pub stream_sequence: usize,
    pub elapsed_ms: u128,
    pub event: AgentEvent,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TrajectoryReceipt {
    pub events: Vec<TrajectoryEventReceipt>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RetrievalTreatmentReceipt {
    pub applicable: bool,
    #[serde(default)]
    pub knowledge_delivery: KnowledgeDelivery,
    pub offered_sources: Vec<EvidenceSource>,
    pub successful_sources: Vec<EvidenceSource>,
    pub missing_sources: Vec<EvidenceSource>,
    pub compliant: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UsageReceipt {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub context_tokens: u64,
    pub cost_usd: f64,
    pub elapsed_ms: u128,
    pub tool_calls: usize,
    pub tools: Vec<String>,
    pub turns: usize,
    pub timed_out: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteReceipt {
    pub requested_model: String,
    pub effective_model: String,
    pub product_route: String,
    pub free_tier_verified: bool,
    pub verification_method: String,
    pub catalog_tier_id: Option<String>,
    pub catalog_model_option_id: Option<String>,
    pub catalog_label: Option<String>,
    pub probe_input_tokens: u64,
    pub probe_output_tokens: u64,
    pub probe_upstream_cost_usd: f64,
    pub probe_retries: Vec<RetryReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextReceipt {
    /// Evidence assigned to this treatment, whether or not the model received it.
    #[serde(default)]
    pub assigned_evidence_ids: Vec<String>,
    /// Evidence bytes actually placed in the model prompt.
    ///
    /// Schema-v4 deferred-discovery artifacts incorrectly stored assigned IDs
    /// here. Judge packet export repairs that legacy representation from the
    /// context hash and delivery receipt without mutating the source artifact.
    #[serde(default)]
    pub injected_evidence_ids: Vec<String>,
    #[serde(default)]
    pub injected_context: String,
    pub context_sha256: String,
    #[serde(default)]
    pub retrievals: Vec<RetrievalReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RetrievalReceipt {
    pub source: String,
    pub operation: String,
    pub query: Option<String>,
    #[serde(default)]
    pub request_method: String,
    #[serde(default)]
    pub request_target: String,
    #[serde(default)]
    pub request_body: String,
    #[serde(default)]
    pub request_sha256: String,
    #[serde(default)]
    pub response_status: u16,
    #[serde(default)]
    pub response_body: String,
    #[serde(default)]
    pub response_sha256: String,
    pub returned_evidence_ids: Vec<String>,
    pub status: String,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RetryReceipt {
    pub scope: String,
    pub attempt: usize,
    pub status: String,
    pub reason: String,
    pub requested_wait_ms: u64,
    pub actual_wait_ms: u128,
    pub model_output_observed: bool,
    pub workspace_mutated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CaseRecord {
    pub schema_version: u32,
    pub run_id: String,
    pub mode: String,
    pub scenario: String,
    pub lane: String,
    pub repetition: usize,
    pub profile: String,
    pub route: RouteReceipt,
    pub fixture_sha256: String,
    #[serde(default)]
    pub planning_contract: String,
    pub planning_prompt_sha256: String,
    #[serde(default)]
    pub task_prompt: String,
    pub task_prompt_sha256: String,
    #[serde(default)]
    pub executor_prompt: String,
    pub executor_prompt_sha256: String,
    #[serde(default)]
    pub handoff: HandoffReceipt,
    pub planner_context: ContextReceipt,
    pub executor_context: ContextReceipt,
    pub plan: Option<String>,
    #[serde(default)]
    pub retrieval_treatment: RetrievalTreatmentReceipt,
    pub planner_usage: UsageReceipt,
    pub executor_usage: UsageReceipt,
    #[serde(default)]
    pub planner_trajectory: TrajectoryReceipt,
    #[serde(default)]
    pub executor_trajectory: TrajectoryReceipt,
    pub verification: Verification,
    pub executor_tree_sha256: String,
    pub executor_files: BTreeMap<String, String>,
    #[serde(default)]
    pub retries: Vec<RetryReceipt>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LaneSummary {
    pub lane: String,
    pub cases: usize,
    pub mean_hidden_check_score: f64,
    pub retrieval_compliance_rate: Option<f64>,
    pub hidden_check_full_success_rate: f64,
    pub mean_total_tokens: f64,
    pub mean_latency_ms: f64,
    pub total_cost_usd: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PairedEffect {
    pub control: String,
    pub candidate: String,
    pub pairs: usize,
    pub mean_executor_delta: f64,
    pub ci95_low: f64,
    pub ci95_high: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Summary {
    pub schema_version: u32,
    pub run_id: String,
    pub mode: String,
    pub route: RouteReceipt,
    pub prompt_profile: String,
    pub repetitions: usize,
    pub lane_summaries: Vec<LaneSummary>,
    pub paired_effects: Vec<PairedEffect>,
    pub first_failures: BTreeMap<String, String>,
    pub plan_bank_entries: usize,
    pub plan_bank_planner_tokens: u64,
    pub plan_bank_provider_reported_upstream_cost_usd: f64,
    pub total_provider_reported_upstream_cost_usd: f64,
}
