//! Shared state for provider text and tool-call callbacks.

use std::sync::{Arc, Mutex};

use agent_loop as ca;
use tokio::sync::mpsc::UnboundedSender;

use crate::llm::WireToolCallDelta;

use super::proposed_plan_stream::ProposedPlanStreamFilter;

#[derive(Clone)]
pub(super) struct StreamProgress {
    held_text: Arc<Mutex<Option<String>>>,
    proposal_filter: Arc<Mutex<ProposedPlanStreamFilter>>,
}

impl StreamProgress {
    pub(super) fn new(hold_completion_text: bool) -> Self {
        Self {
            held_text: Arc::new(Mutex::new(hold_completion_text.then(String::new))),
            proposal_filter: Arc::new(Mutex::new(ProposedPlanStreamFilter::default())),
        }
    }

    pub(super) fn observe_text(&self, tx: &UnboundedSender<ca::StreamEvent>, delta: &str) {
        let visible = {
            let mut held = self.held_text.lock().expect("completion text lock");
            match held.as_mut() {
                Some(text) => {
                    text.push_str(delta);
                    None
                }
                None => Some(delta.to_string()),
            }
        };
        if let Some(visible) = visible {
            self.emit_text(tx, visible);
        }
    }

    pub(super) fn observe_tool(
        &self,
        tx: &UnboundedSender<ca::StreamEvent>,
        delta: WireToolCallDelta,
    ) {
        // A real tool call makes its companion text commentary. Only a
        // natural final answer waits for the effect-completion guard.
        let held = self.held_text.lock().expect("completion text lock").take();
        if let Some(text) = held {
            self.emit_text(tx, text);
        }
        let _ = tx.send(ca::StreamEvent::Chunk(
            ca::AssistantStreamChunk::ToolCallDelta {
                index: delta.index,
                id_delta: delta.id_delta,
                name_delta: delta.name_delta,
                arguments_delta: delta.arguments_delta,
            },
        ));
    }

    pub(super) fn finish_filter(&self, tx: &UnboundedSender<ca::StreamEvent>) {
        let visible = self
            .proposal_filter
            .lock()
            .map(|mut filter| filter.finish())
            .unwrap_or_default();
        if !visible.is_empty() {
            let _ = tx.send(ca::StreamEvent::Chunk(ca::AssistantStreamChunk::Text {
                delta: visible,
            }));
        }
    }

    fn emit_text(&self, tx: &UnboundedSender<ca::StreamEvent>, visible: String) {
        let visible = self
            .proposal_filter
            .lock()
            .map(|mut filter| filter.feed(&visible))
            .unwrap_or(visible);
        if !visible.is_empty() {
            let _ = tx.send(ca::StreamEvent::Chunk(ca::AssistantStreamChunk::Text {
                delta: visible,
            }));
        }
    }
}
