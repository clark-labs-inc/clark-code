use agent_core::recovery::ExecutionRecovery;
use agent_orchestration::FailureClass;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

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

const BACKGROUND_RECEIPT_OUTPUT_CHARS: usize = 12_000;
/// Codex keeps yielded commands session-owned and records their eventual exit
/// independently. Clark's run stream is turn-owned, so keep transient recovery
/// alive long enough to obtain the terminal receipt for an explicitly awaited
/// background build before the run can fail.
const AWAITED_BACKGROUND_RECOVERY_TIMEOUT: Duration = Duration::from_secs(90);
const PROVIDER_RECOVERY_BACKOFF: Duration = Duration::from_secs(10);

pub(super) async fn await_background_completion(
    background: &Arc<crate::background::BackgroundTasks>,
    cancel: &CancellationToken,
) -> Vec<(String, crate::background::TaskStatus)> {
    background
        .wait_for_awaited_completion(AWAITED_BACKGROUND_RECOVERY_TIMEOUT, cancel)
        .await
}

pub(super) async fn provider_backoff(cancel: &CancellationToken) {
    tokio::select! {
        _ = cancel.cancelled() => {}
        _ = tokio::time::sleep(PROVIDER_RECOVERY_BACKOFF) => {}
    }
}

pub(super) fn transcript_marker(
    completions: &[(String, crate::background::TaskStatus)],
) -> agent_loop::AgentMessage {
    let mut content = String::from(
        "[runtime recovery — the previous model stream failed after every started tool had a \
         terminal receipt. Completed transcript and current workspace state were preserved.",
    );
    for (id, status) in completions {
        let exit = match status.exit_code {
            Some(Some(code)) => code.to_string(),
            Some(None) => "signal".to_string(),
            None => "running".to_string(),
        };
        content.push_str(&format!(
            "\n\nHost-observed background completion `{id}` (exit {exit}) for:\n{}",
            status.command
        ));
        if let Some(error) = &status.error {
            content.push_str(&format!("\nerror: {error}"));
        }
        if !status.output.trim().is_empty() {
            content.push_str("\n--- captured output tail ---\n");
            content.push_str(&tail_chars(&status.output, BACKGROUND_RECEIPT_OUTPUT_CHARS));
        }
    }
    content.push_str(
        "\n\nRe-read any state you depend on, do not repeat completed writes, inspect every \
         completed background receipt above, fix failures, and continue from the current repository.]",
    );
    agent_loop::AgentMessage::User {
        content: agent_loop::UserContent::Text(content),
        timestamp: None,
    }
}

fn tail_chars(value: &str, limit: usize) -> String {
    let total = value.chars().count();
    if total <= limit {
        return value.to_string();
    }
    let tail = value
        .chars()
        .rev()
        .take(limit)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("... {} characters omitted ...\n{tail}", total - limit)
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;

pub(super) fn execution_recovery(boundary: RecoveryBoundary) -> ExecutionRecovery {
    ExecutionRecovery {
        attempt: boundary.attempt,
        max_attempts: boundary.max_attempts,
        boundary: boundary.receipt,
        started_at_ms: crate::llm::now_ms(),
    }
}
