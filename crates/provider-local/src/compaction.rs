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

impl CheckpointCompactor {
    /// Whether the provider's own accounting says the prompt has crossed the
    /// threshold. The char/4 heuristic under-counts structured transcripts
    /// (JSON tool args, code); the `input_tokens` the provider reported for
    /// the previous call is ground truth for what the next one will cost.
    fn usage_over_limit(&self, cx: &ca::TransformContext<'_>) -> bool {
        self.config.enabled()
            && cx.last_provider_usage.is_some_and(|usage| {
                usage.input_tokens.max(0) as usize >= self.config.auto_compact_token_limit
            })
    }
}

#[async_trait]
impl ca::ContextTransform for CheckpointCompactor {
    fn should_run(&self, messages: &[ca::AgentMessage], cx: &ca::TransformContext<'_>) -> bool {
        if self.usage_over_limit(cx) {
            return true;
        }
        let views = message_views(messages);
        core::should_compact(&views, &self.config, &core::CharHeuristic)
    }

    async fn transform(
        &self,
        messages: Vec<ca::AgentMessage>,
        cx: &ca::TransformContext<'_>,
    ) -> Vec<ca::AgentMessage> {
        // When real provider usage crossed the limit but the char heuristic
        // hasn't, force the pass — `prepare_compaction` re-checks the
        // heuristic internally and would otherwise no-op forever.
        let config = if self.usage_over_limit(cx) {
            forced(&self.config)
        } else {
            self.config.clone()
        };
        match compact_once(&self.llm, &config, &messages, cx.signal).await {
            Some(next) => next,
            None => messages,
        }
    }
}

/// The auto-compaction threshold in tokens, `None` when compaction is
/// disabled — the number the UI's context meter should measure against.
pub(crate) fn limit_of(config: &CompactionConfig) -> Option<u64> {
    config
        .enabled()
        .then_some(config.auto_compact_token_limit as u64)
}

/// A config whose threshold always fires, keeping the other budgets intact.
fn forced(config: &CompactionConfig) -> CompactionConfig {
    CompactionConfig {
        auto_compact_token_limit: 1,
        ..config.clone()
    }
}

/// One checkpoint-compaction pass over `messages`: summarize via the LLM
/// (one retry on a transient failure) and rebuild the transcript as
/// `[summary] + recent user tail`. `None` = nothing to do or the LLM failed —
/// callers keep the original messages (fail-open).
pub(crate) async fn compact_once(
    llm: &LlmClient,
    config: &CompactionConfig,
    messages: &[ca::AgentMessage],
    signal: &tokio_util::sync::CancellationToken,
) -> Option<Vec<ca::AgentMessage>> {
    let views = message_views(messages);
    let prepared = core::prepare_compaction(&views, config, &core::CharHeuristic)?;

    let mut summary = match llm.complete(None, &prepared.request.prompt, signal).await {
        Ok(summary) => summary,
        Err(_) if !signal.is_cancelled() => {
            // One retry: compaction failing means the run dies at the window
            // edge later, so a transient summarizer hiccup is worth absorbing.
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            llm.complete(None, &prepared.request.prompt, signal)
                .await
                .ok()?
        }
        Err(_) => return None,
    };

    if prepared.request.omitted_messages > 0 {
        summary.push_str(&format!(
            "\n\nNote: {omitted} older message(s) were omitted before compaction because the transcript was already near the context limit.",
            omitted = prepared.request.omitted_messages
        ));
    }
    // The summary is a point-in-time snapshot that outlives the files it
    // describes — other agents share this tree, so beliefs formed from it
    // go stale. Stamp it the way resume-context already is.
    summary.push_str(
        "\n\n[Point-in-time summary: files and code described above may have changed since \
this was written — re-read a file before relying on its described contents.]",
    );

    let compacted = core::finalize_compaction(&prepared.plan, &summary);
    let mut next = vec![user_message(compacted.summary_message)];
    next.extend(compacted.recent_user_messages.into_iter().map(user_message));
    Some(next)
}

/// Force a compaction pass regardless of the configured threshold — the
/// engine's context-overflow recovery: the provider just rejected the prompt,
/// so the transcript must shrink for the retry to have any chance.
pub(crate) async fn force_compact(
    llm: &LlmClient,
    config: &CompactionConfig,
    messages: &[ca::AgentMessage],
    signal: &tokio_util::sync::CancellationToken,
) -> Option<Vec<ca::AgentMessage>> {
    compact_once(llm, &forced(config), messages, signal).await
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

#[cfg(test)]
mod usage_trigger_tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn compactor(limit: usize) -> CheckpointCompactor {
        let provider_config = agent_core::provider::ProviderConfig {
            auth_token: Some("k".into()),
            extra: serde_json::json!({"base_url": "http://127.0.0.1:1/v1", "model": "m"}),
            ..Default::default()
        };
        let local = crate::config::LocalConfig::from_provider_config(&provider_config);
        let llm = crate::llm::LlmClient::new(&local).unwrap();
        CheckpointCompactor::new(
            llm,
            CompactionConfig {
                auto_compact_token_limit: limit,
                ..CompactionConfig::default()
            },
        )
    }

    #[test]
    fn provider_usage_triggers_compaction_before_the_char_heuristic() {
        use clark_agent::ContextTransform;
        let cancel = CancellationToken::new();
        // A tiny transcript the char heuristic would never flag…
        let messages = vec![user_message("short")];
        let over = ca::types::Usage {
            input_tokens: 2_000,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        let cx = ca::TransformContext {
            signal: &cancel,
            model_id: "m",
            iteration: 3,
            last_provider_usage: Some(&over),
            estimator: &ca::CHAR_HEURISTIC,
        };
        // …but the provider says the real prompt already crossed the limit.
        assert!(compactor(1_000).should_run(&messages, &cx));

        let under = ca::types::Usage {
            input_tokens: 10,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        let cx_under = ca::TransformContext {
            signal: &cancel,
            model_id: "m",
            iteration: 3,
            last_provider_usage: Some(&under),
            estimator: &ca::CHAR_HEURISTIC,
        };
        assert!(!compactor(1_000).should_run(&messages, &cx_under));
    }
}
