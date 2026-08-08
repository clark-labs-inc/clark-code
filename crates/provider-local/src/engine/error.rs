use agent_core::domain::{RunFailureKind, RunStatus};

#[derive(Clone)]
pub(super) struct MappedLoopError {
    pub(super) status: RunStatus,
    pub(super) run_error: Option<String>,
    pub(super) failure_kind: Option<RunFailureKind>,
    pub(super) ui_error: Option<(String, String)>,
}

pub(super) fn map_loop_error(error: agent_loop::LoopError) -> MappedLoopError {
    match error {
        agent_loop::LoopError::Aborted => MappedLoopError {
            status: RunStatus::Cancelled,
            run_error: None,
            failure_kind: None,
            ui_error: None,
        },
        agent_loop::LoopError::Stream(stream) => map_stream_error(stream),
        agent_loop::LoopError::ToolFatal { tool, reason } => {
            let message = format!("fatal tool `{tool}` error: {reason}");
            MappedLoopError::failed(RunFailureKind::ToolFatal, "tool_fatal", message)
        }
        agent_loop::LoopError::InvalidContinuation(message) => {
            MappedLoopError::failed(RunFailureKind::LocalState, "local_agent_state", message)
        }
        agent_loop::LoopError::EmptyOutcomeBudgetExhausted { budget, observed } => {
            MappedLoopError::failed(
                RunFailureKind::EmptyResponse,
                "empty_agent_response",
                format!(
                    "empty assistant outcome retry budget exhausted: observed {observed}, budget {budget}"
                ),
            )
        }
    }
}

pub(super) fn map_loop_error_with_completion_state(
    error: agent_loop::LoopError,
    final_answer_committed: bool,
    goal_completed: bool,
    unresolved_effects: &[String],
) -> MappedLoopError {
    let completion_delivery_failed = matches!(
        &error,
        agent_loop::LoopError::Stream(agent_loop::StreamError::Fatal(message))
            if message
                .strip_prefix("provider_error:")
                .is_some_and(|message| message.starts_with(crate::llm::REQUIRED_TOOL_CONTRACT_VIOLATION))
    );
    let mapped = map_loop_error(error);
    if mapped.failure_kind != Some(RunFailureKind::EmptyResponse) && !completion_delivery_failed {
        return mapped;
    }

    // A final answer or a typed goal-complete signal means the work itself
    // has already been delivered. Pending external effects remain a distinct
    // failure because they still need canonical verification.
    if !unresolved_effects.is_empty() && (final_answer_committed || goal_completed) {
        return MappedLoopError::verification_incomplete(unresolved_effects);
    }

    // `update_goal(complete)` is emitted before the model's final post-tool
    // delivery. If that response is empty or exhausts required-tool repair,
    // the completed goal is still the authoritative terminal receipt. A final
    // answer alone is not enough: it can be followed by a user steering turn.
    if goal_completed {
        return MappedLoopError::completed();
    }

    mapped
}

fn map_stream_error(error: agent_loop::StreamError) -> MappedLoopError {
    match error {
        agent_loop::StreamError::Fatal(message) if message.starts_with("insufficient_credits:") => {
            MappedLoopError::failed(
                RunFailureKind::InsufficientCredits,
                "insufficient_credits",
                message,
            )
        }
        agent_loop::StreamError::Fatal(message)
            if message.starts_with("platform_key_rejected:") =>
        {
            let message = message
                .strip_prefix("platform_key_rejected:")
                .unwrap_or(&message)
                .to_string();
            MappedLoopError::failed(
                RunFailureKind::PlatformKeyRejected,
                "platform_key_rejected",
                message,
            )
        }
        agent_loop::StreamError::Fatal(message) if message.starts_with("provider_error:") => {
            let message = message.strip_prefix("provider_error:").unwrap_or(&message);
            let message = message
                .strip_prefix(crate::llm::REQUIRED_TOOL_CONTRACT_VIOLATION)
                .unwrap_or(message)
                .trim()
                .to_string();
            MappedLoopError::failed(RunFailureKind::ProviderError, "provider_error", message)
        }
        agent_loop::StreamError::Fatal(message)
            if message.starts_with("execution_budget_exhausted:") =>
        {
            let message = message
                .strip_prefix("execution_budget_exhausted:")
                .unwrap_or(&message)
                .trim()
                .to_string();
            MappedLoopError::failed(
                RunFailureKind::LocalState,
                "execution_budget_exhausted",
                message,
            )
        }
        agent_loop::StreamError::ProviderRateLimited(message) => {
            MappedLoopError::failed(RunFailureKind::RateLimited, "rate_limited", message)
        }
        agent_loop::StreamError::Transient(message) => {
            MappedLoopError::failed(RunFailureKind::TransportError, "transport_error", message)
        }
        agent_loop::StreamError::ContextOverflow(message) => {
            MappedLoopError::failed(RunFailureKind::ContextOverflow, "context_overflow", message)
        }
        agent_loop::StreamError::ZeroOutputTransport(message) => MappedLoopError::failed(
            RunFailureKind::EmptyResponse,
            "empty_agent_response",
            message,
        ),
        agent_loop::StreamError::Fatal(message) => {
            MappedLoopError::failed(RunFailureKind::ProviderError, "provider_error", message)
        }
        agent_loop::StreamError::Empty => MappedLoopError::failed(
            RunFailureKind::EmptyResponse,
            "empty_agent_response",
            "model returned an empty response".to_string(),
        ),
    }
}

impl MappedLoopError {
    fn completed() -> Self {
        Self {
            status: RunStatus::Done,
            run_error: None,
            failure_kind: None,
            ui_error: None,
        }
    }

    fn failed(failure_kind: RunFailureKind, code: &str, message: String) -> Self {
        Self {
            status: RunStatus::Failed,
            run_error: Some(message.clone()),
            failure_kind: Some(failure_kind),
            ui_error: Some((code.to_string(), message)),
        }
    }

    pub(super) fn verification_incomplete(unresolved: &[String]) -> Self {
        let effects = if unresolved.len() == 1 {
            "effect"
        } else {
            "effects"
        };
        Self::failed(
            RunFailureKind::VerificationIncomplete,
            "effect_verification_incomplete",
            format!(
                "{} external {effects} remained unverified after the final answer:\n- {}",
                unresolved.len(),
                unresolved.join("\n- ")
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_failures_keep_their_typed_category() {
        let cases = [
            (
                agent_loop::StreamError::Fatal(
                    "platform_key_rejected:model endpoint returned 401".into(),
                ),
                RunFailureKind::PlatformKeyRejected,
            ),
            (
                agent_loop::StreamError::ProviderRateLimited("busy".into()),
                RunFailureKind::RateLimited,
            ),
            (
                agent_loop::StreamError::Transient("connection reset".into()),
                RunFailureKind::TransportError,
            ),
            (
                agent_loop::StreamError::Fatal("provider_error:upstream failed".into()),
                RunFailureKind::ProviderError,
            ),
            (
                agent_loop::StreamError::ContextOverflow("too large".into()),
                RunFailureKind::ContextOverflow,
            ),
            (
                agent_loop::StreamError::Fatal(
                    "execution_budget_exhausted: preserved for follow-up".into(),
                ),
                RunFailureKind::LocalState,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(map_stream_error(error).failure_kind, Some(expected));
        }
    }

    #[test]
    fn incomplete_verification_has_its_own_failure_category() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::EmptyOutcomeBudgetExhausted {
                budget: 1,
                observed: 2,
            },
            true,
            false,
            &["`effect-1` pending".into(), "`effect-2` mismatched".into()],
        );
        assert_eq!(
            mapped.failure_kind,
            Some(RunFailureKind::VerificationIncomplete)
        );
        assert_eq!(
            mapped.ui_error.as_ref().map(|(code, _)| code.as_str()),
            Some("effect_verification_incomplete")
        );
        assert!(mapped.run_error.unwrap().contains("2 external effects"));
    }

    #[test]
    fn completed_goal_with_pending_effects_stays_verification_incomplete() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::ZeroOutputTransport(
                "provider returned no content".into(),
            )),
            false,
            true,
            &["`effect-1` pending".into()],
        );
        assert_eq!(
            mapped.failure_kind,
            Some(RunFailureKind::VerificationIncomplete)
        );
    }

    #[test]
    fn completed_goal_stays_done_when_its_post_tool_response_is_empty() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::ZeroOutputTransport(
                "provider returned no content".into(),
            )),
            false,
            true,
            &[],
        );
        assert_eq!(mapped.status, RunStatus::Done);
        assert_eq!(mapped.failure_kind, None);
        assert_eq!(mapped.run_error, None);
        assert_eq!(mapped.ui_error, None);
    }

    #[test]
    fn completed_goal_stays_done_when_required_tool_repair_is_exhausted() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::Fatal(format!(
                "provider_error:{} provider ignored required tool choice",
                crate::llm::REQUIRED_TOOL_CONTRACT_VIOLATION
            ))),
            false,
            true,
            &[],
        );
        assert_eq!(mapped.status, RunStatus::Done);
        assert_eq!(mapped.failure_kind, None);
        assert_eq!(mapped.run_error, None);
        assert_eq!(mapped.ui_error, None);
    }

    #[test]
    fn final_answer_alone_does_not_hide_a_later_empty_turn() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::EmptyOutcomeBudgetExhausted {
                budget: 1,
                observed: 2,
            },
            true,
            false,
            &[],
        );
        assert_eq!(mapped.failure_kind, Some(RunFailureKind::EmptyResponse));
    }

    #[test]
    fn genuinely_empty_first_answer_stays_an_empty_response() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::EmptyOutcomeBudgetExhausted {
                budget: 1,
                observed: 2,
            },
            false,
            false,
            &["`effect-1` pending".into()],
        );
        assert_eq!(mapped.failure_kind, Some(RunFailureKind::EmptyResponse));
    }
}
