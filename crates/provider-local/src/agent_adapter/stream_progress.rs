//! Shared state for provider text and tool-call callbacks.

use std::sync::{Arc, Mutex};

use agent_loop as ca;
use tokio::sync::mpsc::UnboundedSender;

use crate::llm::WireToolCallDelta;

use super::{
    proposed_plan_stream::ProposedPlanStreamFilter, required_tool_text::RequiredToolText,
    tool_call_stream::ToolCallStreamGate,
};

#[derive(Clone)]
pub(super) struct StreamProgress {
    required_text: Arc<Mutex<RequiredToolText>>,
    proposal_filter: Arc<Mutex<ProposedPlanStreamFilter>>,
}

impl StreamProgress {
    pub(super) fn new(force_tool_call: bool) -> Self {
        Self {
            required_text: Arc::new(Mutex::new(RequiredToolText::new(force_tool_call))),
            proposal_filter: Arc::new(Mutex::new(ProposedPlanStreamFilter::default())),
        }
    }

    pub(super) fn observe_text(&self, tx: &UnboundedSender<ca::StreamEvent>, delta: &str) {
        let visible = self
            .required_text
            .lock()
            .ok()
            .and_then(|mut text| text.observe(delta));
        if let Some(visible) = visible {
            self.emit_text(tx, visible);
        }
    }

    pub(super) fn observe_tool(
        &self,
        tx: &UnboundedSender<ca::StreamEvent>,
        gate: &Mutex<ToolCallStreamGate>,
        delta: WireToolCallDelta,
    ) {
        let visible = gate
            .lock()
            .map(|mut gate| gate.observe(delta))
            .unwrap_or_default();
        if visible.ordinary_started {
            let text = self
                .required_text
                .lock()
                .ok()
                .and_then(|mut text| text.release_for_ordinary_tool());
            if let Some(text) = text {
                self.emit_text(tx, text);
            }
        }
        for delta in visible.deltas {
            let _ = tx.send(ca::StreamEvent::Chunk(
                ca::AssistantStreamChunk::ToolCallDelta {
                    index: delta.index,
                    id_delta: delta.id_delta,
                    name_delta: delta.name_delta,
                    arguments_delta: delta.arguments_delta,
                },
            ));
        }
    }

    pub(super) fn reset_attempt(&self) {
        if let Ok(mut text) = self.required_text.lock() {
            text.reset_attempt();
        }
    }

    pub(super) fn finish_ordinary_turn(
        &self,
        tx: &UnboundedSender<ca::StreamEvent>,
        complete_text: &str,
    ) {
        let visible = self
            .required_text
            .lock()
            .ok()
            .and_then(|mut text| text.finish_ordinary_turn(complete_text));
        if let Some(visible) = visible {
            self.emit_text(tx, visible);
        }
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
