use serde_json::Value;

pub(super) fn canonical_message_text(payload: &Value) -> Option<String> {
    if payload.get("type").and_then(Value::as_str) != Some("message_end") {
        return None;
    }
    let blocks = payload.get("message")?.get("content")?.as_array()?;
    Some(
        blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect(),
    )
}
