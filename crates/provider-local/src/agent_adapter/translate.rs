//! Translation between the typed Clark agent protocol and desktop/wire records.

use agent_core::domain as desktop;
use agent_core::ids::ToolCallId;
use clark_agent as ca;
use serde_json::{json, Value};

use crate::llm::{
    AssistantTurn, ChatContent, ChatMessage, ContentPart, ImageUrlRef, LlmError, ToolSchema,
    WireToolCall,
};
use crate::tools::ImageAttachment;

const DESKTOP_IMAGES_DETAIL_KEY: &str = "_clark_desktop_images";

/// Build the wire `ChatMessage` list for a model call from the session's
/// system prompt + typed transcript. Shared by the live stream adapter and the
/// `/btw` side-question fork so both send a byte-identical prefix.
pub(crate) fn to_wire_messages(
    system_prompt: &str,
    messages: &[ca::AgentMessage],
) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    if !system_prompt.trim().is_empty() {
        out.push(ChatMessage::system(system_prompt));
    }
    // Reasoning replays only for the in-flight exchange: assistant messages
    // AFTER the latest user message (the current turn's tool loop, per the
    // OpenRouter contract for GLM/Kimi reasoning models). Older reasoning is
    // display history — replaying it across turns would balloon every prompt.
    let last_user = messages
        .iter()
        .rposition(|m| matches!(m, ca::AgentMessage::User { .. }));
    for (index, message) in messages.iter().enumerate() {
        match message {
            ca::AgentMessage::System { content, .. } => {
                out.push(ChatMessage::system(content.clone()));
            }
            ca::AgentMessage::User { content, .. } => {
                out.push(user_chat_message(content));
            }
            ca::AgentMessage::Assistant { content, .. } => {
                let text = content.plain_text();
                let in_flight = last_user.is_none_or(|user| index > user);
                out.push(ChatMessage {
                    role: "assistant".into(),
                    content: (!text.is_empty()).then(|| ChatContent::text(text)),
                    tool_calls: content
                        .tool_calls()
                        .into_iter()
                        .map(to_wire_tool_call)
                        .collect(),
                    tool_call_id: None,
                    reasoning: in_flight.then(|| reasoning_text(content)).flatten(),
                });
            }
            ca::AgentMessage::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                out.push(ChatMessage::tool(
                    tool_call_id.clone(),
                    content.plain_text(),
                ));
                // `role: "tool"` can't carry a content-parts array on the
                // OpenAI-compatible wire format, so an image result rides in
                // as a synthetic follow-up user turn instead — the standard
                // workaround for tool-result images on this wire format.
                // Purely a wire-time construct: it's re-derived fresh from
                // `content.blocks` on every turn, never written back into
                // `ca::AgentMessage` history, so nothing is duplicated across
                // resume/replay.
                let image_urls: Vec<String> = content
                    .blocks
                    .iter()
                    .filter_map(|block| match block {
                        ca::ToolResultBlock::Image(image) => Some(image.source.clone()),
                        _ => None,
                    })
                    .collect();
                if !image_urls.is_empty() {
                    out.push(ChatMessage::user_with_images(
                        format!("Image result from tool call {tool_call_id}:"),
                        image_urls,
                    ));
                }
            }
            ca::AgentMessage::Custom { kind, payload, .. }
                if kind == crate::planning::DEVELOPER_INSTRUCTION_MESSAGE_KIND =>
            {
                if let Some(content) = payload.get("content").and_then(Value::as_str) {
                    out.push(ChatMessage::developer(content));
                }
            }
            ca::AgentMessage::Custom { kind, payload, .. } => out.push(ChatMessage::system(
                format!("[runtime context: {kind}]\n{payload}"),
            )),
        }
    }
    out
}
/// Build the wire `user` message for a turn, forwarding any attached images
/// as content-parts (the OpenAI-compatible wire format allows parts arrays
/// on `role: "user"`, unlike `role: "tool"`). Falls back to a plain string
/// when there are no images, so the wire payload is byte-identical to before
/// multimodal support existed.
pub(super) fn user_chat_message(content: &ca::UserContent) -> ChatMessage {
    let blocks: &[ca::UserBlock] = match content {
        ca::UserContent::Text(text) => return ChatMessage::user(text.clone()),
        ca::UserContent::Blocks(blocks) => blocks,
    };
    let has_image = blocks
        .iter()
        .any(|block| matches!(block, ca::UserBlock::Image(_)));
    if !has_image {
        let text = blocks
            .iter()
            .filter_map(|block| match block {
                ca::UserBlock::Text(text) => Some(text.text.clone()),
                ca::UserBlock::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return ChatMessage::user(text);
    }
    let mut parts = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block {
            ca::UserBlock::Text(text) => parts.push(ContentPart::Text {
                text: text.text.clone(),
            }),
            ca::UserBlock::Image(image) => parts.push(ContentPart::ImageUrl {
                image_url: ImageUrlRef {
                    url: image.source.clone(),
                },
            }),
        }
    }
    ChatMessage {
        role: "user".into(),
        content: Some(ChatContent::Parts(parts)),
        tool_calls: Vec::new(),
        tool_call_id: None,
        reasoning: None,
    }
}

pub(super) fn to_wire_tool_schema(tool: &ca::stream::ToolSchema) -> ToolSchema {
    ToolSchema::function(
        tool.name.clone(),
        tool.description.clone(),
        tool.parameters.clone(),
    )
}

pub(super) fn to_wire_tool_call(call: &ca::ToolCall) -> WireToolCall {
    let args = serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string());
    WireToolCall::function(call.id.clone(), call.name.clone(), args)
}

pub(super) fn assistant_message(turn: AssistantTurn) -> ca::AgentMessage {
    let tool_calls = turn
        .tool_calls
        .iter()
        .map(to_core_tool_call)
        .collect::<Vec<_>>();
    let stop_reason = if tool_calls.is_empty() {
        stop_reason_from_finish(turn.finish_reason.as_deref())
    } else {
        ca::StopReason::ToolUse
    };
    let mut content = ca::AssistantContent::with_tool_calls(Some(turn.text), tool_calls);
    // Keep provider-native reasoning as a typed block: `plain_text()` skips
    // it (so it never leaks into visible content or compaction rendering),
    // and `to_wire_messages` replays it for the current tool exchange only.
    if !turn.reasoning.trim().is_empty() {
        content.blocks.insert(
            0,
            ca::AssistantBlock::Reasoning(ca::TextContent {
                text: turn.reasoning,
            }),
        );
    }
    ca::AgentMessage::Assistant {
        content,
        stop_reason,
        error_message: None,
        timestamp: None,
        usage: turn.usage.map(|u| ca::types::Usage {
            input_tokens: u.prompt_tokens as i64,
            output_tokens: u.completion_tokens as i64,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }),
    }
}

/// Concatenated provider-native reasoning blocks of one assistant message,
/// `None` when it has none.
fn reasoning_text(content: &ca::AssistantContent) -> Option<String> {
    let text = content
        .blocks
        .iter()
        .filter_map(|block| match block {
            ca::AssistantBlock::Reasoning(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
}

pub(super) fn to_core_tool_call(call: &WireToolCall) -> ca::ToolCall {
    ca::ToolCall {
        id: call.id.clone(),
        name: call.function.name.clone(),
        arguments: parse_tool_args(&call.function.arguments),
    }
}

pub(super) fn parse_tool_args(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(error) => ca::arg_parse_error_value(error.to_string(), raw),
    }
}

pub(super) fn stop_reason_from_finish(reason: Option<&str>) -> ca::StopReason {
    match reason {
        Some("length") => ca::StopReason::MaxTokens,
        Some("stop") | None => ca::StopReason::EndTurn,
        Some(_) => ca::StopReason::Other,
    }
}

pub(super) fn empty_assistant(
    stop_reason: ca::StopReason,
    error_message: Option<String>,
) -> ca::AgentMessage {
    ca::AgentMessage::Assistant {
        content: ca::AssistantContent { blocks: Vec::new() },
        stop_reason,
        error_message,
        timestamp: None,
        usage: None,
    }
}

pub(super) fn stream_error(error: LlmError) -> (ca::stream::StreamErrorKind, String) {
    match error {
        LlmError::Cancelled => (
            ca::stream::StreamErrorKind::Aborted,
            "model request cancelled".to_string(),
        ),
        LlmError::InsufficientCredits => (
            ca::stream::StreamErrorKind::Fatal,
            "insufficient_credits: You're out of Clark credits. Add credits to keep coding."
                .to_string(),
        ),
        LlmError::PlatformKeyRejected(message) => (
            ca::stream::StreamErrorKind::Fatal,
            format!("platform_key_rejected:{message}"),
        ),
        LlmError::RateLimited(message) => {
            (ca::stream::StreamErrorKind::ProviderRateLimited, message)
        }
        LlmError::Transport(message) => (ca::stream::StreamErrorKind::Transient, message),
        LlmError::Provider(message) => (
            ca::stream::StreamErrorKind::Fatal,
            format!("provider_error:{message}"),
        ),
        // Typed so the engine's overflow recovery (force-compact + retry the
        // turn) can catch it instead of failing the run outright.
        LlmError::ContextOverflow(message) => {
            (ca::stream::StreamErrorKind::ContextOverflow, message)
        }
    }
}

pub(super) fn tool_result_blocks_to_content(
    blocks: &[ca::ToolResultBlock],
) -> Vec<desktop::ContentBlock> {
    blocks
        .iter()
        .map(|block| match block {
            ca::ToolResultBlock::Text(text) => desktop::ContentBlock::text(text.text.clone()),
            ca::ToolResultBlock::Image(image) => match parse_data_url(&image.source) {
                Some((mime_type, data)) => desktop::ContentBlock::Image {
                    mime_type,
                    data,
                    uri: None,
                },
                None => desktop::ContentBlock::Image {
                    mime_type: image
                        .media_type
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                    data: String::new(),
                    uri: Some(image.source.clone()),
                },
            },
        })
        .collect()
}

/// Store UI-only image results in tool metadata when the active coding model
/// cannot accept image content parts. `ToolResult.details` is explicitly
/// excluded from model context by clark-agent, while the event sink can still
/// reconstruct the typed desktop image blocks from this structured field.
pub(super) fn store_tool_images(details: &mut Value, images: &[ImageAttachment]) {
    if !details.is_object() {
        *details = json!({});
    }
    let encoded = serde_json::to_value(images).unwrap_or(Value::Array(Vec::new()));
    details[DESKTOP_IMAGES_DETAIL_KEY] = encoded;
}

pub(super) fn tool_result_to_content(result: &ca::ToolResult) -> Vec<desktop::ContentBlock> {
    let mut content = tool_result_blocks_to_content(&result.content);
    let images = result
        .details
        .get(DESKTOP_IMAGES_DETAIL_KEY)
        .and_then(|value| serde_json::from_value::<Vec<ImageAttachment>>(value.clone()).ok())
        .unwrap_or_default();
    content.extend(
        images
            .into_iter()
            .map(|image| desktop::ContentBlock::Image {
                mime_type: image.mime_type,
                data: image.data_base64,
                uri: None,
            }),
    );
    content
}

/// Split a `data:{mime};base64,{data}` URL into its `(mime_type, data)` parts.
/// Returns `None` for anything else (e.g. an external `https://` URL), which
/// callers treat as a URI-only image reference instead.
pub(super) fn parse_data_url(s: &str) -> Option<(String, String)> {
    let rest = s.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime.to_string(), data.to_string()))
}

pub(super) fn locations_from_details(details: &Value) -> Vec<desktop::FsLocation> {
    details
        .get("locations")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// Build an inline Markdown artifact for a file the agent wrote into the document
/// workspace. Returns `None` for anything that isn't an existing Markdown file
/// inside `docs`. The `uri` is the canonical absolute path so the host can read
/// it back for the inline viewer.
pub(super) fn markdown_artifact(
    written: &str,
    tool_call_id: &str,
    docs: &std::path::Path,
) -> Option<desktop::Artifact> {
    let path = std::path::Path::new(written);
    if !path.is_absolute() {
        return None;
    }
    let canon = path.canonicalize().ok()?;
    if !canon.starts_with(docs) || !crate::workspace::is_markdown(&canon) {
        return None;
    }
    let title = canon
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document.md")
        .to_string();
    let uri = canon.to_string_lossy().to_string();
    Some(desktop::Artifact {
        id: format!("doc:{uri}"),
        title,
        kind: desktop::ArtifactKind::File,
        mime_type: Some("text/markdown".to_string()),
        uri: Some(uri),
        tool_call: Some(ToolCallId::new(tool_call_id.to_string())),
    })
}

/// Build an inline image artifact for a screenshot a mobile-control tool
/// wrote into the document workspace. Same shape as `markdown_artifact`,
/// gated on image extension instead of `is_markdown`.
pub(super) fn mobile_screenshot_artifact(
    written: &str,
    tool_call_id: &str,
    docs: &std::path::Path,
) -> Option<desktop::Artifact> {
    let path = std::path::Path::new(written);
    if !path.is_absolute() {
        return None;
    }
    let canon = path.canonicalize().ok()?;
    if !canon.starts_with(docs) {
        return None;
    }
    let ext = canon
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    let mime_type = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return None,
    };
    let title = canon
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("screenshot")
        .to_string();
    let uri = canon.to_string_lossy().to_string();
    Some(desktop::Artifact {
        id: format!("shot:{uri}"),
        title,
        kind: desktop::ArtifactKind::Image,
        mime_type: Some(mime_type.to_string()),
        uri: Some(uri),
        tool_call: Some(ToolCallId::new(tool_call_id.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(text: &str, reasoning: &str) -> AssistantTurn {
        AssistantTurn {
            text: text.to_string(),
            tool_calls: Vec::new(),
            finish_reason: Some("stop".into()),
            usage: None,
            reasoning: reasoning.to_string(),
        }
    }

    #[test]
    fn assistant_message_keeps_reasoning_as_typed_block_out_of_plain_text() {
        let message = assistant_message(turn("the answer", "step by step thinking"));
        let ca::AgentMessage::Assistant { content, .. } = &message else {
            panic!("expected assistant message");
        };
        // Reasoning is preserved as a typed block…
        assert_eq!(
            reasoning_text(content).as_deref(),
            Some("step by step thinking")
        );
        // …but never leaks into the visible text (compaction and the wire
        // `content` field both read plain_text()).
        assert_eq!(content.plain_text(), "the answer");
    }

    #[test]
    fn reasoning_replays_only_for_the_in_flight_exchange() {
        let old_assistant = assistant_message(turn("old turn", "old reasoning"));
        let user = ca::AgentMessage::User {
            content: ca::UserContent::Text("new question".into()),
            timestamp: None,
        };
        let live_assistant = assistant_message(turn("working on it", "live reasoning"));

        let wire = to_wire_messages("sys", &[old_assistant, user, live_assistant]);

        // [system, old assistant, user, live assistant]
        assert_eq!(wire.len(), 4);
        assert_eq!(
            wire[1].reasoning, None,
            "reasoning from before the last user message must not replay"
        );
        assert_eq!(wire[3].reasoning.as_deref(), Some("live reasoning"));
    }

    #[test]
    fn collaboration_instruction_uses_developer_role_on_the_wire() {
        let messages = [crate::planning::developer_instruction_message(
            "Plan Mode is active".into(),
        )];
        let wire = to_wire_messages("system", &messages);
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0].role, "system");
        assert_eq!(wire[1].role, "developer");
        assert!(matches!(
            &wire[1].content,
            Some(ChatContent::Text(text)) if text == "Plan Mode is active"
        ));
    }

    #[test]
    fn typed_llm_failures_map_to_typed_stream_errors() {
        let (kind, _) = stream_error(LlmError::ContextOverflow("too large".into()));
        assert!(matches!(kind, ca::stream::StreamErrorKind::ContextOverflow));

        let (kind, _) = stream_error(LlmError::Transport("connection reset".into()));
        assert!(matches!(kind, ca::stream::StreamErrorKind::Transient));

        let (kind, _) = stream_error(LlmError::RateLimited("busy".into()));
        assert!(matches!(
            kind,
            ca::stream::StreamErrorKind::ProviderRateLimited
        ));

        let (kind, message) =
            stream_error(LlmError::PlatformKeyRejected("401 Unauthorized".into()));
        assert!(matches!(kind, ca::stream::StreamErrorKind::Fatal));
        assert!(message.starts_with("platform_key_rejected:"));
    }
}
