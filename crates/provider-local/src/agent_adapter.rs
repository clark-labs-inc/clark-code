use std::sync::Arc;

use agent_core::domain as desktop;
use agent_core::ids::{RunId, SessionId, ToolCallId};
use async_channel::Sender;
use async_trait::async_trait;
use clark_agent as ca;
use futures::stream::BoxStream;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::llm::{AssistantTurn, ChatMessage, LlmClient, LlmError, ToolSchema, WireToolCall};
use crate::loop_state::{RunControl, SessionState};
use crate::permissions::{PermissionGate, PermissionOutcome};
use crate::tools::{ToolCtx, ToolExecutor, ToolRegistry};

/// Running token/cost totals across a run's model calls, shared between the
/// stream adapter (writer) and the engine (reads them into the run outcome).
#[derive(Default)]
pub(crate) struct UsageTotals {
    inner: std::sync::Mutex<agent_core::domain::RunUsage>,
    seen: std::sync::atomic::AtomicBool,
}

impl UsageTotals {
    fn add(&self, usage: crate::llm::TokenUsage) {
        let mut t = self.inner.lock().expect("usage totals lock");
        t.input_tokens += usage.prompt_tokens;
        t.output_tokens += usage.completion_tokens;
        // The latest call's prompt is the conversation's live context footprint.
        t.context_tokens = usage.prompt_tokens;
        if let Some(cost) = usage.cost_usd {
            t.cost_usd = Some(t.cost_usd.unwrap_or(0.0) + cost);
        }
        self.seen.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The accumulated usage, or `None` if no call reported any.
    pub fn snapshot(&self) -> Option<agent_core::domain::RunUsage> {
        if !self.seen.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        Some(*self.inner.lock().expect("usage totals lock"))
    }
}

#[derive(Clone)]
pub(crate) struct ClarkAgentStream {
    llm: LlmClient,
    totals: Arc<UsageTotals>,
}

impl ClarkAgentStream {
    pub fn new(llm: LlmClient) -> Self {
        Self {
            llm,
            totals: Arc::new(UsageTotals::default()),
        }
    }

    /// Handle the engine holds to fold totals into the run outcome.
    pub fn usage(&self) -> Arc<UsageTotals> {
        self.totals.clone()
    }
}

#[async_trait]
impl ca::StreamFn for ClarkAgentStream {
    async fn stream(
        &self,
        request: ca::StreamRequest,
        signal: CancellationToken,
    ) -> BoxStream<'static, ca::StreamEvent> {
        let llm = self.llm.clone();
        let totals = self.totals.clone();
        let messages = to_wire_messages(&request.system_prompt, &request.messages);
        let tools = request
            .tools
            .iter()
            .map(to_wire_tool_schema)
            .collect::<Vec<_>>();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            let _ = tx.send(ca::StreamEvent::Start {
                partial: empty_assistant(ca::StopReason::EndTurn, None),
            });
            let chunk_tx = tx.clone();
            let turn = llm
                .stream_chat(&messages, &tools, &signal, move |delta| {
                    let _ = chunk_tx.send(ca::StreamEvent::Chunk(ca::AssistantStreamChunk::Text {
                        delta: delta.to_string(),
                    }));
                })
                .await;

            match turn {
                Ok(turn) => {
                    if let Some(usage) = turn.usage {
                        totals.add(usage);
                    }
                    let message = assistant_message(turn);
                    let _ = tx.send(ca::StreamEvent::Done { message });
                }
                Err(error) => {
                    let (kind, message) = stream_error(error);
                    let _ = tx.send(ca::StreamEvent::Error {
                        partial: empty_assistant(ca::StopReason::Error, None),
                        kind,
                        message,
                    });
                }
            }
        });

        Box::pin(futures::stream::unfold(rx, |mut rx| async {
            rx.recv().await.map(|event| (event, rx))
        }))
    }
}

pub(crate) fn desktop_tool_registry(
    source: Arc<ToolRegistry>,
    ctx: ToolCtx,
    session: Arc<Mutex<SessionState>>,
    control: Arc<Mutex<RunControl>>,
    session_id: SessionId,
    events: Sender<desktop::AgentEvent>,
) -> ca::ToolRegistry {
    let mut registry = ca::ToolRegistry::new();
    let gate = PermissionGate::new(session, control, session_id, events);
    for exec in source.executors() {
        registry.register(Arc::new(DesktopToolAdapter {
            exec,
            ctx: ctx.clone(),
            gate: gate.clone(),
        }));
    }
    registry
}

struct DesktopToolAdapter {
    exec: Arc<dyn ToolExecutor>,
    ctx: ToolCtx,
    gate: PermissionGate,
}

#[async_trait]
impl ca::AgentTool for DesktopToolAdapter {
    fn name(&self) -> &str {
        self.exec.name()
    }

    fn description(&self) -> &str {
        self.exec.description()
    }

    fn parameters_schema(&self) -> Value {
        self.exec.parameters()
    }

    fn requires_exclusive_sandbox(&self) -> bool {
        self.exec.mutating()
    }

    async fn execute(
        &self,
        call_id: &str,
        args: Value,
        signal: CancellationToken,
        _update: ca::ToolUpdateSink,
    ) -> Result<ca::ToolResult, ca::ToolError> {
        let tool_id = ToolCallId::new(call_id.to_string());
        match self
            .gate
            .check(
                &tool_id,
                self.exec.name(),
                self.exec.as_ref(),
                &args,
                &self.ctx,
                &signal,
            )
            .await
        {
            PermissionOutcome::Allowed => {}
            PermissionOutcome::Denied(message) => return Ok(ca::ToolResult::error(message)),
            PermissionOutcome::Cancelled => return Err(ca::ToolError::Aborted),
        }

        if signal.is_cancelled() {
            return Err(ca::ToolError::Aborted);
        }

        let args = match args {
            Value::Null => json!({}),
            other => other,
        };
        let outcome = self.exec.invoke(args, &self.ctx).await;
        let mut result = if outcome.is_error {
            ca::ToolResult::error(outcome.content)
        } else {
            ca::ToolResult::text(outcome.content)
        };
        if !outcome.locations.is_empty() {
            result.details = json!({ "locations": outcome.locations });
        }
        Ok(result)
    }
}

pub(crate) struct DesktopEventSink {
    events: Sender<desktop::AgentEvent>,
    run: RunId,
    registry: Arc<ToolRegistry>,
    /// The app-managed document workspace (canonical), when this is a local
    /// session. Markdown files written here are surfaced as inline artifacts.
    docs_dir: Option<std::path::PathBuf>,
}

impl DesktopEventSink {
    pub fn new(
        events: Sender<desktop::AgentEvent>,
        run: RunId,
        registry: Arc<ToolRegistry>,
        docs_dir: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            events,
            run,
            registry,
            docs_dir,
        }
    }
}

#[async_trait]
impl ca::EventSink for DesktopEventSink {
    async fn emit(&self, event: ca::AgentEvent) {
        match event {
            ca::AgentEvent::MessageUpdate {
                chunk: ca::AssistantStreamChunk::Text { delta },
                ..
            } => {
                let _ = self
                    .events
                    .send(desktop::AgentEvent::MessageChunk {
                        run: self.run.clone(),
                        role: desktop::Role::Agent,
                        delta: desktop::ContentBlock::text(delta),
                    })
                    .await;
            }
            ca::AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let kind = self
                    .registry
                    .get(&tool_name)
                    .map(|tool| tool.kind())
                    .unwrap_or_default();
                let id = ToolCallId::new(tool_call_id);
                let _ = self
                    .events
                    .send(desktop::AgentEvent::ToolCall {
                        run: self.run.clone(),
                        call: desktop::ToolCall {
                            id,
                            title: tool_title(&tool_name, &args),
                            kind,
                            status: desktop::ToolStatus::Pending,
                            locations: Vec::new(),
                            content: Vec::new(),
                            raw_input: Some(args),
                        },
                    })
                    .await;
            }
            ca::AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial,
                ..
            } => {
                let blocks = tool_result_blocks_to_content(&partial.content);
                let _ = self
                    .events
                    .send(desktop::AgentEvent::ToolCallUpdate {
                        run: self.run.clone(),
                        id: ToolCallId::new(tool_call_id),
                        patch: desktop::ToolCallPatch {
                            status: Some(desktop::ToolStatus::InProgress),
                            append_content: blocks,
                            ..Default::default()
                        },
                    })
                    .await;
            }
            ca::AgentEvent::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                ..
            } => {
                let locations = locations_from_details(&result.details);
                // A Markdown file written into the document workspace becomes an
                // inline artifact (a rendered doc / slide viewer). Emitted before
                // the tool update so ordering is deterministic; the projection
                // dedupes by uri, so a rewrite updates the same card.
                if !is_error {
                    if let Some(docs) = &self.docs_dir {
                        for loc in &locations {
                            if let Some(artifact) = markdown_artifact(&loc.path, &tool_call_id, docs)
                            {
                                let _ = self
                                    .events
                                    .send(desktop::AgentEvent::Artifact {
                                        run: self.run.clone(),
                                        artifact,
                                    })
                                    .await;
                            }
                        }
                    }
                }
                let _ = self
                    .events
                    .send(desktop::AgentEvent::ToolCallUpdate {
                        run: self.run.clone(),
                        id: ToolCallId::new(tool_call_id),
                        patch: desktop::ToolCallPatch {
                            status: Some(if is_error {
                                desktop::ToolStatus::Failed
                            } else {
                                desktop::ToolStatus::Completed
                            }),
                            locations: (!locations.is_empty()).then_some(locations),
                            append_content: tool_result_blocks_to_content(&result.content),
                            ..Default::default()
                        },
                    })
                    .await;
            }
            _ => {}
        }
    }
}

fn to_wire_messages(system_prompt: &str, messages: &[ca::AgentMessage]) -> Vec<ChatMessage> {
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
                out.push(ChatMessage::user(user_content_text(content)));
            }
            ca::AgentMessage::Assistant { content, .. } => {
                out.push(ChatMessage {
                    role: "assistant".into(),
                    content: Some(content.plain_text()).filter(|text| !text.is_empty()),
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
            } => out.push(ChatMessage::tool(
                tool_call_id.clone(),
                content.plain_text(),
            )),
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

fn to_wire_tool_schema(tool: &ca::stream::ToolSchema) -> ToolSchema {
    ToolSchema::function(
        tool.name.clone(),
        tool.description.clone(),
        tool.parameters.clone(),
    )
}

fn to_wire_tool_call(call: &ca::ToolCall) -> WireToolCall {
    let args = serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string());
    WireToolCall::function(call.id.clone(), call.name.clone(), args)
}

fn assistant_message(turn: AssistantTurn) -> ca::AgentMessage {
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

fn to_core_tool_call(call: &WireToolCall) -> ca::ToolCall {
    ca::ToolCall {
        id: call.id.clone(),
        name: call.function.name.clone(),
        arguments: parse_tool_args(&call.function.arguments),
    }
}

fn parse_tool_args(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(error) => ca::arg_parse_error_value(error.to_string(), raw),
    }
}

fn stop_reason_from_finish(reason: Option<&str>) -> ca::StopReason {
    match reason {
        Some("length") => ca::StopReason::MaxTokens,
        Some("stop") | None => ca::StopReason::EndTurn,
        Some(_) => ca::StopReason::Other,
    }
}

fn empty_assistant(stop_reason: ca::StopReason, error_message: Option<String>) -> ca::AgentMessage {
    ca::AgentMessage::Assistant {
        content: ca::AssistantContent { blocks: Vec::new() },
        stop_reason,
        error_message,
        timestamp: None,
        usage: None,
    }
}

fn stream_error(error: LlmError) -> (ca::stream::StreamErrorKind, String) {
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

fn user_content_text(content: &ca::UserContent) -> String {
    match content {
        ca::UserContent::Text(text) => text.clone(),
        ca::UserContent::Blocks(blocks) => blocks
            .iter()
            .map(|block| match block {
                ca::UserBlock::Text(text) => text.text.clone(),
                ca::UserBlock::Image(image) => {
                    format!(
                        "[image: {}]",
                        image.alt.as_deref().unwrap_or("attached image")
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn tool_result_blocks_to_content(blocks: &[ca::ToolResultBlock]) -> Vec<desktop::ContentBlock> {
    blocks
        .iter()
        .map(|block| match block {
            ca::ToolResultBlock::Text(text) => desktop::ContentBlock::text(text.text.clone()),
            ca::ToolResultBlock::Image(image) => desktop::ContentBlock::ResourceLink {
                uri: image.source.clone(),
                name: image.alt.clone(),
            },
        })
        .collect()
}

fn locations_from_details(details: &Value) -> Vec<desktop::FsLocation> {
    details
        .get("locations")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// Build an inline Markdown artifact for a file the agent wrote into the document
/// workspace. Returns `None` for anything that isn't an existing Markdown file
/// inside `docs`. The `uri` is the canonical absolute path so the host can read
/// it back for the inline viewer.
fn markdown_artifact(
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

fn tool_title(name: &str, args: &Value) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_tool_args_use_core_parse_error_marker() {
        let value = parse_tool_args("{bad");
        assert!(ca::detect_arg_parse_error(&value).is_some());
    }

    #[test]
    fn tool_title_uses_salient_argument() {
        assert_eq!(
            tool_title("read_file", &json!({"path":"src/main.rs"})),
            "read_file: src/main.rs"
        );
    }

    #[test]
    fn markdown_artifact_only_for_md_inside_the_workspace() {
        let docs = tempfile::tempdir().unwrap();
        let docs_canon = docs.path().canonicalize().unwrap();

        // A .md written into the workspace → an inline markdown artifact.
        let md = docs_canon.join("report.md");
        std::fs::write(&md, "# Hi").unwrap();
        let art = markdown_artifact(md.to_str().unwrap(), "call-1", &docs_canon).expect("md doc");
        assert_eq!(art.kind, desktop::ArtifactKind::File);
        assert_eq!(art.mime_type.as_deref(), Some("text/markdown"));
        assert_eq!(art.uri.as_deref(), Some(md.to_str().unwrap()));
        assert_eq!(art.title, "report.md");

        // A non-markdown file in the workspace → no artifact.
        let txt = docs_canon.join("notes.txt");
        std::fs::write(&txt, "x").unwrap();
        assert!(markdown_artifact(txt.to_str().unwrap(), "c", &docs_canon).is_none());

        // A markdown file outside the workspace → no artifact.
        let outside = tempfile::tempdir().unwrap();
        let out_md = outside.path().canonicalize().unwrap().join("x.md");
        std::fs::write(&out_md, "x").unwrap();
        assert!(markdown_artifact(out_md.to_str().unwrap(), "c", &docs_canon).is_none());
    }
}
