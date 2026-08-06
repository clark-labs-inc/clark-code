use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use agent_core::domain::{AgentEvent, RunExecutionSummary, RunUsage};
use agent_core::ids::{RunId, SessionId};
use agent_orchestration::{
    AgentPath, AgentRole, AgentStatus, ExecutionEvent, ExecutionId, ExecutionLedger,
    ExecutionPolicy, ExecutionSnapshot, ExecutionState, FailureClass, ToolExecutionStatus,
    UsageCharge,
};
use async_channel::Sender;
use serde_json::Value;

use crate::exec::Executor;

#[derive(Clone, Debug)]
pub(crate) struct RootExecutionConfig {
    pub max_attempts: u32,
    pub weighted_token_limit: Option<f64>,
    pub max_cost_usd: Option<f64>,
}

impl Default for RootExecutionConfig {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            weighted_token_limit: None,
            max_cost_usd: None,
        }
    }
}

impl RootExecutionConfig {
    pub(crate) fn from_extra(extra: &Value) -> Self {
        let Some(object) = extra.get("execution").and_then(Value::as_object) else {
            return Self::default();
        };
        let defaults = Self::default();
        let max_attempts = object
            .get("max_attempts")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(defaults.max_attempts)
            .clamp(1, 3);
        let weighted_token_limit = positive_f64(object.get("weighted_token_limit"));
        let max_cost_usd = non_negative_f64(object.get("max_cost_usd"));
        Self {
            max_attempts,
            weighted_token_limit,
            max_cost_usd,
        }
    }

    fn policy(&self) -> ExecutionPolicy {
        ExecutionPolicy {
            max_attempts: self.max_attempts,
            weighted_token_limit: self.weighted_token_limit,
            max_cost_usd: self.max_cost_usd,
            ..Default::default()
        }
    }
}

fn positive_f64(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn non_negative_f64(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
}

/// Provider-local adapter for the provider-neutral root execution ledger.
/// Recording is synchronous and deterministic; normalized Trace delivery uses
/// the unbounded run channel and never blocks the model/tool loop.
#[derive(Clone)]
pub(crate) struct RootExecutionTrace {
    ledger: Arc<ExecutionLedger>,
    events: Sender<AgentEvent>,
    run: RunId,
}

pub(crate) struct RecoveryBoundary {
    pub attempt: u32,
    pub max_attempts: u32,
    pub receipt: agent_core::recovery::ExecutionBoundaryReceipt,
}

impl RootExecutionTrace {
    pub(crate) fn new(
        session: &SessionId,
        run: &RunId,
        config: &RootExecutionConfig,
        events: Sender<AgentEvent>,
    ) -> Result<Self, String> {
        let id = ExecutionId::new(format!("{}:{}", session.as_str(), run.as_str()))?;
        let ledger = Arc::new(ExecutionLedger::new_root(id, config.policy())?);
        let trace = Self {
            ledger,
            events,
            run: run.clone(),
        };
        trace.emit(trace.ledger.created_event());
        trace.record(trace.ledger.start_attempt());
        Ok(trace)
    }

    pub(crate) fn checkpoint(&self, id: impl Into<String>) {
        self.record(self.ledger.checkpoint(id));
    }

    pub(crate) fn transition(&self, state: ExecutionState, reason: Option<String>) {
        self.record(self.ledger.transition(state, reason));
    }

    pub(crate) fn steering(&self) {
        self.record(self.ledger.record_steering());
    }

    pub(crate) fn tool_started(&self, id: &str, name: &str, mutating: bool) {
        self.record(
            self.ledger
                .tool_started(self.tool_key(id), name.to_string(), mutating),
        );
    }

    pub(crate) fn tool_finished(
        &self,
        id: &str,
        status: ToolExecutionStatus,
        locations: BTreeSet<String>,
    ) {
        self.record(
            self.ledger
                .tool_finished(self.tool_key(id), status, locations),
        );
    }

    pub(crate) fn record_usage_delta(&self, before: Option<RunUsage>, after: Option<RunUsage>) {
        let before = before.unwrap_or_default();
        let after = after.unwrap_or_default();
        let cost = after.cost_usd.unwrap_or(0.0) - before.cost_usd.unwrap_or(0.0);
        let usage = UsageCharge {
            input_tokens: after.input_tokens.saturating_sub(before.input_tokens),
            cached_input_tokens: 0,
            output_tokens: after.output_tokens.saturating_sub(before.output_tokens),
            cost_usd: cost.max(0.0),
        };
        if usage.input_tokens > 0 || usage.output_tokens > 0 || usage.cost_usd > 0.0 {
            self.record(self.ledger.record_usage(usage));
        }
    }

    pub(crate) fn can_recover(&self, failure: FailureClass) -> bool {
        self.ledger.recovery_decision(failure).allowed
    }

    pub(crate) fn schedule_recovery(&self, failure: FailureClass, message: String) -> bool {
        match self.ledger.schedule_recovery(failure, message) {
            Ok(event) => {
                self.emit(event);
                self.record(self.ledger.start_attempt());
                true
            }
            Err(error) => {
                tracing::warn!(%error, "root execution recovery rejected");
                false
            }
        }
    }

    pub(crate) fn recovery_boundary(&self) -> RecoveryBoundary {
        let snapshot = self.ledger.snapshot();
        let mut started = BTreeMap::new();
        let mut completed_tools = 0_u32;
        let mut last_completed_tool_id = None;
        let mut last_completed_tool_name = None;
        let events = self.ledger.events();
        for event in &events {
            match &event.kind {
                agent_orchestration::ExecutionEventKind::ToolStarted { id, name, .. } => {
                    started.insert(id.clone(), name.clone());
                }
                agent_orchestration::ExecutionEventKind::ToolFinished {
                    id,
                    status: ToolExecutionStatus::Completed,
                    ..
                } => {
                    completed_tools = completed_tools.saturating_add(1);
                    last_completed_tool_id = Some(id.clone());
                    last_completed_tool_name = started.get(id).cloned();
                }
                _ => {}
            }
        }
        let attempt = snapshot
            .attempts
            .len()
            .saturating_add(1)
            .try_into()
            .unwrap_or(u32::MAX);
        let event_sequence = events.last().map(|event| event.sequence).unwrap_or(0);
        RecoveryBoundary {
            attempt,
            max_attempts: snapshot.policy.max_attempts,
            receipt: agent_core::recovery::ExecutionBoundaryReceipt {
                execution_id: snapshot.id.to_string(),
                attempt_sequence: snapshot
                    .active_attempt
                    .as_ref()
                    .map(|attempt| attempt.sequence)
                    .unwrap_or_else(|| attempt.saturating_sub(1)),
                event_sequence,
                transcript_commit_id: format!(
                    "{}:transcript-commit:{event_sequence}",
                    self.run.as_str()
                ),
                completed_tools,
                last_completed_tool_id,
                last_completed_tool_name,
                baseline_checkpoint_id: snapshot.evidence.baseline_checkpoint,
            },
        }
    }

    pub(crate) fn attach_child(&self, path: AgentPath, role: AgentRole) {
        self.record(self.ledger.attach_child(path, role));
    }

    pub(crate) fn update_child(&self, path: AgentPath, status: AgentStatus) {
        self.record(self.ledger.update_child(path, status));
    }

    pub(crate) fn record_child_budget(&self, weighted_tokens: f64, cost_usd: f64) {
        // Child providers currently normalize only their weighted aggregate at
        // this boundary. Preserve it exactly as input-token-equivalent work;
        // the trace labels the field as weighted rather than pretending cache
        // or raw token detail was available.
        let input_tokens = if weighted_tokens.is_finite() && weighted_tokens > 0.0 {
            weighted_tokens.min(u64::MAX as f64) as u64
        } else {
            0
        };
        self.record(self.ledger.record_usage(UsageCharge {
            input_tokens,
            cached_input_tokens: 0,
            output_tokens: 0,
            cost_usd: cost_usd.max(0.0),
        }));
    }

    pub(crate) async fn finalize(
        &self,
        executor: &dyn Executor,
        root: &std::path::Path,
        terminal: ExecutionState,
        reason: Option<String>,
    ) -> RunExecutionSummary {
        debug_assert!(terminal.is_final());
        self.close_unfinished_tools();
        let state = self.ledger.snapshot().state;
        if state == ExecutionState::AwaitingInput {
            self.transition(
                ExecutionState::Running,
                Some("permission wait closed before finalization".to_string()),
            );
        }
        if self.ledger.snapshot().state == ExecutionState::Running {
            self.transition(ExecutionState::Verifying, None);
        }

        if self.ledger.snapshot().state == ExecutionState::Verifying {
            let snapshot = self.ledger.snapshot();
            let mut receipt = snapshot.evidence.clone();
            if let Some(checkpoint) = receipt.baseline_checkpoint.clone() {
                match crate::changes::changes_summary(executor, root, &checkpoint).await {
                    Ok(changes) => {
                        receipt
                            .changed_paths
                            .extend(changes.into_iter().map(|change| change.path));
                    }
                    Err(error) => {
                        tracing::debug!(%error, "root execution change receipt unavailable");
                    }
                }
            }
            self.record(self.ledger.finalize_evidence(receipt));
            self.transition(terminal, reason);
        }
        summary(&self.ledger.snapshot())
    }

    fn close_unfinished_tools(&self) {
        let active = self
            .ledger
            .snapshot()
            .active_tools
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in active {
            self.record(self.ledger.tool_finished(
                id,
                ToolExecutionStatus::Cancelled,
                BTreeSet::new(),
            ));
        }
    }

    fn tool_key(&self, id: &str) -> String {
        let sequence = self
            .ledger
            .snapshot()
            .active_attempt
            .map(|attempt| attempt.sequence)
            .unwrap_or(0);
        format!("{sequence}:{id}")
    }

    fn record(&self, result: Result<ExecutionEvent, String>) {
        match result {
            Ok(event) => self.emit(event),
            Err(error) => tracing::warn!(%error, "root execution event rejected"),
        }
    }

    fn emit(&self, event: ExecutionEvent) {
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        let _ = self.events.try_send(AgentEvent::Trace {
            run: Some(self.run.clone()),
            source: "execution_lifecycle".to_string(),
            payload,
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
    let completed_children = snapshot
        .children
        .values()
        .filter(|child| child.status == AgentStatus::Completed)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let failed_children = snapshot
        .children
        .values()
        .filter(|child| {
            matches!(
                child.status,
                AgentStatus::Interrupted | AgentStatus::Errored | AgentStatus::Shutdown
            )
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    RunExecutionSummary {
        execution_id: snapshot.id.to_string(),
        root_path: snapshot.root.to_string(),
        attempts: snapshot.attempts.len() as u32,
        recoveries: snapshot.recoveries,
        child_executions: snapshot.children.len().try_into().unwrap_or(u32::MAX),
        completed_children,
        failed_children,
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
    fn execution_config_is_bounded_and_defaults_to_one_recovery() {
        let defaults = RootExecutionConfig::from_extra(&serde_json::json!({}));
        assert_eq!(defaults.max_attempts, 2);
        assert_eq!(defaults.weighted_token_limit, None);

        let bounded = RootExecutionConfig::from_extra(&serde_json::json!({
            "execution": {
                "max_attempts": 99,
                "weighted_token_limit": 50_000,
                "max_cost_usd": 0.5
            }
        }));
        assert_eq!(bounded.max_attempts, 3);
        assert_eq!(bounded.weighted_token_limit, Some(50_000.0));
        assert_eq!(bounded.max_cost_usd, Some(0.5));
    }
}
