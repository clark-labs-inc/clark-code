use agent_core::domain as desktop;
use agent_core::ids::RunId;
use agent_loop as ca;
use async_channel::Sender;
use serde_json::Value;

pub(super) async fn emit_details(
    events: &Sender<desktop::AgentEvent>,
    run: &RunId,
    delta: Vec<Value>,
) {
    let details = ca::ReasoningDetailsContent::new(delta);
    for item in details.as_items() {
        let readable = match item {
            ca::ReasoningItem::Text { text, .. } => Some(text),
            ca::ReasoningItem::Summary { summary, .. } => Some(summary),
            ca::ReasoningItem::Encrypted { .. } => None,
        };
        if let Some(readable) = readable.filter(|text| !text.is_empty()) {
            let _ = events
                .send(desktop::AgentEvent::MessageChunk {
                    run: run.clone(),
                    role: desktop::Role::Agent,
                    delta: desktop::ContentBlock::thinking(readable),
                })
                .await;
        }
    }
}
