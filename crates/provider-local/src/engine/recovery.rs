use agent_core::recovery::ExecutionRecovery;
use agent_orchestration::FailureClass;

use crate::root_execution::RecoveryBoundary;

pub(super) fn candidate(error: &agent_loop::LoopError) -> Option<(FailureClass, String)> {
    match error {
        agent_loop::LoopError::Stream(agent_loop::StreamError::Transient(message)) => {
            Some((FailureClass::TransientTransport, message.clone()))
        }
        agent_loop::LoopError::Stream(agent_loop::StreamError::ProviderRateLimited(message)) => {
            Some((FailureClass::RateLimited, message.clone()))
        }
        _ => None,
    }
}

pub(super) fn transcript_marker() -> agent_loop::AgentMessage {
    agent_loop::AgentMessage::User {
        content: agent_loop::UserContent::Text(
            "[runtime recovery — the previous model stream failed after every started tool had a \
             terminal receipt. Completed transcript and current workspace state were preserved. \
             Re-read any state you depend on, do not repeat completed writes, and continue from \
             the current repository.]"
                .to_string(),
        ),
        timestamp: None,
    }
}

pub(super) fn execution_recovery(boundary: RecoveryBoundary) -> ExecutionRecovery {
    ExecutionRecovery {
        attempt: boundary.attempt,
        max_attempts: boundary.max_attempts,
        boundary: boundary.receipt,
        started_at_ms: crate::llm::now_ms(),
    }
}
