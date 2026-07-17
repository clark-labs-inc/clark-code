//! Translation between the typed Clark agent protocol and desktop/wire records.

use agent_core::domain as desktop;
use agent_core::ids::ToolCallId;
use clark_agent as ca;
use serde_json::{json, Value};

use crate::llm::{
    AssistantTurn, ChatContent, ChatMessage, ContentPart, ImageUrlRef, LlmError, ToolSchema,
    WireToolCall,
};

pub(super) fn to_wire_messages(
    system_prompt: &str,
    messages: &[ca::AgentMessage],
) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    if !system_prompt.trim().is_empty() {
        out.push(ChatMessage::system(system_prompt));
    }
    for message in messages {
        match message {
            ca::AgentMessage::System { content, .. } => {
                out.push(ChatMessage::system(content.clone()));
            }
            ca::AgentMessage::User { content, .. } => {
                out.push(user_chat_message(content));
            }
            ca::AgentMessage::Assistant { content, .. } => {
                let text = content.plain_text();
                out.push(ChatMessage {
                    role: "assistant".into(),
                    content: (!text.is_empty()).then(|| ChatContent::text(text)),
                    tool_calls: content
                        .tool_calls()
                        .into_iter()
                        .map(to_wire_tool_call)
                        .collect(),
                    tool_call_id: None,
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
            ca::AgentMessage::Custom { kind, payload, .. } => {
                out.push(ChatMessage::system(format!(
                    "[runtime context: {kind}]\n{}",
                    payload
                )));
            }
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
    ca::AgentMessage::Assistant {
        content: ca::AssistantContent::with_tool_calls(Some(turn.text), tool_calls),
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
        LlmError::Message(message) => (ca::stream::StreamErrorKind::Fatal, message),
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

pub(super) fn tool_title(name: &str, args: &Value) -> String {
    match name {
        "propose_plan" => return "Proposed a plan".to_string(),
        "update_plan" => return "Updated the plan".to_string(),
        "organization_knowledge" => return "Searched organization knowledge".to_string(),
        _ => {}
    }
    let salient = ["path", "pattern", "command", "query", "old_string"]
        .iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str));
    match salient {
        Some(value) => {
            let snippet: String = value
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect();
            format!("{name}: {snippet}")
        }
        None => name.to_string(),
    }
}
