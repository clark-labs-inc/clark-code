use std::sync::{Arc, Mutex};

use agent_core::domain::AgentEvent;
use agent_core::ids::RunId;
use agent_core::recovery::{
    ExecutionRecovery, ProviderFailureClass, ProviderIncident, ProviderIncidentScope,
    ProviderIncidentStatus, ProviderRequestDiagnostics,
};
use async_channel::Sender;

use crate::llm::{now_ms, ProviderFailureContext};

#[derive(Clone)]
pub(crate) struct ProviderIncidentTracker {
    inner: Arc<Mutex<State>>,
    events: Sender<AgentEvent>,
    run: RunId,
}

#[derive(Default)]
struct State {
    sequence: u32,
    current: Option<ProviderIncident>,
}

impl ProviderIncidentTracker {
    pub(crate) fn new(run: RunId, events: Sender<AgentEvent>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State::default())),
            events,
            run,
        }
    }

    pub(crate) fn observe_retry(&self, context: ProviderFailureContext) {
        self.observe(context, ProviderIncidentStatus::Retrying);
    }

    pub(crate) fn observe_terminal(&self, context: ProviderFailureContext) {
        self.observe(context, ProviderIncidentStatus::Observed);
    }

    fn observe(&self, context: ProviderFailureContext, status: ProviderIncidentStatus) {
        let incident = {
            let mut state = self.inner.lock().expect("provider incident lock");
            let start_new = state
                .current
                .as_ref()
                .is_none_or(|incident| is_terminal(incident.status));
            if start_new {
                state.sequence = state.sequence.saturating_add(1);
                state.current = Some(new_incident(&self.run, state.sequence, context, status));
            } else if let Some(incident) = state.current.as_mut() {
                update_incident(incident, context, status);
            }
            state.current.clone().expect("incident was initialized")
        };
        self.emit(incident);
    }

    pub(crate) fn attach_execution_recovery(&self, recovery: ExecutionRecovery) {
        self.update_active(|incident| {
            incident.status = ProviderIncidentStatus::Retrying;
            incident.updated_at_ms = recovery.started_at_ms;
            incident.execution_recovery = Some(recovery);
            incident.completed_at_ms = None;
        });
    }

    pub(crate) fn mark_recovered(&self) {
        self.settle(ProviderIncidentStatus::Recovered);
    }

    pub(crate) fn mark_failed(&self) {
        self.settle(ProviderIncidentStatus::Failed);
    }

    fn settle(&self, status: ProviderIncidentStatus) {
        let now = now_ms();
        self.update_active(|incident| {
            incident.status = status;
            incident.updated_at_ms = now;
            incident.completed_at_ms = Some(now);
        });
    }

    fn update_active(&self, mutate: impl FnOnce(&mut ProviderIncident)) {
        let incident = {
            let mut state = self.inner.lock().expect("provider incident lock");
            let Some(incident) = state.current.as_mut() else {
                return;
            };
            if is_terminal(incident.status) {
                return;
            }
            mutate(incident);
            incident.clone()
        };
        self.emit(incident);
    }

    fn emit(&self, incident: ProviderIncident) {
        let _ = self.events.try_send(AgentEvent::ProviderIncidentUpdated {
            run: self.run.clone(),
            incident,
        });
    }
}

fn is_terminal(status: ProviderIncidentStatus) -> bool {
    matches!(
        status,
        ProviderIncidentStatus::Recovered
            | ProviderIncidentStatus::Failed
            | ProviderIncidentStatus::Interrupted
    )
}

fn summary(category: agent_core::recovery::ProviderIncidentCategory) -> &'static str {
    use agent_core::recovery::ProviderIncidentCategory;
    match category {
        ProviderIncidentCategory::Timeout => {
            "Model connection timed out while Clark Code was working."
        }
        ProviderIncidentCategory::RateLimit => "The model provider is temporarily rate limited.",
        ProviderIncidentCategory::UpstreamUnavailable => {
            "The model provider is temporarily unavailable."
        }
        ProviderIncidentCategory::ConnectionLost => "The model connection was interrupted.",
    }
}

fn failure_class(category: agent_core::recovery::ProviderIncidentCategory) -> ProviderFailureClass {
    if category == agent_core::recovery::ProviderIncidentCategory::RateLimit {
        ProviderFailureClass::RateLimited
    } else {
        ProviderFailureClass::TransientTransport
    }
}

fn request(context: &ProviderFailureContext) -> ProviderRequestDiagnostics {
    ProviderRequestDiagnostics {
        idempotency_key: context.idempotency_key.clone(),
        provider_request_id: context.provider_request_id.clone(),
        attempts: context.attempts,
        max_attempts: context.max_attempts,
        retries: context.retries.clone(),
        output_started: context.output_started,
        started_at_ms: context.request_started_at_ms,
    }
}

fn new_incident(
    run: &RunId,
    sequence: u32,
    context: ProviderFailureContext,
    status: ProviderIncidentStatus,
) -> ProviderIncident {
    let request = request(&context);
    ProviderIncident {
        id: format!("{}:provider-incident:{sequence}", run.as_str()),
        status,
        scope: ProviderIncidentScope::ModelRequest,
        failure_class: failure_class(context.category),
        category: context.category,
        message: summary(context.category).to_string(),
        detail: context.message,
        model: context.model,
        provider_route: context.provider_route,
        provider_status: context.provider_status,
        provider_error_type: context.provider_error_type,
        request,
        execution_recovery: None,
        observed_at_ms: context.observed_at_ms,
        updated_at_ms: context.observed_at_ms,
        completed_at_ms: None,
    }
}

fn update_incident(
    incident: &mut ProviderIncident,
    context: ProviderFailureContext,
    status: ProviderIncidentStatus,
) {
    let request = request(&context);
    incident.status = status;
    incident.failure_class = failure_class(context.category);
    incident.category = context.category;
    incident.message = summary(context.category).to_string();
    incident.detail = context.message;
    incident.model = context.model;
    incident.provider_route = context.provider_route;
    incident.provider_status = context.provider_status;
    incident.provider_error_type = context.provider_error_type;
    incident.request = request;
    incident.updated_at_ms = context.observed_at_ms;
    incident.completed_at_ms = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::recovery::{
        ExecutionBoundaryReceipt, ProviderIncidentCategory, ProviderRetryCounts,
    };

    fn context(category: ProviderIncidentCategory, attempts: u32) -> ProviderFailureContext {
        ProviderFailureContext {
            category,
            message: format!("failure {attempts}"),
            model: "test-model".into(),
            provider_route: "gateway.test/v1".into(),
            provider_status: None,
            provider_error_type: None,
            idempotency_key: "request-1".into(),
            provider_request_id: None,
            attempts,
            max_attempts: 4,
            retries: ProviderRetryCounts {
                transient: attempts,
                ..Default::default()
            },
            output_started: false,
            request_started_at_ms: 1,
            observed_at_ms: u64::from(attempts) + 1,
        }
    }

    fn next_incident(receiver: &async_channel::Receiver<AgentEvent>) -> ProviderIncident {
        match receiver.try_recv().unwrap() {
            AgentEvent::ProviderIncidentUpdated { incident, .. } => incident,
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn retries_update_one_incident_with_the_latest_terminal_cause() {
        let (sender, receiver) = async_channel::unbounded();
        let tracker = ProviderIncidentTracker::new(RunId::new("run-1"), sender);
        tracker.observe_retry(context(ProviderIncidentCategory::Timeout, 1));
        let first = next_incident(&receiver);
        tracker.observe_terminal(context(ProviderIncidentCategory::ConnectionLost, 4));
        let terminal_cause = next_incident(&receiver);
        tracker.attach_execution_recovery(ExecutionRecovery {
            attempt: 2,
            boundary: ExecutionBoundaryReceipt {
                execution_id: "run-1".into(),
                attempt_sequence: 1,
                event_sequence: 9,
                transcript_commit_id: "commit-9".into(),
                completed_tools: 3,
                last_completed_tool_id: None,
                last_completed_tool_name: None,
                baseline_checkpoint_id: None,
            },
            started_at_ms: 6,
        });
        let retrying = next_incident(&receiver);
        tracker.mark_failed();
        let failed = next_incident(&receiver);

        assert_eq!(first.id, terminal_cause.id);
        assert_eq!(first.id, retrying.id);
        assert_eq!(first.id, failed.id);
        assert_eq!(failed.status, ProviderIncidentStatus::Failed);
        assert_eq!(failed.category, ProviderIncidentCategory::ConnectionLost);
        assert_eq!(failed.request.attempts, 4);
        assert!(failed.execution_recovery.is_some());
    }

    #[test]
    fn unrelated_later_failures_do_not_reopen_a_recovered_incident() {
        let (sender, receiver) = async_channel::unbounded();
        let tracker = ProviderIncidentTracker::new(RunId::new("run-1"), sender);
        tracker.observe_retry(context(ProviderIncidentCategory::Timeout, 1));
        let _ = next_incident(&receiver);
        tracker.mark_recovered();
        let recovered = next_incident(&receiver);
        tracker.mark_failed();

        assert_eq!(recovered.status, ProviderIncidentStatus::Recovered);
        assert!(receiver.try_recv().is_err());
    }
}
