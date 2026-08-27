use agent_core::domain as desktop;
use agent_loop as ca;
use serde_json::Value;

pub(super) fn readable_details(delta: Vec<Value>) -> Vec<desktop::ContentBlock> {
    let details = ca::ReasoningDetailsContent::new(delta);
    details
        .as_items()
        .into_iter()
        .filter_map(|item| match item {
            ca::ReasoningItem::Text { text, .. } => Some(text),
            ca::ReasoningItem::Summary { summary, .. } => Some(summary),
            ca::ReasoningItem::Encrypted { .. } => None,
        })
        .filter(|text| !text.is_empty())
        .map(desktop::ContentBlock::thinking)
        .collect()
}
