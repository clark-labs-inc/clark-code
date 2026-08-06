//! Model-visible context checkpointing; the UI transcript remains untouched.

use agent_core::domain::{AgentEvent, ContentBlock, Role, RunFailureKind, RunOutcome, RunStatus};
use agent_core::ids::RunId;
use async_channel::Sender;
use async_trait::async_trait;
use clark_agent as ca;
use clark_agent_compaction as core;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

use crate::llm::LlmClient;
use crate::loop_state::SessionState;

mod reasoning;

pub use core::CompactionConfig;

#[derive(Clone)]
pub(crate) struct CheckpointCompactor {
    llm: LlmClient,
    config: CompactionConfig,
    checkpoint: Arc<Mutex<Option<ModelCheckpoint>>>,
}

#[derive(Clone)]
struct ModelCheckpoint {
    /// Raw-lineage length; later messages are new, uncheckpointed turns.
    source_len: usize,
    source_fingerprint: Option<[u8; 32]>,
    messages: Vec<ca::AgentMessage>,
}

impl CheckpointCompactor {
    pub fn new(llm: LlmClient, config: CompactionConfig) -> Self {
        Self {
            llm,
            config,
            checkpoint: Arc::new(Mutex::new(None)),
        }
    }

    /// clark-agent transforms a request-local clone rather than mutating its
    /// live context. Keep the replacement here and project it over the raw
    /// lineage on every later request; otherwise the same full transcript is
    /// summarized again on every turn.
    fn projected_messages(&self, messages: &[ca::AgentMessage]) -> (Vec<ca::AgentMessage>, bool) {
        let mut state = self.checkpoint.lock().expect("compaction checkpoint lock");
        let Some(checkpoint) = state.as_ref() else {
            return (messages.to_vec(), false);
        };
        if messages.len() < checkpoint.source_len
            || lineage_fingerprint(messages, checkpoint.source_len) != checkpoint.source_fingerprint
        {
            // Overflow recovery can replace the live lineage itself. A
            // checkpoint into the old lineage must not be spliced into it.
            *state = None;
            return (messages.to_vec(), false);
        }
        let mut projected = checkpoint.messages.clone();
        projected.extend_from_slice(&messages[checkpoint.source_len..]);
        (projected, true)
    }

    fn has_applicable_checkpoint(&self, messages: &[ca::AgentMessage]) -> bool {
        let mut state = self.checkpoint.lock().expect("compaction checkpoint lock");
        if state.as_ref().is_some_and(|checkpoint| {
            messages.len() < checkpoint.source_len
                || lineage_fingerprint(messages, checkpoint.source_len)
                    != checkpoint.source_fingerprint
        }) {
            *state = None;
        }
        state.is_some()
    }

    fn install_checkpoint(
        &self,
        source_len: usize,
        source_fingerprint: Option<[u8; 32]>,
        messages: Vec<ca::AgentMessage>,
    ) {
        *self.checkpoint.lock().expect("compaction checkpoint lock") = Some(ModelCheckpoint {
            source_len,
            source_fingerprint,
            messages,
        });
    }

    /// Atomically install the request-time checkpoint into canonical model
    /// history. It is consumed only for the exact lineage it summarized, so a
    /// concurrent or recovery mutation fails closed to `messages`.
    pub(crate) fn commit_checkpoint(
        &self,
        messages: Vec<ca::AgentMessage>,
    ) -> Vec<ca::AgentMessage> {
        let checkpoint = self
            .checkpoint
            .lock()
            .expect("compaction checkpoint lock")
            .take();
        let Some(checkpoint) = checkpoint else {
            return messages;
        };
        if messages.len() < checkpoint.source_len
            || lineage_fingerprint(&messages, checkpoint.source_len)
                != checkpoint.source_fingerprint
        {
            return messages;
        }
        let mut projected = checkpoint.messages;
        projected.extend_from_slice(&messages[checkpoint.source_len..]);
        projected
    }

    pub(crate) fn commit_appended(
        &self,
        transcript: &mut Vec<ca::AgentMessage>,
        messages: impl IntoIterator<Item = ca::AgentMessage>,
    ) {
        let mut combined = std::mem::take(transcript);
        combined.extend(messages);
        *transcript = self.commit_checkpoint(combined);
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
        // Applying the cached checkpoint is itself a transform. The upstream
        // context still contains the raw pre-checkpoint prefix, so skipping
        // here would expand the next request back to the full transcript.
        if self.has_applicable_checkpoint(messages) {
            return true;
        }
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
        let source_len = messages.len();
        let source_fingerprint = lineage_fingerprint(&messages, source_len);
        let (effective, _) = self.projected_messages(&messages);
        let effective_views = message_views(&effective);
        let should_checkpoint = self.usage_over_limit(cx)
            || core::should_compact(&effective_views, &self.config, &core::CharHeuristic);
        if !should_checkpoint {
            return effective;
        }

        // When real provider usage crossed the limit but the char heuristic
        // hasn't, force the pass — `prepare_compaction` re-checks the
        // heuristic internally and would otherwise no-op forever.
        let config = if self.usage_over_limit(cx) {
            forced(&self.config)
        } else {
            self.config.clone()
        };
        match compact_once(&self.llm, &config, &effective, cx.signal).await {
            Some(next) => {
                self.install_checkpoint(source_len, source_fingerprint, next.clone());
                next
            }
            // A failed refresh must keep the last working checkpoint applied;
            // expanding to the original history recreates the overflow loop.
            None => effective,
        }
    }
}

fn lineage_fingerprint(messages: &[ca::AgentMessage], prefix_len: usize) -> Option<[u8; 32]> {
    let prefix = messages.get(..prefix_len)?;
    let mut hasher = Sha256::new();
    for message in prefix {
        let encoded = serde_json::to_vec(message).ok()?;
        hasher.update(encoded.len().to_le_bytes());
        hasher.update(encoded);
    }
    Some(hasher.finalize().into())
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

/// Manual compaction stays available when automatic compaction is disabled.
/// `disabled()` uses `usize::MAX` for every budget, which is useful for the
/// automatic gate but unsafe as a request size; restore the library defaults
/// for those two request-shaping limits while forcing only the threshold.
fn manual(config: &CompactionConfig) -> CompactionConfig {
    let mut config = forced(config);
    if config.compact_request_token_limit == usize::MAX {
        config.compact_request_token_limit = core::DEFAULT_COMPACT_REQUEST_TOKEN_LIMIT;
    }
    if config.recent_user_token_budget == usize::MAX {
        config.recent_user_token_budget = core::DEFAULT_RECENT_USER_TOKEN_BUDGET;
    }
    config
}

/// One checkpoint-compaction pass over `messages`: summarize via the LLM
/// (one retry on a transient failure) and rebuild the transcript as
/// `[summary] + contiguous raw tail`. `None` = nothing to do or the LLM failed.
pub(crate) async fn compact_once(
    llm: &LlmClient,
    config: &CompactionConfig,
    messages: &[ca::AgentMessage],
    signal: &tokio_util::sync::CancellationToken,
) -> Option<Vec<ca::AgentMessage>> {
    let views = message_views(messages);
    let config = core::with_private_reasoning_summary_guidance(config);
    let prepared = prepare_complete_compaction(&views, &config)?;

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

    // The summary is a point-in-time snapshot that outlives the files it
    // describes — other agents share this tree, so beliefs formed from it
    // go stale. Stamp it the way resume-context already is.
    summary.push_str(
        "\n\n[Point-in-time summary: files and code described above may have changed since \
this was written — re-read a file before relying on its described contents.]",
    );

    let compacted = core::finalize_compaction(&prepared.plan, &summary);
    let mut next = vec![user_message(compacted.summary_message)];
    let raw_tail = recent_raw_tail(messages, config.recent_user_token_budget);
    if raw_tail.is_empty() {
        // A single oversized current user request cannot be retained as a raw
        // typed message within the budget. Preserve the core's truncated user
        // fallback rather than relying on the summary alone.
        next.extend(compacted.recent_user_messages.into_iter().map(user_message));
    } else {
        next.extend(raw_tail);
    }
    Some(next)
}

/// Keep a contiguous suffix so assistant tool calls retain their exact tool
/// results. Never begin at a tool result: OpenAI-compatible providers require
/// the preceding assistant tool-call message in the same request.
fn recent_raw_tail(messages: &[ca::AgentMessage], token_budget: usize) -> Vec<ca::AgentMessage> {
    if token_budget == 0 {
        return Vec::new();
    }

    let mut remaining = token_budget;
    let mut start = messages.len();
    while start > 0 {
        let view = AgentMessageView(&messages[start - 1]);
        let tokens = core::estimate_transcript_tokens(&[view], &core::CharHeuristic);
        if tokens > remaining {
            break;
        }
        remaining = remaining.saturating_sub(tokens);
        start -= 1;
    }
    while matches!(
        messages.get(start),
        Some(ca::AgentMessage::ToolResult { .. })
    ) {
        start += 1;
    }
    messages[start..].to_vec()
}

/// Force a compaction pass regardless of the configured threshold — the
/// context-overflow recovery path: the provider just rejected the prompt, so
/// the transcript must shrink for the retry to have any chance.
pub(crate) async fn force_compact(
    llm: &LlmClient,
    config: &CompactionConfig,
    messages: &[ca::AgentMessage],
    signal: &tokio_util::sync::CancellationToken,
) -> Option<Vec<ca::AgentMessage>> {
    compact_once(llm, &forced(config), messages, signal).await
}

/// Run an explicit, standalone compaction turn. The visible conversation is
/// left intact; only the provider's canonical model transcript is replaced.
/// A lineage check makes the replacement fail closed if anything mutated the
/// session while the summary request was in flight.
pub(crate) async fn run_manual_compaction(
    llm: LlmClient,
    config: CompactionConfig,
    session: Arc<tokio::sync::Mutex<SessionState>>,
    tx: Sender<AgentEvent>,
    run: RunId,
    signal: tokio_util::sync::CancellationToken,
) {
    let _ = tx.send(AgentEvent::RunStarted { run: run.clone() }).await;

    let source = session.lock().await.transcript.clone();
    let source_len = source.len();
    let source_fingerprint = lineage_fingerprint(&source, source_len);
    let Some(next) = compact_once(&llm, &manual(&config), &source, &signal).await else {
        if signal.is_cancelled() {
            finish_manual(&tx, &run, RunStatus::Cancelled, None, None).await;
        } else {
            let message = "Clark could not summarize this conversation. Your existing context was left unchanged.";
            let _ = tx
                .send(AgentEvent::Error {
                    code: "compaction_failed".into(),
                    message: message.into(),
                    run: Some(run.clone()),
                })
                .await;
            finish_manual(
                &tx,
                &run,
                RunStatus::Failed,
                Some(message.to_string()),
                Some(RunFailureKind::ProviderError),
            )
            .await;
        }
        return;
    };

    let replaced = {
        let mut state = session.lock().await;
        if state.transcript.len() != source_len
            || lineage_fingerprint(&state.transcript, source_len) != source_fingerprint
        {
            false
        } else {
            state.transcript = next.clone();
            true
        }
    };
    if !replaced {
        let message = "The conversation changed while context was being compacted, so Clark kept the newer context unchanged.";
        let _ = tx
            .send(AgentEvent::Error {
                code: "compaction_conflict".into(),
                message: message.into(),
                run: Some(run.clone()),
            })
            .await;
        finish_manual(
            &tx,
            &run,
            RunStatus::Failed,
            Some(message.to_string()),
            Some(RunFailureKind::LocalState),
        )
        .await;
        return;
    }

    let _ = tx
        .send(AgentEvent::Trace {
            run: Some(run.clone()),
            source: "clark_code_compaction".into(),
            payload: serde_json::json!({
                "trigger": "manual",
                "before_messages": source_len,
                "after_messages": next.len(),
                "before_estimated_tokens": core::estimate_transcript_tokens(
                    &message_views(&source),
                    &core::CharHeuristic,
                ),
                "after_estimated_tokens": core::estimate_transcript_tokens(
                    &message_views(&next),
                    &core::CharHeuristic,
                ),
            }),
        })
        .await;
    let _ = tx
        .send(AgentEvent::MessageChunk {
            run: run.clone(),
            role: Role::System,
            delta: ContentBlock::text(
                "Earlier turns were summarized to free context space for this conversation.",
            ),
        })
        .await;
    let _ = tx
        .send(AgentEvent::ContextCompacted {
            run: run.clone(),
            transcript: crate::resume::from_agent_messages(&next),
        })
        .await;
    finish_manual(&tx, &run, RunStatus::Done, Some("compacted".into()), None).await;
}

async fn finish_manual(
    tx: &Sender<AgentEvent>,
    run: &RunId,
    status: RunStatus,
    message: Option<String>,
    failure_kind: Option<RunFailureKind>,
) {
    let _ = tx
        .send(AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status,
                stop_reason: (status == RunStatus::Done).then_some("compacted".into()),
                error: message.filter(|_| status == RunStatus::Failed),
                failure_kind,
                usage: None,
                execution: None,
            },
        })
        .await;
    tx.close();
}

#[async_trait::async_trait]
impl ca::ContextOverflowRecovery for CheckpointCompactor {
    async fn recover(
        &self,
        messages: Vec<ca::AgentMessage>,
        cx: &ca::TransformContext<'_>,
    ) -> Vec<ca::AgentMessage> {
        let source_len = messages.len();
        let source_fingerprint = lineage_fingerprint(&messages, source_len);
        let (effective, _) = self.projected_messages(&messages);
        // Fail-open: if compaction can't run (LLM error, nothing to shrink),
        // return the input unchanged — the loop's no-progress guard then lets
        // the overflow surface instead of retrying against the same history.
        match force_compact(&self.llm, &self.config, &effective, cx.signal).await {
            Some(next) => {
                self.install_checkpoint(source_len, source_fingerprint, next.clone());
                next
            }
            None => messages,
        }
    }

    fn max_attempts(&self) -> u8 {
        2
    }

    fn name(&self) -> &'static str {
        "checkpoint_compactor"
    }
}

#[derive(Clone, Copy)]
struct AgentMessageView<'a>(&'a ca::AgentMessage);

fn message_views(messages: &[ca::AgentMessage]) -> Vec<AgentMessageView<'_>> {
    messages.iter().map(AgentMessageView).collect()
}

fn prepare_complete_compaction<'a>(
    views: &[AgentMessageView<'a>],
    config: &CompactionConfig,
) -> Option<core::PreparedCompaction> {
    // The compaction request replaces this history. Letting the helper's
    // independent request budget discard the oldest messages creates a
    // plausible-looking but incomplete checkpoint. The normal compaction
    // threshold already fires below the model's context limit, so render the
    // entire source transcript. If the provider still rejects that request,
    // keep the raw history and surface the context failure instead of silently
    // installing a partial summary.
    let mut complete = config.clone();
    complete.compact_request_token_limit = usize::MAX;
    let prepared = core::prepare_compaction(views, &complete, &core::CharHeuristic)?;
    (prepared.request.omitted_messages == 0).then_some(prepared)
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

    wrote |= reasoning::append_readable_findings(content, out);

    if !wrote {
        out.push_str("(empty)");
    }
}

fn compact_json_value(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
include!("compaction_tests.rs");
