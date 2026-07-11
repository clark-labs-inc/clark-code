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

use crate::llm::{
    AssistantTurn, ChatContent, ChatMessage, ContentPart, ImageUrlRef, LlmClient, LlmError,
    ToolSchema, WireToolCall,
};
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
            let reasoning_tx = tx.clone();
            let turn = llm
                .stream_chat(
                    &messages,
                    &tools,
                    &signal,
                    move |delta| {
                        let _ =
                            chunk_tx.send(ca::StreamEvent::Chunk(ca::AssistantStreamChunk::Text {
                                delta: delta.to_string(),
                            }));
                    },
                    move |delta| {
                        // GLM/OpenRouter streams hidden reasoning in
                        // `delta.reasoning`; forward it as a Reasoning chunk so
                        // the UI can render a live Thinking block instead of
                        // silence while the model thinks.
                        let _ = reasoning_tx.send(ca::StreamEvent::Chunk(
                            ca::AssistantStreamChunk::Reasoning {
                                delta: delta.to_string(),
                            },
                        ));
                    },
                )
                .await;

            match turn {
                Ok(turn) => {
                    if let Some(usage) = turn.usage {
                        totals.add(usage);
                    }
                    // GLM 5.2 over the Clark passthrough often ends a turn with
                    // its whole output in the OpenRouter `reasoning` field —
                    // empty `content`, no `tool_calls`, `finish_reason: stop`.
                    // Our accumulator reads only `content`/`tool_calls`, so that
                    // lands here as a genuinely empty turn. Reporting it as a
                    // normal `Done` ends the run with nothing ("second message
                    // did nothing"). Surface it as a zero-output transport so
                    // clark-agent replays the turn with its built-in recovery
                    // rather than succeeding on emptiness. This is a purely
                    // structural check (no output at all) — it never inspects
                    // what the text says.
                    if turn.text.is_empty() && turn.tool_calls.is_empty() {
                        let _ = tx.send(ca::StreamEvent::Error {
                            partial: empty_assistant(ca::StopReason::Error, None),
                            kind: ca::stream::StreamErrorKind::ZeroOutputTransport,
                            message: "provider returned no content and no tool call".to_string(),
                        });
                    } else {
                        let message = assistant_message(turn);
                        let _ = tx.send(ca::StreamEvent::Done { message });
                    }
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
    run: RunId,
    events: Sender<desktop::AgentEvent>,
) -> ca::ToolRegistry {
    let mut registry = ca::ToolRegistry::new();
    let gate = PermissionGate::new(session, control, session_id, events.clone());
    for exec in source.executors() {
        registry.register(Arc::new(DesktopToolAdapter {
            exec,
            ctx: ctx.clone(),
            gate: gate.clone(),
            run: run.clone(),
            events: events.clone(),
        }));
    }
    registry
}

struct DesktopToolAdapter {
    exec: Arc<dyn ToolExecutor>,
    ctx: ToolCtx,
    gate: PermissionGate,
    /// Needed so `update_plan` calls can emit a synthetic `AgentEvent::Plan`
    /// (that tool isn't part of `ca::AgentEvent`, so it can't ride the normal
    /// `DesktopEventSink` translation path).
    run: RunId,
    events: Sender<desktop::AgentEvent>,
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
        update: ca::ToolUpdateSink,
    ) -> Result<ca::ToolResult, ca::ToolError> {
        let tool_id = ToolCallId::new(call_id.to_string());

        // `update_plan` is an execution-progress checklist; Plan Mode is a
        // separate read-only research phase. It's non-mutating so it never
        // reaches the gate below, hence this dedicated check.
        if self.exec.name() == "update_plan" && self.gate.plan_mode_active().await {
            return Ok(ca::ToolResult::error(
                "update_plan is a checklist tool for the implementation phase — you're in Plan \
                mode; write your plan and call propose_plan instead.",
            ));
        }

        let mut args = match args {
            Value::Null => json!({}),
            other => other,
        };

        let hooks = { self.ctx.session.lock().await.hooks.clone() };
        if !hooks.pre_tool_use.is_empty() {
            match crate::hooks::run_pre_tool_use(
                self.ctx.executor.as_ref(),
                self.ctx.sandbox.root(),
                &hooks.pre_tool_use,
                self.exec.name(),
                args.clone(),
                &signal,
            )
            .await
            {
                crate::hooks::PreToolUseResult::Deny { reason } => {
                    return Ok(ca::ToolResult::error(format!(
                        "Blocked by a PreToolUse hook: {reason}"
                    )));
                }
                crate::hooks::PreToolUseResult::Allow { args: updated } => args = updated,
            }
        }

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

        let update_plan_args = (self.exec.name() == "update_plan").then(|| args.clone());
        // Hand the tool a live-progress sink: each reported delta rides the
        // engine's update channel out as `ToolExecutionUpdate`, which the UI
        // shows on the in-flight tool row (streamed shell output, grep progress).
        let mut call_ctx = self.ctx.clone();
        call_ctx.progress = Some(Arc::new(move |delta: String| {
            let _ = update.send(ca::ToolResult::text(delta));
        }));
        let mut outcome = self.exec.invoke(args.clone(), &call_ctx).await;
        if !outcome.is_error {
            if let Some(raw) = update_plan_args {
                if let Some(plan) = parse_update_plan(&raw) {
                    let _ = self
                        .events
                        .send(desktop::AgentEvent::Plan {
                            run: self.run.clone(),
                            plan,
                        })
                        .await;
                }
            }
        }

        if !hooks.post_tool_use.is_empty() {
            let extra = crate::hooks::run_post_tool_use(
                self.ctx.executor.as_ref(),
                self.ctx.sandbox.root(),
                &hooks.post_tool_use,
                self.exec.name(),
                &args,
                &outcome.content,
                &signal,
            )
            .await;
            if !extra.is_empty() {
                outcome.content = format!(
                    "{}\n\n[hook context]\n{}",
                    outcome.content,
                    extra.join("\n")
                );
            }
        }

        let mut result = if outcome.is_error {
            ca::ToolResult::error(outcome.content)
        } else {
            ca::ToolResult::text(outcome.content)
        };
        for image in &outcome.images {
            result
                .content
                .push(ca::ToolResultBlock::Image(ca::ImageContent {
                    source: format!("data:{};base64,{}", image.mime_type, image.data_base64),
                    media_type: Some(image.mime_type.clone()),
                    alt: image.alt.clone(),
                }));
        }
        if !outcome.locations.is_empty() {
            result.details = json!({ "locations": outcome.locations });
        }
        Ok(result)
    }
}

/// Parse an `update_plan` call's `{plan: [{step, status}]}` args into the
/// normalized `desktop::Plan` the projection layer already understands (it's
/// the same shape ACP's `plan` session/update and Clark's execution-plan
/// events already produce — see `provider-acp`/`provider-clark` translate.rs).
fn parse_update_plan(args: &Value) -> Option<desktop::Plan> {
    let items = args.get("plan")?.as_array()?;
    let phases = items
        .iter()
        .filter_map(|item| {
            let title = item.get("step")?.as_str()?.to_string();
            let status = match item.get("status").and_then(Value::as_str).unwrap_or("") {
                "in_progress" => desktop::PlanPhaseStatus::InProgress,
                "completed" => desktop::PlanPhaseStatus::Completed,
                _ => desktop::PlanPhaseStatus::Pending,
            };
            Some(desktop::PlanPhase {
                title,
                status,
                priority: None,
            })
        })
        .collect();
    Some(desktop::Plan { phases })
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
        if let Ok(payload) = serde_json::to_value(&event) {
            let _ = self
                .events
                .send(desktop::AgentEvent::Trace {
                    run: Some(self.run.clone()),
                    source: "clark_agent".to_string(),
                    payload,
                })
                .await;
        }
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
            ca::AgentEvent::MessageUpdate {
                chunk:
                    ca::AssistantStreamChunk::Reasoning { delta }
                    | ca::AssistantStreamChunk::Thinking { delta },
                ..
            } => {
                // Hidden reasoning → a Thinking content block. The frontend
                // renders it as the collapsible Thinking row (the same UI the
                // inline `<thinking>` tag path uses), and projection coalesces
                // adjacent blocks so streaming deltas merge into one.
                let _ = self
                    .events
                    .send(desktop::AgentEvent::MessageChunk {
                        run: self.run.clone(),
                        role: desktop::Role::Agent,
                        delta: desktop::ContentBlock::thinking(delta),
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
                // A Markdown file (or mobile-tool screenshot) written into the
                // document workspace becomes an inline artifact (a rendered
                // doc/slide viewer, or an image card). Emitted before the tool
                // update so ordering is deterministic; the projection dedupes
                // by uri, so a rewrite updates the same card.
                if !is_error {
                    if let Some(docs) = &self.docs_dir {
                        for loc in &locations {
                            let artifact = markdown_artifact(&loc.path, &tool_call_id, docs)
                                .or_else(|| {
                                    mobile_screenshot_artifact(&loc.path, &tool_call_id, docs)
                                });
                            if let Some(artifact) = artifact {
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
                            // Replace (not append): the final result supersedes
                            // any streamed partials so progress lines don't
                            // linger or duplicate the output.
                            replace_content: Some(tool_result_blocks_to_content(&result.content)),
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
fn user_chat_message(content: &ca::UserContent) -> ChatMessage {
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

fn tool_result_blocks_to_content(blocks: &[ca::ToolResultBlock]) -> Vec<desktop::ContentBlock> {
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
fn parse_data_url(s: &str) -> Option<(String, String)> {
    let rest = s.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime.to_string(), data.to_string()))
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

/// Build an inline image artifact for a screenshot a mobile-control tool
/// wrote into the document workspace. Same shape as `markdown_artifact`,
/// gated on image extension instead of `is_markdown`.
fn mobile_screenshot_artifact(
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

fn tool_title(name: &str, args: &Value) -> String {
    match name {
        "propose_plan" => return "Proposed a plan".to_string(),
        "update_plan" => return "Updated the plan".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_tool_args_use_core_parse_error_marker() {
        let value = parse_tool_args("{bad");
        assert!(ca::detect_arg_parse_error(&value).is_some());
    }

    #[tokio::test]
    async fn desktop_sink_preserves_stream_lifecycle_events_as_trace() {
        let (send, receive) = async_channel::bounded(2);
        let sink = DesktopEventSink::new(
            send,
            RunId::new("run-1"),
            Arc::new(ToolRegistry::new(None, None)),
            None,
        );
        ca::EventSink::emit(
            &sink,
            ca::AgentEvent::MessageStart {
                message: ca::AgentMessage::User {
                    content: ca::UserContent::Text("hello".into()),
                    timestamp: None,
                },
            },
        )
        .await;

        let event = receive.recv().await.expect("trace event");
        match event {
            desktop::AgentEvent::Trace {
                source, payload, ..
            } => {
                assert_eq!(source, "clark_agent");
                assert_eq!(payload["type"], "message_start");
                assert_eq!(payload["message"]["content"], "hello");
            }
            other => panic!("expected trace event, got {other:?}"),
        }
    }

    #[test]
    fn tool_title_uses_salient_argument() {
        assert_eq!(
            tool_title("read_file", &json!({"path":"src/main.rs"})),
            "read_file: src/main.rs"
        );
    }

    #[test]
    fn tool_title_special_cases_plan_tools() {
        assert_eq!(tool_title("propose_plan", &json!({})), "Proposed a plan");
        assert_eq!(tool_title("update_plan", &json!({})), "Updated the plan");
    }

    #[test]
    fn parse_update_plan_maps_steps_and_statuses() {
        let args = json!({"plan": [
            {"step": "read the code", "status": "completed"},
            {"step": "write the fix", "status": "in_progress"},
            {"step": "test it", "status": "pending"},
        ]});
        let plan = parse_update_plan(&args).expect("valid plan");
        assert_eq!(plan.phases.len(), 3);
        assert_eq!(plan.phases[0].title, "read the code");
        assert_eq!(plan.phases[0].status, desktop::PlanPhaseStatus::Completed);
        assert_eq!(plan.phases[1].status, desktop::PlanPhaseStatus::InProgress);
        assert_eq!(plan.phases[2].status, desktop::PlanPhaseStatus::Pending);
    }

    #[test]
    fn parse_update_plan_rejects_missing_plan_array() {
        assert!(parse_update_plan(&json!({})).is_none());
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

    #[test]
    fn mobile_screenshot_artifact_only_for_images_inside_the_workspace() {
        let docs = tempfile::tempdir().unwrap();
        let docs_canon = docs.path().canonicalize().unwrap();

        let png = docs_canon.join("sim.png");
        std::fs::write(&png, [0u8; 4]).unwrap();
        let art = mobile_screenshot_artifact(png.to_str().unwrap(), "call-1", &docs_canon)
            .expect("png screenshot");
        assert_eq!(art.kind, desktop::ArtifactKind::Image);
        assert_eq!(art.mime_type.as_deref(), Some("image/png"));
        assert_eq!(art.uri.as_deref(), Some(png.to_str().unwrap()));

        let jpg = docs_canon.join("sim.jpg");
        std::fs::write(&jpg, [0u8; 4]).unwrap();
        let art = mobile_screenshot_artifact(jpg.to_str().unwrap(), "call-1", &docs_canon)
            .expect("jpg screenshot");
        assert_eq!(art.mime_type.as_deref(), Some("image/jpeg"));

        // A non-image file in the workspace → no artifact.
        let txt = docs_canon.join("notes.txt");
        std::fs::write(&txt, "x").unwrap();
        assert!(mobile_screenshot_artifact(txt.to_str().unwrap(), "c", &docs_canon).is_none());

        // A PNG outside the workspace → no artifact.
        let outside = tempfile::tempdir().unwrap();
        let out_png = outside.path().canonicalize().unwrap().join("x.png");
        std::fs::write(&out_png, [0u8; 4]).unwrap();
        assert!(mobile_screenshot_artifact(out_png.to_str().unwrap(), "c", &docs_canon).is_none());
    }

    #[test]
    fn user_chat_message_stays_plain_text_with_no_images() {
        let content = ca::UserContent::Blocks(vec![ca::UserBlock::Text(ca::types::TextContent {
            text: "hello".into(),
        })]);
        let msg = user_chat_message(&content);
        assert_eq!(msg.role, "user");
        match msg.content {
            Some(ChatContent::Text(t)) => assert_eq!(t, "hello"),
            other => panic!("expected plain text content, got {other:?}"),
        }
    }

    #[test]
    fn user_chat_message_forwards_images_as_content_parts() {
        let content = ca::UserContent::Blocks(vec![
            ca::UserBlock::Text(ca::types::TextContent {
                text: "check this out".into(),
            }),
            ca::UserBlock::Image(ca::ImageContent {
                source: "data:image/png;base64,QUJD".into(),
                media_type: Some("image/png".into()),
                alt: None,
            }),
        ]);
        let msg = user_chat_message(&content);
        assert_eq!(msg.role, "user");
        match msg.content {
            Some(ChatContent::Parts(parts)) => {
                assert_eq!(parts.len(), 2);
                assert!(
                    matches!(&parts[0], ContentPart::Text { text } if text == "check this out")
                );
                assert!(matches!(
                    &parts[1],
                    ContentPart::ImageUrl { image_url } if image_url.url == "data:image/png;base64,QUJD"
                ));
            }
            other => panic!("expected content-parts, got {other:?}"),
        }
    }

    #[test]
    fn to_wire_messages_injects_synthetic_user_turn_for_tool_result_images() {
        let messages = vec![ca::AgentMessage::ToolResult {
            tool_call_id: "call-1".into(),
            tool_name: "ios_screenshot".into(),
            content: ca::ToolResultContent {
                blocks: vec![
                    ca::ToolResultBlock::Text(ca::types::TextContent {
                        text: "Screenshot captured.".into(),
                    }),
                    ca::ToolResultBlock::Image(ca::ImageContent {
                        source: "data:image/png;base64,QUJD".into(),
                        media_type: Some("image/png".into()),
                        alt: None,
                    }),
                ],
            },
            is_error: false,
            narration: None,
            details: None,
            timestamp: None,
        }];
        let wire = to_wire_messages("", &messages);

        // The tool-role message itself stays plain text — the OpenAI-compatible
        // wire format doesn't allow a content-parts array on role: "tool".
        assert_eq!(wire[0].role, "tool");
        match &wire[0].content {
            Some(ChatContent::Text(t)) => assert_eq!(t, "Screenshot captured."),
            other => panic!("expected plain text tool content, got {other:?}"),
        }

        // The image rides in as a synthetic follow-up user turn.
        assert_eq!(wire[1].role, "user");
        match &wire[1].content {
            Some(ChatContent::Parts(parts)) => {
                assert!(parts.iter().any(|p| matches!(
                    p,
                    ContentPart::ImageUrl { image_url } if image_url.url == "data:image/png;base64,QUJD"
                )));
            }
            other => panic!("expected content-parts with the image, got {other:?}"),
        }
        assert_eq!(wire.len(), 2);
    }

    #[test]
    fn to_wire_messages_skips_synthetic_turn_when_no_images() {
        let messages = vec![ca::AgentMessage::ToolResult {
            tool_call_id: "call-1".into(),
            tool_name: "grep".into(),
            content: ca::ToolResultContent::text("no matches"),
            is_error: false,
            narration: None,
            details: None,
            timestamp: None,
        }];
        let wire = to_wire_messages("", &messages);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0].role, "tool");
    }

    #[test]
    fn tool_result_blocks_to_content_maps_image_data_url_to_image_block() {
        let blocks = vec![ca::ToolResultBlock::Image(ca::ImageContent {
            source: "data:image/png;base64,QUJD".into(),
            media_type: Some("image/png".into()),
            alt: Some("a screenshot".into()),
        })];
        let content = tool_result_blocks_to_content(&blocks);
        assert_eq!(content.len(), 1);
        match &content[0] {
            desktop::ContentBlock::Image {
                mime_type,
                data,
                uri,
            } => {
                assert_eq!(mime_type, "image/png");
                assert_eq!(data, "QUJD");
                assert!(uri.is_none());
            }
            other => panic!("expected an Image content block, got {other:?}"),
        }
    }
}
