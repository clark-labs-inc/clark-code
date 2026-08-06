use std::collections::BTreeMap;

use agent_core::domain::{AgentEvent, RunOutcome, RunUsage};
use serde::Serialize;
use serde_json::Value;
use url::Url;

#[derive(Serialize)]
pub(super) struct Receipt {
    pub(super) schema_version: u32,
    pub(super) evidence_class: &'static str,
    pub(super) requested_model: &'static str,
    pub(super) base_url: String,
    pub(super) started_at_ms: u128,
    pub(super) finished_at_ms: u128,
    pub(super) source_commit: Option<String>,
    pub(super) source_dirty: bool,
    pub(super) repetitions_requested: usize,
    pub(super) repetitions_completed: usize,
    pub(super) concurrency: usize,
    pub(super) max_provider_cost_usd: f64,
    pub(super) trajectories: Vec<TrajectoryReceipt>,
    pub(super) summary: Summary,
}

#[derive(Serialize)]
pub(super) struct TrajectoryReceipt {
    pub(super) repetition: usize,
    pub(super) workspace: String,
    pub(super) error: Option<String>,
    pub(super) cases: Vec<CaseReceipt>,
}

#[derive(Serialize)]
pub(super) struct CaseReceipt {
    pub(super) id: &'static str,
    pub(super) repetition: usize,
    pub(super) verdict: &'static str,
    pub(super) passed: bool,
    pub(super) infrastructure_failure: bool,
    pub(super) route_valid: bool,
    pub(super) duration_ms: u128,
    pub(super) outcome: Option<RunOutcome>,
    pub(super) usage: Option<RunUsage>,
    pub(super) text: String,
    pub(super) tools: Vec<String>,
    pub(super) goal_completed: bool,
    pub(super) event_counts: BTreeMap<String, usize>,
    pub(super) model_responses: Vec<Value>,
    pub(super) errors: Vec<String>,
    pub(super) oracle_failures: Vec<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct Summary {
    pub(super) cases: usize,
    pub(super) passed: usize,
    pub(super) quality_failures: usize,
    pub(super) runtime_failures: usize,
    pub(super) infrastructure_failures: usize,
    pub(super) route_violations: usize,
    pub(super) cancellation_failures: usize,
    pub(super) provider_cost_usd: f64,
    pub(super) pass_rate: f64,
    pub(super) by_scenario: BTreeMap<String, ScenarioSummary>,
    pub(super) gate_passed: bool,
}

#[derive(Clone, Default, Serialize)]
pub(super) struct ScenarioSummary {
    pub(super) cases: usize,
    pub(super) passed: usize,
    pub(super) pass_rate: f64,
}

pub(super) fn sanitized_base_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        return "<invalid-or-redacted>".to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

pub(super) fn event_name(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::RunStarted { .. } => "run_started",
        AgentEvent::Checkpoint { .. } => "checkpoint",
        AgentEvent::MessageChunk { .. } => "message_chunk",
        AgentEvent::MessagePhase { .. } => "message_phase",
        AgentEvent::SpecialistPresentation { .. } => "specialist_presentation",
        AgentEvent::ToolCall { .. } => "tool_call",
        AgentEvent::ToolCallUpdate { .. } => "tool_call_update",
        AgentEvent::ExecutionChecklistUpdated { .. } => "execution_checklist_updated",
        AgentEvent::RunUsageUpdated { .. } => "run_usage_updated",
        AgentEvent::ContextCompacted { .. } => "context_compacted",
        AgentEvent::ProposedPlanUpdated { .. } => "proposed_plan_updated",
        AgentEvent::GoalUpdated { .. } => "goal_updated",
        AgentEvent::FanOut { .. } => "fan_out",
        AgentEvent::PermissionRequest { .. } => "permission_request",
        AgentEvent::Artifact { .. } => "artifact",
        AgentEvent::Surface { .. } => "surface",
        AgentEvent::ProviderIncidentUpdated { .. } => "provider_incident_updated",
        AgentEvent::ModeChanged { .. } => "mode_changed",
        AgentEvent::Trace { .. } => "trace",
        AgentEvent::Error { .. } => "error",
        AgentEvent::RunFinished { .. } => "run_finished",
    }
}

pub(super) fn summarize(trajectories: &[TrajectoryReceipt]) -> Summary {
    let cases = trajectories
        .iter()
        .flat_map(|trajectory| trajectory.cases.iter())
        .collect::<Vec<_>>();
    let mut by_scenario: BTreeMap<String, ScenarioSummary> = BTreeMap::new();
    for case in &cases {
        let scenario = by_scenario.entry(case.id.to_string()).or_default();
        scenario.cases += 1;
        scenario.passed += usize::from(case.passed);
    }
    for scenario in by_scenario.values_mut() {
        scenario.pass_rate = scenario.passed as f64 / scenario.cases.max(1) as f64;
    }
    let passed = cases.iter().filter(|case| case.passed).count();
    let infrastructure_failures = cases
        .iter()
        .filter(|case| case.infrastructure_failure)
        .count();
    let quality_failures = cases
        .iter()
        .filter(|case| case.verdict == "quality_failure")
        .count();
    let runtime_failures = cases
        .iter()
        .filter(|case| case.verdict == "runtime_failure")
        .count();
    let route_violations = cases.iter().filter(|case| !case.route_valid).count();
    let cancellation_failures = cases
        .iter()
        .filter(|case| case.id == "cancel" && !case.passed)
        .count();
    let provider_cost_usd = cases
        .iter()
        .filter_map(|case| case.usage.and_then(|usage| usage.cost_usd))
        .sum();
    let pass_rate = passed as f64 / cases.len().max(1) as f64;
    let trajectory_errors = trajectories
        .iter()
        .any(|trajectory| trajectory.error.is_some());
    let gate_passed = !trajectory_errors
        && !cases.is_empty()
        && pass_rate >= 0.90
        && by_scenario
            .values()
            .all(|scenario| scenario.pass_rate >= 0.90)
        && infrastructure_failures == 0
        && route_violations == 0
        && cancellation_failures == 0;
    Summary {
        cases: cases.len(),
        passed,
        quality_failures,
        runtime_failures,
        infrastructure_failures,
        route_violations,
        cancellation_failures,
        provider_cost_usd,
        pass_rate,
        by_scenario,
        gate_passed,
    }
}
