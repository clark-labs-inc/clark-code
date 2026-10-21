use super::*;

/// Canonical messages that reached a complete commit boundary during a run.
///
/// `agent_loop::run` returns its message tail only on success. On an error it
/// still emits typed lifecycle events, so retain the initial prompt/steering
/// messages plus complete `TurnEnd` assistant/tool-result groups. Assistant
/// `MessageEnd` alone is not a commit boundary: it can be a discarded
/// max-token attempt, a transport error, or a tool turn that never completed.
#[derive(Clone, Default)]
pub(crate) struct CompletedRunTranscript {
    messages: Arc<std::sync::Mutex<Vec<ca::AgentMessage>>>,
}

impl CompletedRunTranscript {
    pub(super) fn observe(&self, event: &ca::AgentEvent) {
        let mut messages = self.messages.lock().expect("run transcript lock");
        match event {
            ca::AgentEvent::MessageEnd { message }
                if !matches!(message, ca::AgentMessage::Assistant { .. }) =>
            {
                messages.push(message.clone());
            }
            ca::AgentEvent::TurnEnd {
                message,
                tool_results,
            } if !matches!(
                message,
                ca::AgentMessage::Assistant {
                    stop_reason: ca::StopReason::Error | ca::StopReason::Aborted,
                    ..
                }
            ) =>
            {
                messages.push(message.clone());
                messages.extend(tool_results.iter().cloned());
            }
            _ => {}
        }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<ca::AgentMessage> {
        self.messages.lock().expect("run transcript lock").clone()
    }

    /// Whether this run already committed a user-visible final answer. A
    /// later hidden follow-up (for example effect verification) must not turn
    /// that answer into an "empty model response" if the follow-up itself
    /// produces no output.
    pub fn has_final_answer(&self) -> bool {
        self.messages
            .lock()
            .expect("run transcript lock")
            .iter()
            .any(|message| {
                matches!(
                    message,
                    ca::AgentMessage::Assistant {
                        content,
                        stop_reason: ca::StopReason::EndTurn,
                        ..
                    } if !content.plain_text().trim().is_empty()
                        && content.tool_calls().is_empty()
                )
            })
    }

    /// Take everything observed so far, leaving the tracker empty. The
    /// engine's overflow recovery folds progress into the session transcript
    /// mid-run; draining (instead of snapshotting) means a later fold after
    /// the retry can't duplicate the same messages.
    pub fn drain(&self) -> Vec<ca::AgentMessage> {
        std::mem::take(&mut *self.messages.lock().expect("run transcript lock"))
    }
}
