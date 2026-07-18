use agent_core::domain::{RunFailureKind, RunStatus};

#[derive(Clone)]
pub(super) struct MappedLoopError {
    pub(super) status: RunStatus,
    pub(super) run_error: Option<String>,
    pub(super) failure_kind: Option<RunFailureKind>,
    pub(super) ui_error: Option<(String, String)>,
}

pub(super) fn map_loop_error(error: clark_agent::LoopError) -> MappedLoopError {
    match error {
        clark_agent::LoopError::Aborted => MappedLoopError {
            status: RunStatus::Cancelled,
            run_error: None,
            failure_kind: None,
            ui_error: None,
        },
        clark_agent::LoopError::Stream(stream) => map_stream_error(stream),
        clark_agent::LoopError::ToolFatal { tool, reason } => {
            let message = format!("fatal tool `{tool}` error: {reason}");
            MappedLoopError::failed(RunFailureKind::ToolFatal, "tool_fatal", message)
        }
        clark_agent::LoopError::InvalidContinuation(message) => {
            MappedLoopError::failed(RunFailureKind::LocalState, "local_agent_state", message)
        }
        clark_agent::LoopError::EmptyOutcomeBudgetExhausted { budget, observed } => {
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

fn map_stream_error(error: clark_agent::StreamError) -> MappedLoopError {
    match error {
        clark_agent::StreamError::Fatal(message)
            if message.starts_with("insufficient_credits:") =>
        {
            MappedLoopError::failed(
                RunFailureKind::InsufficientCredits,
                "insufficient_credits",
                message,
            )
        }
        clark_agent::StreamError::Fatal(message)
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
        clark_agent::StreamError::Fatal(message) if message.starts_with("provider_error:") => {
            let message = message
                .strip_prefix("provider_error:")
                .unwrap_or(&message)
                .to_string();
            MappedLoopError::failed(RunFailureKind::ProviderError, "provider_error", message)
        }
        clark_agent::StreamError::ProviderRateLimited(message) => {
            MappedLoopError::failed(RunFailureKind::RateLimited, "rate_limited", message)
        }
        clark_agent::StreamError::Transient(message) => {
            MappedLoopError::failed(RunFailureKind::TransportError, "transport_error", message)
        }
        clark_agent::StreamError::ContextOverflow(message) => {
            MappedLoopError::failed(RunFailureKind::ContextOverflow, "context_overflow", message)
        }
        clark_agent::StreamError::ZeroOutputTransport(message) => MappedLoopError::failed(
            RunFailureKind::EmptyResponse,
            "empty_agent_response",
            message,
        ),
        clark_agent::StreamError::Fatal(message) => {
            MappedLoopError::failed(RunFailureKind::ProviderError, "provider_error", message)
        }
        clark_agent::StreamError::Empty => MappedLoopError::failed(
            RunFailureKind::EmptyResponse,
            "empty_agent_response",
            "model returned an empty response".to_string(),
        ),
    }
}

impl MappedLoopError {
    fn failed(failure_kind: RunFailureKind, code: &str, message: String) -> Self {
        Self {
            status: RunStatus::Failed,
            run_error: Some(message.clone()),
            failure_kind: Some(failure_kind),
            ui_error: Some((code.to_string(), message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_failures_keep_their_typed_category() {
        let cases = [
            (
                clark_agent::StreamError::Fatal(
                    "platform_key_rejected:model endpoint returned 401".into(),
                ),
                RunFailureKind::PlatformKeyRejected,
            ),
            (
                clark_agent::StreamError::ProviderRateLimited("busy".into()),
                RunFailureKind::RateLimited,
            ),
            (
                clark_agent::StreamError::Transient("connection reset".into()),
                RunFailureKind::TransportError,
            ),
            (
                clark_agent::StreamError::Fatal("provider_error:upstream failed".into()),
                RunFailureKind::ProviderError,
            ),
            (
                clark_agent::StreamError::ContextOverflow("too large".into()),
                RunFailureKind::ContextOverflow,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(map_stream_error(error).failure_kind, Some(expected));
        }
    }
}
