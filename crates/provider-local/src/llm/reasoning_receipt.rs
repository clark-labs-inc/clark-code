//! Private-safe provider reasoning capture and replay receipts.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::ChatMessage;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningPayloadReceipt {
    pub payload_sha256: String,
    pub reasoning_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_sha256: Option<String>,
    pub reasoning_details_items: usize,
    pub reasoning_details_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_details_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningReplayReceipt {
    pub message_index: usize,
    #[serde(flatten)]
    pub payload: ReasoningPayloadReceipt,
}

pub(super) fn summarize_payload(
    reasoning: Option<&str>,
    reasoning_details: &[Value],
) -> Option<ReasoningPayloadReceipt> {
    let reasoning = reasoning.filter(|value| !value.is_empty());
    if reasoning.is_none() && reasoning_details.is_empty() {
        return None;
    }
    let reasoning_sha256 = reasoning.map(|value| sha256(value.as_bytes()));
    let details_bytes = (!reasoning_details.is_empty())
        .then(|| serde_json::to_vec(reasoning_details).unwrap_or_default());
    let reasoning_details_sha256 = details_bytes.as_deref().map(sha256);

    // This mirrors the wire serializer: structured details are authoritative;
    // plain reasoning is used only when the provider returned no details.
    let mut replay = Map::new();
    if reasoning_details.is_empty() {
        if let Some(reasoning) = reasoning {
            replay.insert(
                "reasoning".to_string(),
                Value::String(reasoning.to_string()),
            );
        }
    } else {
        replay.insert(
            "reasoning_details".to_string(),
            Value::Array(reasoning_details.to_vec()),
        );
    }
    let payload_sha256 = sha256(&serde_json::to_vec(&Value::Object(replay)).unwrap_or_default());
    Some(ReasoningPayloadReceipt {
        payload_sha256,
        reasoning_bytes: reasoning.map(str::len).unwrap_or(0),
        reasoning_sha256,
        reasoning_details_items: reasoning_details.len(),
        reasoning_details_bytes: details_bytes.as_ref().map(Vec::len).unwrap_or(0),
        reasoning_details_sha256,
    })
}

pub(super) fn summarize_replays(messages: &[ChatMessage]) -> Vec<ReasoningReplayReceipt> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == "assistant")
        .filter_map(|(message_index, message)| {
            summarize_payload(message.reasoning.as_deref(), &message.reasoning_details).map(
                |payload| ReasoningReplayReceipt {
                    message_index,
                    payload,
                },
            )
        })
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatContent;

    #[test]
    fn structured_details_are_the_authoritative_replay_payload() {
        let details = vec![serde_json::json!({
            "type": "reasoning.text",
            "text": "private",
            "index": 0
        })];
        let capture = summarize_payload(Some("also private"), &details).unwrap();
        let messages = [ChatMessage {
            role: "assistant".into(),
            content: Some(ChatContent::text("visible")),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning: None,
            reasoning_details: details,
        }];
        let replay = summarize_replays(&messages);
        assert_eq!(replay[0].payload.payload_sha256, capture.payload_sha256);
        assert_eq!(replay[0].payload.reasoning_details_items, 1);
        assert_eq!(replay[0].payload.reasoning_bytes, 0);
        assert!(!serde_json::to_string(&replay).unwrap().contains("private"));
    }
}
