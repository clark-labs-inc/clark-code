use serde::{Deserialize, Serialize};

use crate::policy::{enforce_decision, HostDisposition, InvocationMode, Scenario};

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SentinelAction {
    Stop,
    DeferToHost,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatus {
    Done,
    Cancelled,
    VerificationIncomplete,
    StalledNoProgress,
    NotTerminal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SentinelVerdict {
    StopDone,
    StopCancelled,
    StopVerificationIncomplete,
    StopStalledNoProgress,
    DeferToHost,
}

impl SentinelVerdict {
    pub fn action(self) -> SentinelAction {
        match self {
            Self::DeferToHost => SentinelAction::DeferToHost,
            _ => SentinelAction::Stop,
        }
    }

    pub fn terminal_status(self) -> TerminalStatus {
        match self {
            Self::StopDone => TerminalStatus::Done,
            Self::StopCancelled => TerminalStatus::Cancelled,
            Self::StopVerificationIncomplete => TerminalStatus::VerificationIncomplete,
            Self::StopStalledNoProgress => TerminalStatus::StalledNoProgress,
            Self::DeferToHost => TerminalStatus::NotTerminal,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    TerminalAnswerNoPendingWork,
    UserCancellation,
    NonProgressAfterTerminalAnswer,
    StateCycleNoNovelty,
    VerificationBudgetExhausted,
    ProductiveStateDelta,
    ExplorationNovelty,
    BoundedRecoveryAvailable,
    InsufficientEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SentinelDecision {
    pub decision: SentinelVerdict,
    pub reason_code: ReasonCode,
    pub confidence: Confidence,
    pub evidence_event_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallReceipt {
    pub duration_ms: u128,
    pub timed_out: bool,
    pub http_status: Option<u16>,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub generation_id: Option<String>,
    pub finish_reason: Option<String>,
    pub choice_count: usize,
    pub tool_call_count: usize,
    pub assistant_content_present: bool,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub provider_cost_usd: f64,
    pub decision: Option<SentinelDecision>,
    pub strict_payload: bool,
    pub route_valid: bool,
    pub one_shot: bool,
    pub errors: Vec<String>,
}

impl CallReceipt {
    pub fn infrastructure_ok(&self) -> bool {
        !self.timed_out
            && self.errors.is_empty()
            && self.strict_payload
            && self.route_valid
            && self.one_shot
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TrialReceipt {
    pub scenario: &'static str,
    pub source: &'static str,
    pub repetition: usize,
    pub invocation: InvocationMode,
    pub expected_host_disposition: HostDisposition,
    pub observed_host_disposition: HostDisposition,
    pub stop_required: bool,
    pub expected_action: Option<SentinelAction>,
    pub allowed_terminal_statuses: Vec<TerminalStatus>,
    pub call: Option<CallReceipt>,
    pub raw_decision_correct: bool,
    pub effective_action: Option<SentinelAction>,
    pub stop_accepted: bool,
    pub stop_rejection_reason: Option<&'static str>,
    pub raw_false_stop: bool,
    pub enforced_false_stop: bool,
    pub passed: bool,
}

impl TrialReceipt {
    pub fn from_call(scenario: Scenario, repetition: usize, call: CallReceipt) -> Self {
        let decision = call.decision.as_ref();
        let enforcement = decision.map(|decision| enforce_decision(&scenario.packet, decision));
        let raw_decision_correct = decision.is_some_and(|decision| {
            Some(decision.decision.action()) == scenario.expected_action
                && scenario
                    .allowed_terminal_statuses
                    .contains(&decision.decision.terminal_status())
        });
        let raw_false_stop = scenario.expected_action == Some(SentinelAction::DeferToHost)
            && decision.is_some_and(|decision| decision.decision.action() == SentinelAction::Stop);
        let effective_action = enforcement.map(|enforcement| enforcement.effective_action);
        let enforced_false_stop = scenario.expected_action == Some(SentinelAction::DeferToHost)
            && effective_action == Some(SentinelAction::Stop);
        let passed = call.infrastructure_ok()
            && decision.is_some_and(|decision| {
                effective_action == scenario.expected_action
                    && (effective_action != Some(SentinelAction::Stop)
                        || scenario
                            .allowed_terminal_statuses
                            .contains(&decision.decision.terminal_status()))
                    && decision
                        .evidence_event_ids
                        .iter()
                        .all(|id| scenario.packet.has_event(id))
                    && !decision.evidence_event_ids.is_empty()
            });
        Self {
            scenario: scenario.id,
            source: scenario.source,
            repetition,
            invocation: scenario.invocation,
            expected_host_disposition: scenario.expected_host_disposition,
            observed_host_disposition: crate::policy::host_disposition(&scenario.packet),
            stop_required: scenario.expected_action == Some(SentinelAction::Stop),
            expected_action: scenario.expected_action,
            allowed_terminal_statuses: scenario.allowed_terminal_statuses,
            call: Some(call),
            raw_decision_correct,
            effective_action,
            stop_accepted: enforcement.is_some_and(|enforcement| enforcement.stop_accepted),
            stop_rejection_reason: enforcement.and_then(|enforcement| enforcement.rejection_reason),
            raw_false_stop,
            enforced_false_stop,
            passed,
        }
    }

    pub fn host_only(scenario: Scenario, repetition: usize) -> Self {
        let observed = crate::policy::host_disposition(&scenario.packet);
        Self {
            scenario: scenario.id,
            source: scenario.source,
            repetition,
            invocation: scenario.invocation,
            expected_host_disposition: scenario.expected_host_disposition,
            observed_host_disposition: observed,
            stop_required: false,
            expected_action: None,
            allowed_terminal_statuses: scenario.allowed_terminal_statuses,
            call: None,
            raw_decision_correct: true,
            effective_action: None,
            stop_accepted: false,
            stop_rejection_reason: None,
            raw_false_stop: false,
            enforced_false_stop: false,
            passed: observed == scenario.expected_host_disposition,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MatrixSummary {
    pub trials: usize,
    pub model_calls: usize,
    pub runtime_sentinel_calls: usize,
    pub shadow_control_calls: usize,
    pub deterministic_host_stops: usize,
    pub passed: usize,
    pub stop_required_calls: usize,
    pub correct_stops: usize,
    pub stop_recall: f64,
    pub defer_required_calls: usize,
    pub raw_decisions_correct: usize,
    pub raw_decision_accuracy: f64,
    pub raw_false_stops: usize,
    pub raw_false_stop_rate: f64,
    pub enforced_false_stops: usize,
    pub enforced_false_stop_rate: f64,
    pub strict_payload_rate: f64,
    pub one_shot_rate: f64,
    pub route_valid_rate: f64,
    pub timeout_count: usize,
    pub average_latency_ms: u128,
    pub max_latency_ms: u128,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub provider_cost_usd: f64,
    pub gate_passed: bool,
}

pub fn summarize(trials: &[TrialReceipt]) -> MatrixSummary {
    let calls = trials
        .iter()
        .filter_map(|trial| trial.call.as_ref())
        .collect::<Vec<_>>();
    let model_calls = calls.len();
    let denominator = model_calls.max(1) as f64;
    let stop_required = trials
        .iter()
        .filter(|trial| trial.call.is_some() && trial.stop_required)
        .collect::<Vec<_>>();
    let defer_required = trials
        .iter()
        .filter(|trial| trial.call.is_some() && !trial.stop_required)
        .collect::<Vec<_>>();
    let correct_stops = stop_required.iter().filter(|trial| trial.passed).count();
    let raw_decisions_correct = trials
        .iter()
        .filter(|trial| trial.call.is_some() && trial.raw_decision_correct)
        .count();
    let raw_false_stops = defer_required
        .iter()
        .filter(|trial| trial.raw_false_stop)
        .count();
    let enforced_false_stops = defer_required
        .iter()
        .filter(|trial| trial.enforced_false_stop)
        .count();
    let passed = trials.iter().filter(|trial| trial.passed).count();
    let average_latency_ms =
        calls.iter().map(|call| call.duration_ms).sum::<u128>() / model_calls.max(1) as u128;
    let stop_recall = correct_stops as f64 / stop_required.len().max(1) as f64;
    let raw_decision_accuracy = raw_decisions_correct as f64 / model_calls.max(1) as f64;
    let raw_false_stop_rate = raw_false_stops as f64 / defer_required.len().max(1) as f64;
    let enforced_false_stop_rate = enforced_false_stops as f64 / defer_required.len().max(1) as f64;
    let rate = |predicate: fn(&CallReceipt) -> bool| {
        calls.iter().filter(|call| predicate(call)).count() as f64 / denominator
    };
    let strict_payload_rate = rate(|call| call.strict_payload);
    let one_shot_rate = rate(|call| call.one_shot);
    let route_valid_rate = rate(|call| call.route_valid);
    MatrixSummary {
        trials: trials.len(),
        model_calls,
        runtime_sentinel_calls: trials
            .iter()
            .filter(|trial| trial.invocation == InvocationMode::RuntimeSentinel)
            .count(),
        shadow_control_calls: trials
            .iter()
            .filter(|trial| trial.invocation == InvocationMode::ShadowControl)
            .count(),
        deterministic_host_stops: trials
            .iter()
            .filter(|trial| trial.invocation == InvocationMode::HostBypass)
            .count(),
        passed,
        stop_required_calls: stop_required.len(),
        correct_stops,
        stop_recall,
        defer_required_calls: defer_required.len(),
        raw_decisions_correct,
        raw_decision_accuracy,
        raw_false_stops,
        raw_false_stop_rate,
        enforced_false_stops,
        enforced_false_stop_rate,
        strict_payload_rate,
        one_shot_rate,
        route_valid_rate,
        timeout_count: calls.iter().filter(|call| call.timed_out).count(),
        average_latency_ms,
        max_latency_ms: calls.iter().map(|call| call.duration_ms).max().unwrap_or(0),
        input_tokens: calls.iter().map(|call| call.input_tokens).sum(),
        output_tokens: calls.iter().map(|call| call.output_tokens).sum(),
        provider_cost_usd: calls.iter().map(|call| call.provider_cost_usd).sum(),
        gate_passed: !calls.is_empty()
            && passed == trials.len()
            && stop_recall == 1.0
            && raw_decision_accuracy >= 0.75
            && enforced_false_stop_rate == 0.0
            && strict_payload_rate == 1.0
            && one_shot_rate == 1.0
            && route_valid_rate == 1.0,
    }
}

#[derive(Debug, Serialize)]
pub struct MatrixReceipt {
    pub schema_version: u32,
    pub evidence_class: &'static str,
    pub design: &'static str,
    pub requested_model: &'static str,
    pub reasoning_effort: &'static str,
    pub route: RouteReceipt,
    pub repetitions_requested: usize,
    pub concurrency: usize,
    pub source_commit: Option<String>,
    pub source_dirty: bool,
    pub trials: Vec<TrialReceipt>,
    pub summary: MatrixSummary,
    pub started_at_ms: u128,
    pub finished_at_ms: u128,
}
