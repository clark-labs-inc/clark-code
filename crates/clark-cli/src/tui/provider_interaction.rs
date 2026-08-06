use std::collections::VecDeque;

use agent_core::{RunId, SessionId};

use super::provider_events::SteeringDisposition;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CancelRequest {
    pub(crate) session: SessionId,
    pub(crate) run: RunId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SteeringEffect {
    Delivered(String),
    Queued(String),
    Restore { message: String, error: String },
}

#[derive(Debug, Default)]
pub(crate) struct ProviderInteractionSimulation {
    queued_follow_ups: VecDeque<String>,
}

impl ProviderInteractionSimulation {
    pub(crate) fn cancellation(
        &self,
        session: &SessionId,
        run: Option<&RunId>,
    ) -> Option<CancelRequest> {
        run.map(|run| CancelRequest {
            session: session.clone(),
            run: run.clone(),
        })
    }

    pub(crate) fn resolve_steering(
        &mut self,
        message: String,
        disposition: SteeringDisposition,
    ) -> SteeringEffect {
        match disposition {
            SteeringDisposition::Delivered => SteeringEffect::Delivered(message),
            SteeringDisposition::QueueFollowUp => {
                self.queued_follow_ups.push_back(message.clone());
                SteeringEffect::Queued(message)
            }
            SteeringDisposition::RestoreInput(error) => SteeringEffect::Restore { message, error },
        }
    }

    pub(crate) fn next_follow_up(&mut self) -> Option<String> {
        self.queued_follow_ups.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_preserves_exact_session_and_run_identity() {
        let state = ProviderInteractionSimulation::default();
        let session = SessionId::new("session-7");
        let run = RunId::new("run-9");
        assert_eq!(
            state.cancellation(&session, Some(&run)),
            Some(CancelRequest {
                session: session.clone(),
                run: run.clone(),
            })
        );
        assert_eq!(state.cancellation(&session, None), None);
    }

    #[test]
    fn unsupported_steering_queues_exactly_one_ordered_follow_up() {
        let mut state = ProviderInteractionSimulation::default();
        assert_eq!(
            state.resolve_steering("first".into(), SteeringDisposition::QueueFollowUp),
            SteeringEffect::Queued("first".into())
        );
        assert_eq!(
            state.resolve_steering("second".into(), SteeringDisposition::QueueFollowUp),
            SteeringEffect::Queued("second".into())
        );
        assert_eq!(state.next_follow_up().as_deref(), Some("first"));
        assert_eq!(state.next_follow_up().as_deref(), Some("second"));
        assert_eq!(state.next_follow_up(), None);
    }

    #[test]
    fn transport_failure_restores_input_without_queuing() {
        let mut state = ProviderInteractionSimulation::default();
        assert_eq!(
            state.resolve_steering(
                "retry me".into(),
                SteeringDisposition::RestoreInput("offline".into()),
            ),
            SteeringEffect::Restore {
                message: "retry me".into(),
                error: "offline".into(),
            }
        );
        assert_eq!(state.next_follow_up(), None);
    }

    #[test]
    fn delivered_steering_never_enters_follow_up_queue() {
        let mut state = ProviderInteractionSimulation::default();
        assert_eq!(
            state.resolve_steering("now".into(), SteeringDisposition::Delivered),
            SteeringEffect::Delivered("now".into())
        );
        assert_eq!(state.next_follow_up(), None);
    }
}
