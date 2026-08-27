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
            if tool_protocol_exhausted_message(message).is_some()
    );
    let mapped = map_loop_error(error);

    // `update_goal(complete)` is a typed terminal receipt emitted before the
    // model's final post-tool narration request. Once it was committed in this
    // run, any failure of that delivery-only request (empty output, malformed
    // tool choice, rate limit, or transport loss) cannot undo completed work.
    // External effects still keep their independent verification obligation.
    if goal_completed {
        if !unresolved_effects.is_empty() {
            return MappedLoopError::verification_incomplete(unresolved_effects);
        }
        return MappedLoopError::completed();
    }

    if mapped.failure_kind != Some(RunFailureKind::EmptyResponse) && !completion_delivery_failed {
        return mapped;
    }

    // A final answer means the response itself was delivered. Pending external
    // effects remain a distinct failure because they still need canonical
    // verification.
    if !unresolved_effects.is_empty() && final_answer_committed {
        return MappedLoopError::verification_incomplete(unresolved_effects);
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
        agent_loop::StreamError::Fatal(message)
            if tool_protocol_exhausted_message(&message).is_some() =>
        {
            let message = tool_protocol_exhausted_message(&message)
                .unwrap_or(&message)
                .to_string();
            MappedLoopError::failed(
                RunFailureKind::ToolProtocolExhausted,
                "tool_protocol_exhausted",
                message,
            )
        }
        agent_loop::StreamError::Fatal(message) if message.starts_with("provider_error:") => {
            let message = message.strip_prefix("provider_error:").unwrap_or(&message);
            MappedLoopError::failed(
                RunFailureKind::ProviderError,
                "provider_error",
                message.trim().to_string(),
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
        agent_loop::StreamError::InconsistentToolHistory(message) => MappedLoopError::failed(
            RunFailureKind::InconsistentToolHistory,
            "inconsistent_tool_history",
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

fn tool_protocol_exhausted_message(message: &str) -> Option<&str> {
    message
        .strip_prefix(crate::agent_adapter::TOOL_PROTOCOL_EXHAUSTED_PREFIX)
        .map(str::trim)
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
                agent_loop::StreamError::Fatal(format!(
                    "{}model ignored named final_answer",
                    crate::agent_adapter::TOOL_PROTOCOL_EXHAUSTED_PREFIX
                )),
                RunFailureKind::ToolProtocolExhausted,
            ),
            (
                agent_loop::StreamError::ContextOverflow("too large".into()),
                RunFailureKind::ContextOverflow,
            ),
            (
                agent_loop::StreamError::InconsistentToolHistory(
                    "interleaved tool result batch".into(),
                ),
                RunFailureKind::InconsistentToolHistory,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(map_stream_error(error).failure_kind, Some(expected));
        }
    }

    #[test]
    fn inconsistent_history_is_not_collapsed_into_empty_or_provider_failure() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::InconsistentToolHistory(
                "tool result `call-2` interrupted the pending `call-1` batch".into(),
            )),
            false,
            false,
            &[],
        );

        assert_eq!(
            mapped.failure_kind,
            Some(RunFailureKind::InconsistentToolHistory)
        );
        assert_eq!(
            mapped.ui_error.as_ref().map(|(code, _)| code.as_str()),
            Some("inconsistent_tool_history")
        );
    }

    #[test]
    fn incomplete_verification_has_its_own_failure_category() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::ZeroOutputTransport(
                "provider returned no content".into(),
            )),
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
    fn completed_goal_stays_done_when_tool_protocol_repair_is_exhausted() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::Fatal(format!(
                "{}model returned no structured tool call",
                crate::agent_adapter::TOOL_PROTOCOL_EXHAUSTED_PREFIX
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
    fn completed_goal_stays_done_when_post_completion_transport_closes() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::Transient(
                "connection reset after update_goal complete".into(),
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
    fn final_answer_alone_does_not_hide_a_later_empty_turn() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::ZeroOutputTransport(
                "provider returned no content".into(),
            )),
            true,
            false,
            &[],
        );
        assert_eq!(mapped.failure_kind, Some(RunFailureKind::EmptyResponse));
    }

    #[test]
    fn genuinely_empty_first_answer_stays_an_empty_response() {
        let mapped = map_loop_error_with_completion_state(
            agent_loop::LoopError::Stream(agent_loop::StreamError::ZeroOutputTransport(
                "provider returned no content".into(),
            )),
            false,
            false,
            &["`effect-1` pending".into()],
        );
        assert_eq!(mapped.failure_kind, Some(RunFailureKind::EmptyResponse));
    }
}
