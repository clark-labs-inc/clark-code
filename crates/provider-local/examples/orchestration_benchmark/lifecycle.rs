use std::collections::BTreeSet;

use agent_core::domain::{AgentEvent, RunExecutionSummary, RunStatus, RunUsage};
use agent_core::ids::{RunId, SessionId};
use agent_orchestration::{
    ExecutionEvent, ExecutionEventKind, ExecutionId, ExecutionLedger, ExecutionPolicy,
    ExecutionSnapshot, ExecutionState, FailureClass, ToolExecutionStatus, UsageCharge,
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LifecycleRecoveryCase {
    pub case: &'static str,
    pub baseline_correctness: f64,
    pub lifecycle_correctness: f64,
    pub recovery_expected: bool,
    pub recovery_allowed: bool,
    pub attempts: u32,
    pub recoveries: u32,
    pub weighted_tokens: f64,
    pub cost_usd: f64,
    pub trace_events: usize,
    pub trace_replayable: bool,
    pub duplicate_tool_receipts: u32,
    pub safety_passed: bool,
}

#[derive(Clone, Copy)]
enum Boundary {
    Clean,
    CompletedWrite,
    ActiveWrite,
    AwaitingPermission,
}

pub(crate) fn recovery_matrix() -> Vec<LifecycleRecoveryCase> {
    [
        ("clean_completion", Boundary::Clean, None, false),
        (
            "transient_after_completed_write",
            Boundary::CompletedWrite,
            Some(FailureClass::TransientTransport),
            true,
        ),
        (
            "rate_limit_at_clean_boundary",
            Boundary::Clean,
            Some(FailureClass::RateLimited),
            true,
        ),
        (
            "transient_during_write",
            Boundary::ActiveWrite,
            Some(FailureClass::TransientTransport),
            false,
        ),
        (
            "transient_while_permission_pending",
            Boundary::AwaitingPermission,
            Some(FailureClass::TransientTransport),
            false,
        ),
        (
            "non_transient_provider_failure",
            Boundary::Clean,
            Some(FailureClass::Provider),
            false,
        ),
    ]
    .into_iter()
    .map(|(name, boundary, failure, expected)| evaluate_case(name, boundary, failure, expected))
    .collect()
}

fn evaluate_case(
    case: &'static str,
    boundary: Boundary,
    failure: Option<FailureClass>,
    recovery_expected: bool,
) -> LifecycleRecoveryCase {
    let policy = ExecutionPolicy::default();
    let ledger = ExecutionLedger::new_root(
        ExecutionId::new(format!("benchmark-{case}")).unwrap(),
        policy,
    )
    .unwrap();
    ledger.start_attempt().unwrap();
    ledger
        .record_usage(UsageCharge {
            input_tokens: 10,
            output_tokens: 0,
            cost_usd: 0.001,
            ..Default::default()
        })
        .unwrap();
    if matches!(boundary, Boundary::CompletedWrite | Boundary::ActiveWrite) {
        ledger.tool_started("1:write", "write_file", true).unwrap();
    }
    if matches!(boundary, Boundary::CompletedWrite) {
        ledger
            .tool_finished(
                "1:write",
                ToolExecutionStatus::Completed,
                BTreeSet::from(["src/lib.rs".to_string()]),
            )
            .unwrap();
    }
    if matches!(boundary, Boundary::AwaitingPermission) {
        ledger
            .transition(ExecutionState::AwaitingInput, None)
            .unwrap();
    }

    let baseline_correctness = if failure.is_none() { 1.0 } else { 0.0 };
    let recovery_allowed = failure
        .map(|class| ledger.recovery_decision(class).allowed)
        .unwrap_or(false);
    if let Some(class) = failure {
        if recovery_allowed {
            ledger.schedule_recovery(class, "injected failure").unwrap();
            ledger.start_attempt().unwrap();
            ledger
                .record_usage(UsageCharge {
                    input_tokens: 5,
                    output_tokens: 2,
                    cost_usd: 0.0005,
                    ..Default::default()
                })
                .unwrap();
            complete(&ledger, ExecutionState::Completed);
        } else {
            if matches!(boundary, Boundary::ActiveWrite) {
                ledger
                    .tool_finished("1:write", ToolExecutionStatus::Cancelled, BTreeSet::new())
                    .unwrap();
            }
            ledger
                .transition(ExecutionState::Failed, Some("injected failure".into()))
                .unwrap();
        }
    } else {
        complete(&ledger, ExecutionState::Completed);
    }

    let events = ledger.events();
    let replay = ExecutionLedger::replay(&events);
    let snapshot = ledger.snapshot();
    let mut terminal_ids = BTreeSet::new();
    let duplicate_tool_receipts = events
        .iter()
        .filter_map(|event| match &event.kind {
            ExecutionEventKind::ToolFinished { id, .. } => Some(id),
            _ => None,
        })
        .filter(|id| !terminal_ids.insert((*id).clone()))
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let lifecycle_correctness = (snapshot.state == ExecutionState::Completed) as u8 as f64;
    let trace_replayable = replay.as_ref().is_ok_and(|value| value == &snapshot);
    LifecycleRecoveryCase {
        case,
        baseline_correctness,
        lifecycle_correctness,
        recovery_expected,
        recovery_allowed,
        attempts: snapshot.attempts.len() as u32,
        recoveries: snapshot.recoveries,
        weighted_tokens: snapshot.usage.weighted_tokens,
        cost_usd: snapshot.usage.cost_usd,
        trace_events: events.len(),
        trace_replayable,
        duplicate_tool_receipts,
        safety_passed: recovery_allowed == recovery_expected
            && trace_replayable
            && duplicate_tool_receipts == 0,
    }
}

fn complete(ledger: &ExecutionLedger, terminal: ExecutionState) {
    ledger.transition(ExecutionState::Verifying, None).unwrap();
    ledger
        .finalize_evidence(ledger.snapshot().evidence)
        .unwrap();
    ledger.transition(terminal, None).unwrap();
}

/// Deterministic provider fixture that drives the same lifecycle contract as
/// provider-local. Scripted benchmark results remain mechanics-only, but the
/// retained traces can now prove replay and duplicate-receipt invariants.
pub(crate) struct ScriptedLifecycle {
    ledger: ExecutionLedger,
    run: RunId,
}

impl ScriptedLifecycle {
    pub(crate) fn new(
        session: &SessionId,
        run: &RunId,
        events: &mut Vec<AgentEvent>,
    ) -> Result<Self, String> {
        let id = ExecutionId::new(format!("{}:{}", session.as_str(), run.as_str()))?;
        let ledger = ExecutionLedger::new_root(id, ExecutionPolicy::default())?;
        let lifecycle = Self {
            ledger,
            run: run.clone(),
        };
        lifecycle.emit(events, lifecycle.ledger.created_event());
        lifecycle.record(events, lifecycle.ledger.start_attempt())?;
        Ok(lifecycle)
    }

    pub(crate) fn tool_started(
        &self,
        events: &mut Vec<AgentEvent>,
        id: &str,
        name: &str,
        mutating: bool,
    ) -> Result<(), String> {
        self.record(
            events,
            self.ledger
                .tool_started(format!("1:{id}"), name.to_string(), mutating),
        )
    }

    pub(crate) fn tool_finished(
        &self,
        events: &mut Vec<AgentEvent>,
        id: &str,
        locations: BTreeSet<String>,
    ) -> Result<(), String> {
        self.record(
            events,
            self.ledger
                .tool_finished(format!("1:{id}"), ToolExecutionStatus::Completed, locations),
        )
    }

    pub(crate) fn finish_with_usage(
        &self,
        events: &mut Vec<AgentEvent>,
        status: RunStatus,
        usage: RunUsage,
    ) -> Result<RunExecutionSummary, String> {
        self.record(
            events,
            self.ledger.record_usage(UsageCharge {
                input_tokens: usage.input_tokens,
                cached_input_tokens: 0,
                output_tokens: usage.output_tokens,
                cost_usd: usage.cost_usd.unwrap_or(0.0),
            }),
        )?;
        self.finish(events, status, None)
    }

    pub(crate) fn finish(
        &self,
        events: &mut Vec<AgentEvent>,
        status: RunStatus,
        reason: Option<&str>,
    ) -> Result<RunExecutionSummary, String> {
        self.record(
            events,
            self.ledger.transition(ExecutionState::Verifying, None),
        )?;
        let receipt = self.ledger.snapshot().evidence;
        self.record(events, self.ledger.finalize_evidence(receipt))?;
        let terminal = match status {
            RunStatus::Done => ExecutionState::Completed,
            RunStatus::Failed => ExecutionState::Failed,
            RunStatus::Cancelled => ExecutionState::Cancelled,
            _ => ExecutionState::Blocked,
        };
        self.record(
            events,
            self.ledger.transition(terminal, reason.map(str::to_string)),
        )?;
        Ok(summary(&self.ledger.snapshot()))
    }

    fn record(
        &self,
        events: &mut Vec<AgentEvent>,
        event: Result<ExecutionEvent, String>,
    ) -> Result<(), String> {
        self.emit(events, event?);
        Ok(())
    }

    fn emit(&self, events: &mut Vec<AgentEvent>, event: ExecutionEvent) {
        events.push(AgentEvent::Trace {
            run: Some(self.run.clone()),
            source: "execution_lifecycle".to_string(),
            payload: serde_json::to_value(event).expect("execution events serialize"),
        });
    }
}

fn summary(snapshot: &ExecutionSnapshot) -> RunExecutionSummary {
    let mut completed_tools = BTreeSet::new();
    let mut failed_tools = BTreeSet::new();
    for tool in snapshot.evidence.tools.values() {
        match tool.status {
            ToolExecutionStatus::Completed => {
                completed_tools.insert(tool.name.clone());
            }
            ToolExecutionStatus::Failed | ToolExecutionStatus::Cancelled => {
                failed_tools.insert(tool.name.clone());
            }
        }
    }
    RunExecutionSummary {
        execution_id: snapshot.id.to_string(),
        root_path: snapshot.root.to_string(),
        attempts: snapshot.attempts.len() as u32,
        recoveries: snapshot.recoveries,
        child_executions: snapshot.children.len().try_into().unwrap_or(u32::MAX),
        completed_children: snapshot
            .children
            .values()
            .filter(|child| child.status == agent_orchestration::AgentStatus::Completed)
            .count()
            .try_into()
            .unwrap_or(u32::MAX),
        failed_children: snapshot
            .children
            .values()
            .filter(|child| {
                matches!(
                    child.status,
                    agent_orchestration::AgentStatus::Interrupted
                        | agent_orchestration::AgentStatus::Errored
                        | agent_orchestration::AgentStatus::Shutdown
                )
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX),
        weighted_tokens: snapshot.usage.weighted_tokens,
        cost_usd: snapshot.usage.cost_usd,
        changed_paths: snapshot.evidence.changed_paths.iter().cloned().collect(),
        completed_tools: completed_tools.into_iter().collect(),
        failed_tools: failed_tools.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_trace_round_trips_through_normalized_events() {
        let session = SessionId::new("session-1");
        let run = RunId::new("run-1");
        let mut events = Vec::new();
        let lifecycle = ScriptedLifecycle::new(&session, &run, &mut events).unwrap();
        lifecycle
            .tool_started(&mut events, "call-1", "write_file", true)
            .unwrap();
        lifecycle
            .tool_finished(
                &mut events,
                "call-1",
                BTreeSet::from(["src/lib.rs".to_string()]),
            )
            .unwrap();
        let summary = lifecycle
            .finish_with_usage(
                &mut events,
                RunStatus::Done,
                RunUsage {
                    input_tokens: 10,
                    output_tokens: 2,
                    ..Default::default()
                },
            )
            .unwrap();

        let trace = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Trace { payload, .. } => {
                    serde_json::from_value::<ExecutionEvent>(payload.clone()).ok()
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let replay = ExecutionLedger::replay(&trace).unwrap();
        assert_eq!(replay.state, ExecutionState::Completed);
        assert_eq!(summary.attempts, 1);
        assert_eq!(summary.completed_tools, vec!["write_file"]);
    }

    #[test]
    fn recovery_matrix_improves_only_safe_transient_boundaries() {
        let cases = recovery_matrix();
        assert_eq!(cases.len(), 8);
        assert!(cases.iter().all(|case| case.safety_passed));
        assert!(cases.iter().all(|case| case.duplicate_tool_receipts == 0));
        let recovered = cases
            .iter()
            .find(|case| case.case == "transient_after_completed_write")
            .unwrap();
        assert_eq!(recovered.baseline_correctness, 0.0);
        assert_eq!(recovered.lifecycle_correctness, 1.0);
        assert_eq!(recovered.attempts, 2);
        let active_write = cases
            .iter()
            .find(|case| case.case == "transient_during_write")
            .unwrap();
        assert!(!active_write.recovery_allowed);
        assert_eq!(active_write.attempts, 1);
    }
}
