//! Context checkpointing for the local agent loop.
//!
//! The UI transcript remains untouched. This only rewrites the model-visible
//! `clark_agent::AgentMessage` history through a core `ContextTransform`.

use async_trait::async_trait;
use clark_agent as ca;
use clark_agent_compaction as core;
use serde_json::Value;

use crate::llm::LlmClient;

pub use core::CompactionConfig;

#[derive(Clone)]
pub(crate) struct CheckpointCompactor {
    llm: LlmClient,
    config: CompactionConfig,
}

impl CheckpointCompactor {
    pub fn new(llm: LlmClient, config: CompactionConfig) -> Self {
        Self { llm, config }
    }
}

impl ca::Plugin for CheckpointCompactor {
    fn name(&self) -> &'static str {
        "checkpoint_compactor"
    }

    fn capabilities(&self) -> ca::PluginCapabilities {
        ca::PluginCapabilities::context_transform()
    }
}

#[async_trait]
impl ca::ContextTransform for CheckpointCompactor {
    fn should_run(&self, messages: &[ca::AgentMessage], _cx: &ca::TransformContext<'_>) -> bool {
        let views = message_views(messages);
        core::should_compact(&views, &self.config, &core::CharHeuristic)
    }

    async fn transform(
        &self,
        messages: Vec<ca::AgentMessage>,
        cx: &ca::TransformContext<'_>,
    ) -> Vec<ca::AgentMessage> {
        let views = message_views(&messages);
        let Some(prepared) = core::prepare_compaction(&views, &self.config, &core::CharHeuristic)
        else {
            return messages;
        };

        let Ok(mut summary) = self
            .llm
            .complete(None, &prepared.request.prompt, cx.signal)
            .await
        else {
            return messages;
        };

        if prepared.request.omitted_messages > 0 {
            summary.push_str(&format!(
                "\n\nNote: {omitted} older message(s) were omitted before compaction because the transcript was already near the context limit.",
                omitted = prepared.request.omitted_messages
            ));
        }

        let compacted = core::finalize_compaction(&prepared.plan, &summary);
        let mut next = vec![user_message(compacted.summary_message)];
        next.extend(compacted.recent_user_messages.into_iter().map(user_message));
        next
    }
}

#[derive(Clone, Copy)]
struct AgentMessageView<'a>(&'a ca::AgentMessage);

fn message_views(messages: &[ca::AgentMessage]) -> Vec<AgentMessageView<'_>> {
    messages.iter().map(AgentMessageView).collect()
}

impl core::TranscriptMessage for AgentMessageView<'_> {
    fn render_for_compaction(&self, out: &mut String) {
        match self.0 {
            ca::AgentMessage::System { content, .. } => {
                out.push_str("[system]\n");
                out.push_str(content);
            }
            ca::AgentMessage::User { content, .. } => {
                out.push_str("[user]\n");
                render_user_content(content, out);
            }
            ca::AgentMessage::Assistant { content, .. } => render_assistant_content(content, out),
            ca::AgentMessage::ToolResult {
                tool_call_id,
                tool_name,
                content,
                is_error,
                ..
            } => {
                let status = if *is_error { "error" } else { "ok" };
                out.push_str("[tool result ");
                out.push_str(tool_call_id);
                out.push(' ');
                out.push_str(tool_name);
                out.push(' ');
                out.push_str(status);
                out.push_str("]\n");
                out.push_str(&content.plain_text());
            }
            ca::AgentMessage::Custom { kind, payload, .. } => {
                out.push_str("[custom ");
                out.push_str(kind);
                out.push_str("]\n");
                out.push_str(&compact_json_value(payload));
            }
        }
    }

    fn user_text_for_compaction(&self, out: &mut String) -> bool {
        let ca::AgentMessage::User { content, .. } = self.0 else {
            return false;
        };
        render_user_text(content, out);
        true
    }

    fn is_compaction_summary(&self, summary_prefix: &str) -> bool {
        let ca::AgentMessage::User { content, .. } = self.0 else {
            return false;
        };
        match content {
            ca::UserContent::Text(text) => text.starts_with(summary_prefix),
            ca::UserContent::Blocks(blocks) => blocks.iter().any(|block| match block {
                ca::UserBlock::Text(text) => text.text.starts_with(summary_prefix),
                ca::UserBlock::Image(_) => false,
            }),
        }
    }
}

fn user_message(text: impl Into<String>) -> ca::AgentMessage {
    ca::AgentMessage::User {
        content: ca::UserContent::Text(text.into()),
        timestamp: None,
    }
}

fn render_user_content(content: &ca::UserContent, out: &mut String) {
    match content {
        ca::UserContent::Text(text) => out.push_str(text),
        ca::UserContent::Blocks(blocks) => {
            for (idx, block) in blocks.iter().enumerate() {
                if idx > 0 {
                    out.push('\n');
                }
                match block {
                    ca::UserBlock::Text(text) => out.push_str(&text.text),
                    ca::UserBlock::Image(image) => {
                        out.push_str("[image: ");
                        out.push_str(image.alt.as_deref().unwrap_or("attached image"));
                        out.push(']');
                    }
                }
            }
        }
    }
}

fn render_user_text(content: &ca::UserContent, out: &mut String) {
    match content {
        ca::UserContent::Text(text) => out.push_str(text),
        ca::UserContent::Blocks(blocks) => {
            let mut wrote = false;
            for block in blocks {
                let ca::UserBlock::Text(text) = block else {
                    continue;
                };
                if wrote {
                    out.push('\n');
                }
                out.push_str(&text.text);
                wrote = true;
            }
        }
    }
}

fn render_assistant_content(content: &ca::AssistantContent, out: &mut String) {
    out.push_str("[assistant]\n");
    let mut wrote = false;

    let text = content.plain_text();
    if !text.is_empty() {
        out.push_str(&text);
        wrote = true;
    }

    let calls = content.tool_calls();
    if !calls.is_empty() {
        if wrote {
            out.push('\n');
        }
        out.push_str("tool calls: ");
        for (idx, call) in calls.into_iter().enumerate() {
            if idx > 0 {
                out.push_str(", ");
            }
            out.push_str(&call.name);
            out.push('(');
            out.push_str(&compact_json_value(&call.arguments));
            out.push(')');
        }
        wrote = true;
    }

    if !wrote {
        out.push_str("(empty)");
    }
}

fn compact_json_value(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clark_agent_compaction::TranscriptMessage;

    fn config() -> CompactionConfig {
        CompactionConfig {
            auto_compact_token_limit: 20,
            compact_request_token_limit: 1_000,
            recent_user_token_budget: 12,
            ..CompactionConfig::default()
        }
    }

    fn assistant_tool_call() -> ca::AgentMessage {
        ca::AgentMessage::Assistant {
            content: ca::AssistantContent::with_tool_calls(
                Some("reading".into()),
                vec![ca::ToolCall {
                    id: "call_1".into(),
                    name: "read_file".into(),
                    arguments: serde_json::json!({"path":"src/main.rs"}),
                }],
            ),
            stop_reason: ca::StopReason::ToolUse,
            error_message: None,
            timestamp: None,
            usage: None,
        }
    }

    fn tool_result() -> ca::AgentMessage {
        ca::AgentMessage::ToolResult {
            tool_call_id: "call_1".into(),
            tool_name: "read_file".into(),
            content: ca::ToolResultContent::text("fn main() {}"),
            is_error: false,
            narration: None,
            details: None,
            timestamp: None,
        }
    }

    fn rendered(message: &ca::AgentMessage) -> String {
        let mut out = String::new();
        AgentMessageView(message).render_for_compaction(&mut out);
        out
    }

    #[test]
    fn adapter_renders_tool_context_for_core_compaction() {
        let transcript = vec![
            user_message("please inspect the project"),
            assistant_tool_call(),
            tool_result(),
        ];
        let views = message_views(&transcript);

        let prepared = core::prepare_compaction(&views, &config(), &core::CharHeuristic)
            .expect("compaction request");

        assert!(prepared
            .request
            .prompt
            .contains(r#"read_file({"path":"src/main.rs"})"#));
        assert!(prepared
            .request
            .prompt
            .contains("[tool result call_1 read_file ok]"));
    }

    #[test]
    fn adapter_keeps_image_alt_in_render_but_not_recent_user_text() {
        let message = ca::AgentMessage::User {
            content: ca::UserContent::Blocks(vec![
                ca::UserBlock::Text(ca::TextContent {
                    text: "look".into(),
                }),
                ca::UserBlock::Image(ca::ImageContent {
                    source: "data:image/png;base64,abc".into(),
                    media_type: Some("image/png".into()),
                    alt: Some("diagram".into()),
                }),
            ]),
            timestamp: None,
        };

        assert!(rendered(&message).contains("[image: diagram]"));

        let mut text = String::new();
        assert!(AgentMessageView(&message).user_text_for_compaction(&mut text));
        assert_eq!(text, "look");
    }

    #[test]
    fn adapter_finalizes_summary_first_then_recent_user_tail() {
        let text = format!(
            "{}\nNow answer with CLARK_LIVE_COMPACTION_DONE_3003.",
            "Important project context. ".repeat(900)
        );
        let transcript = vec![user_message(text)];
        let views = message_views(&transcript);
        let prepared = core::prepare_compaction(&views, &config(), &core::CharHeuristic)
            .expect("compaction request");
        let compacted = core::finalize_compaction(&prepared.plan, "summary");

        let mut next = vec![user_message(compacted.summary_message)];
        next.extend(compacted.recent_user_messages.into_iter().map(user_message));

        let first = rendered(next.first().unwrap());
        assert!(first.contains(core::DEFAULT_SUMMARY_PREFIX));
        let last = rendered(next.last().unwrap());
        assert!(last.contains("CLARK_LIVE_COMPACTION_DONE_3003"));
    }
}
