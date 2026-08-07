mod event_sink;
pub(crate) mod redaction;
mod tool_title;
mod translate;

use std::sync::Arc;

use agent_core::domain as desktop;
use agent_core::ids::{RunId, SessionId, ToolCallId};
use async_channel::Sender;
use async_trait::async_trait;
use clark_agent as ca;
use futures::{stream::BoxStream, StreamExt};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::config::AgenticClarkConfig;
use crate::llm::{AssistantTurn, LlmClient};
#[cfg(test)]
use crate::llm::{ChatContent, ContentPart};
use crate::loop_state::{RunControl, SessionState};
use crate::permissions::{PermissionGate, PermissionOutcome};
use crate::tools::{
    ProducedArtifact, ToolCtx, ToolExecutor, ToolOutcome, ToolRegistry, ToolSignal,
};

use tool_title::tool_title;
use translate::*;

pub(crate) use event_sink::DesktopEventSink;
pub(crate) use translate::to_wire_messages;

/// Running token/cost totals across a run's model calls, shared between the
/// stream adapter (writer) and the engine (reads them into the run outcome).
#[derive(Default)]
pub(crate) struct UsageTotals {
    inner: std::sync::Mutex<agent_core::domain::RunUsage>,
    seen: std::sync::atomic::AtomicBool,
}

impl UsageTotals {
    fn add(&self, usage: crate::llm::TokenUsage) -> agent_core::domain::RunUsage {
        let mut t = self.inner.lock().expect("usage totals lock");
        t.input_tokens += usage.prompt_tokens;
        t.output_tokens += usage.completion_tokens;
        // The latest call's prompt is the conversation's live context footprint.
        t.context_tokens = usage.prompt_tokens;
        if let Some(cost) = usage.cost_usd {
            t.cost_usd = Some(t.cost_usd.unwrap_or(0.0) + cost);
        }
        self.seen.store(true, std::sync::atomic::Ordering::Relaxed);
        *t
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
    incidents: crate::incidents::ProviderIncidentTracker,
    events: Sender<desktop::AgentEvent>,
    session: Arc<Mutex<SessionState>>,
    run: RunId,
    context_limit: Option<u64>,
    weighted_token_limit: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExecutionBudgetState {
    Within,
    Approaching,
    Exhausted,
}

fn execution_budget_state(
    usage: Option<agent_core::domain::RunUsage>,
    limit: Option<f64>,
) -> ExecutionBudgetState {
    let Some(limit) = limit.filter(|limit| limit.is_finite() && *limit > 0.0) else {
        return ExecutionBudgetState::Within;
    };
    let used = usage
        .map(|usage| usage.input_tokens.saturating_add(usage.output_tokens) as f64)
        .unwrap_or(0.0);
    if used >= limit {
        ExecutionBudgetState::Exhausted
    } else if used >= limit * 0.9 {
        ExecutionBudgetState::Approaching
    } else {
        ExecutionBudgetState::Within
    }
}

impl ClarkAgentStream {
    pub fn new(
        llm: LlmClient,
        incidents: crate::incidents::ProviderIncidentTracker,
        events: Sender<desktop::AgentEvent>,
        session: Arc<Mutex<SessionState>>,
        run: RunId,
        context_limit: Option<u64>,
        weighted_token_limit: Option<f64>,
    ) -> Self {
        Self {
            llm,
            totals: Arc::new(UsageTotals::default()),
            incidents,
            events,
            session,
            run,
            context_limit,
            weighted_token_limit,
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
        let budget_state =
            execution_budget_state(self.totals.snapshot(), self.weighted_token_limit);
        if budget_state == ExecutionBudgetState::Exhausted {
            return futures::stream::iter([
                ca::StreamEvent::Start {
                    partial: empty_assistant(ca::StopReason::EndTurn, None),
                },
                ca::StreamEvent::Error {
                    partial: empty_assistant(ca::StopReason::Error, None),
                    kind: ca::stream::StreamErrorKind::Fatal,
                    message: "execution_budget_exhausted: cumulative model-token safety limit reached; the conversation and completed work are preserved, so continue in a follow-up run".to_string(),
                },
            ])
            .boxed();
        }
        let llm = self.llm.clone();
        let totals = self.totals.clone();
        let incidents = self.incidents.clone();
        let events = self.events.clone();
        let session = self.session.clone();
        let run = self.run.clone();
        let context_limit = self.context_limit;
        let force_tool_call = {
            let session = session.lock().await;
            let autonomous_goal_in_progress = session
                .goal
                .as_ref()
                .is_some_and(|goal| goal.status != crate::loop_state::GoalStatus::Complete);
            request.force_tool_call
                && !session.planning.plan_mode()
                && !autonomous_goal_in_progress
                && request
                    .tools
                    .iter()
                    .any(|tool| tool.name == crate::tools::final_answer::FINAL_ANSWER_TOOL)
        };
        let mut messages = to_wire_messages(&request.system_prompt, &request.messages);
        if budget_state == ExecutionBudgetState::Approaching {
            messages.push(crate::llm::ChatMessage::system(
                "Execution budget is nearly exhausted. Stop broad exploration, complete only essential verification, then call final_answer with a concise handoff. Do not start another large task in this run.",
            ));
        }
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
            let proposal_filter =
                Arc::new(std::sync::Mutex::new(ProposedPlanStreamFilter::default()));
            let proposal_filter_for_text = proposal_filter.clone();
            let turn = llm
                .stream_chat_observed_with_tool_choice(
                    &messages,
                    &tools,
                    crate::llm::StreamChatOptions {
                        cancel: &signal,
                        force_tool_call,
                    },
                    move |delta| {
                        if force_tool_call {
                            return;
                        }
                        let visible = proposal_filter_for_text
                            .lock()
                            .map(|mut filter| filter.feed(delta))
                            .unwrap_or_else(|_| delta.to_string());
                        if !visible.is_empty() {
                            let _ = chunk_tx.send(ca::StreamEvent::Chunk(
                                ca::AssistantStreamChunk::Text { delta: visible },
                            ));
                        }
                    },
                    move |delta| {
                        // The transport releases reasoning only after the whole
                        // provider turn passes isolation validation. Preserve
                        // its typed Thinking projection after that boundary.
                        let _ = reasoning_tx.send(ca::StreamEvent::Chunk(
                            ca::AssistantStreamChunk::Reasoning {
                                delta: delta.to_string(),
                            },
                        ));
                    },
                    {
                        let incidents = incidents.clone();
                        move |context| incidents.observe_retry(context)
                    },
                )
                .await;

            match turn {
                Ok(mut turn) => {
                    if force_tool_call && turn.tool_calls.is_empty() {
                        let payload = turn
                            .response_metadata
                            .as_ref()
                            .and_then(|metadata| serde_json::to_value(metadata).ok())
                            .unwrap_or(Value::Null);
                        let _ = events
                            .send(desktop::AgentEvent::Trace {
                                run: Some(run.clone()),
                                source: "provider_output_contract_violation".to_string(),
                                payload,
                            })
                            .await;
                        let _ = tx.send(ca::StreamEvent::Error {
                            partial: empty_assistant(ca::StopReason::Error, None),
                            kind: ca::stream::StreamErrorKind::ZeroOutputTransport,
                            message: "provider ignored required tool choice".to_string(),
                        });
                        return;
                    }
                    if force_tool_call {
                        // Hold text until the required structured boundary is
                        // known. Ordinary tool turns may carry progress
                        // commentary, while terminal delivery comes only from
                        // the typed final_answer payload.
                        let is_terminal = turn.tool_calls.iter().any(|call| {
                            call.function.name == crate::tools::final_answer::FINAL_ANSWER_TOOL
                        });
                        if is_terminal {
                            turn.text.clear();
                        } else if !turn.text.is_empty() {
                            let _ =
                                tx.send(ca::StreamEvent::Chunk(ca::AssistantStreamChunk::Text {
                                    delta: turn.text.clone(),
                                }));
                        }
                    }
                    // If a response ended while the stream filter was holding
                    // a possible tag prefix, release it. A malformed marker
                    // must remain visible rather than disappearing.
                    if let Ok(mut filter) = proposal_filter.lock() {
                        let visible = filter.finish();
                        if !visible.is_empty() {
                            let _ =
                                tx.send(ca::StreamEvent::Chunk(ca::AssistantStreamChunk::Text {
                                    delta: visible,
                                }));
                        }
                    }
                    incidents.mark_recovered();
                    if let Some(metadata) = &turn.response_metadata {
                        if let Ok(payload) = serde_json::to_value(metadata) {
                            let _ = events
                                .send(desktop::AgentEvent::Trace {
                                    run: Some(run.clone()),
                                    source: "model_response".to_string(),
                                    payload,
                                })
                                .await;
                        }
                    }
                    if let Some(usage) = turn.usage {
                        let mut usage = totals.add(usage);
                        usage.context_limit = context_limit;
                        let _ = events
                            .send(desktop::AgentEvent::RunUsageUpdated {
                                run: run.clone(),
                                usage,
                            })
                            .await;
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
                    let proposed_plan = {
                        let mut session = session.lock().await;
                        if session.planning.plan_mode() {
                            crate::planning::extract_proposed_plan(&turn.text)
                                .map(|markdown| session.planning.next_markdown_proposal(markdown))
                        } else {
                            None
                        }
                    };
                    if let Some(plan) = proposed_plan {
                        let _ = events
                            .send(desktop::AgentEvent::ProposedPlanUpdated {
                                run: run.clone(),
                                plan,
                            })
                            .await;
                    }
                    if turn.text.is_empty() && turn.tool_calls.is_empty() {
                        let _ = tx.send(ca::StreamEvent::Error {
                            partial: empty_assistant(ca::StopReason::Error, None),
                            kind: ca::stream::StreamErrorKind::ZeroOutputTransport,
                            message: "provider returned no content and no tool call".to_string(),
                        });
                    } else {
                        if should_surface_reasoning_details(&turn) {
                            let _ = tx.send(ca::StreamEvent::Chunk(
                                ca::AssistantStreamChunk::ReasoningDetails {
                                    delta: turn.reasoning_details.clone(),
                                },
                            ));
                        }
                        let message = assistant_message(turn);
                        let _ = tx.send(ca::StreamEvent::Done { message });
                    }
                }
                Err(error) => {
                    if let Some((reason, metadata)) = error.quarantine_receipt() {
                        let _ = events
                            .send(desktop::AgentEvent::Trace {
                                run: Some(run.clone()),
                                source: "provider_output_quarantined".to_string(),
                                payload: json!({
                                    "reason": reason,
                                    "response": metadata,
                                }),
                            })
                            .await;
                    }
                    if let Some(context) = error.provider_failure().cloned() {
                        incidents.observe_terminal(context);
                    }
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

fn should_surface_reasoning_details(turn: &AssistantTurn) -> bool {
    // Some OpenRouter providers mirror the same readable thought through both
    // `reasoning` and `reasoning_details`. The former was already streamed live,
    // so emitting the latter here would append a duplicate Thinking block after
    // the answer. Details remain on the completed assistant message for replay.
    turn.reasoning.trim().is_empty() && !turn.reasoning_details.is_empty()
}

#[derive(Default)]
struct ProposedPlanStreamFilter {
    pending: String,
    in_plan: bool,
}

impl ProposedPlanStreamFilter {
    fn feed(&mut self, delta: &str) -> String {
        self.pending.push_str(delta);
        self.drain(false)
    }

    fn finish(&mut self) -> String {
        self.drain(true)
    }

    fn drain(&mut self, final_chunk: bool) -> String {
        let mut visible = String::new();
        loop {
            if self.in_plan {
                if let Some(end) = self.pending.find("</proposed_plan>") {
                    self.pending = self.pending[end + "</proposed_plan>".len()..].to_string();
                    self.in_plan = false;
                    continue;
                }
                if final_chunk {
                    visible.push_str("<proposed_plan>");
                    visible.push_str(&self.pending);
                    self.pending.clear();
                    self.in_plan = false;
                }
                break;
            }
            if let Some(start) = self.pending.find("<proposed_plan>") {
                visible.push_str(&self.pending[..start]);
                self.pending = self.pending[start + "<proposed_plan>".len()..].to_string();
                self.in_plan = true;
                continue;
            }
            if final_chunk {
                visible.push_str(&self.pending);
                self.pending.clear();
            } else {
                // Hold only a possible opening-tag prefix across token
                // boundaries; emit all other text immediately.
                let keep = (1.."<proposed_plan>".len())
                    .rev()
                    .find(|length| self.pending.ends_with(&"<proposed_plan>"[..*length]))
                    .unwrap_or(0);
                let emit_len = self.pending.len().saturating_sub(keep);
                visible.push_str(&self.pending[..emit_len]);
                self.pending = self.pending[emit_len..].to_string();
            }
            break;
        }
        visible
    }
}

pub(crate) struct DesktopToolRegistryOptions {
    pub session: Arc<Mutex<SessionState>>,
    pub control: Arc<Mutex<RunControl>>,
    pub session_id: SessionId,
    pub run: RunId,
    pub events: Sender<desktop::AgentEvent>,
    pub execution: Option<crate::root_execution::RootExecutionTrace>,
    pub image_policy: ToolImagePolicy,
    /// Plan Mode uses Codex's hidden proposed-plan artifact instead of making
    /// either plan tool part of autoregressive context.
    pub hide_plan_mode_tools: bool,
}

/// How tool-produced images reach the current coding model. The UI always
/// receives typed image blocks; models without native image input receive a
/// bounded isolated vision description instead of an incompatible synthetic
/// multimodal turn.
#[derive(Clone)]
pub(crate) struct ToolImagePolicy {
    pub native_image_support: bool,
    pub vision: Option<AgenticClarkConfig>,
}

pub(crate) fn desktop_tool_registry(
    source: Arc<ToolRegistry>,
    ctx: ToolCtx,
    options: DesktopToolRegistryOptions,
) -> ca::ToolRegistry {
    let mut registry = ca::ToolRegistry::new();
    let mut gate = PermissionGate::new(
        options.session,
        options.control,
        options.session_id,
        options.events.clone(),
    );
    if let Some(execution) = options.execution {
        gate = gate.with_execution(execution);
    }
    for exec in source.executors() {
        if options.hide_plan_mode_tools
            && matches!(
                exec.name(),
                "propose_plan" | "update_plan" | crate::tools::final_answer::FINAL_ANSWER_TOOL
            )
        {
            continue;
        }
        registry.register(Arc::new(DesktopToolAdapter {
            exec,
            ctx: ctx.clone(),
            gate: gate.clone(),
            run: options.run.clone(),
            events: options.events.clone(),
            image_policy: options.image_policy.clone(),
        }));
    }
    registry
}

struct DesktopToolAdapter {
    exec: Arc<dyn ToolExecutor>,
    ctx: ToolCtx,
    gate: PermissionGate,
    /// Typed tool effects are normalized against the active run here because
    /// they are provider-local outcomes, not `clark-agent` stream events.
    run: RunId,
    events: Sender<desktop::AgentEvent>,
    image_policy: ToolImagePolicy,
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

        let mut args = match args {
            Value::Null => json!({}),
            other => other,
        };

        let hooks = { self.ctx.session.lock().await.hooks.clone() };
        if !hooks.pre_tool_use.is_empty() {
            let hook_args = redaction::persisted_tool_args(self.exec.name(), &args);
            let runtime_args = args.clone();
            match crate::hooks::run_pre_tool_use(
                self.ctx.executor.as_ref(),
                self.ctx.sandbox.root(),
                &hooks.pre_tool_use,
                self.exec.name(),
                hook_args,
                &signal,
            )
            .await
            {
                crate::hooks::PreToolUseResult::Deny { reason } => {
                    return Ok(ca::ToolResult::error(format!(
                        "Blocked by a PreToolUse hook: {reason}"
                    )));
                }
                crate::hooks::PreToolUseResult::Allow { args: updated } => {
                    args =
                        redaction::restore_runtime_payload(self.exec.name(), &runtime_args, updated)
                }
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
            PermissionOutcome::Failed(message) => return Err(ca::ToolError::Fatal(message)),
        }

        if signal.is_cancelled() {
            return Err(ca::ToolError::Aborted);
        }

        // Hand the tool a live-progress sink: each reported delta rides the
        // engine's update channel out as `ToolExecutionUpdate`, which the UI
        // shows on the in-flight tool row (streamed shell output, grep progress).
        let mut call_ctx = self.ctx.clone();
        call_ctx.progress = Some(Arc::new(move |delta: String| {
            let _ = update.send(ca::ToolResult::text(delta));
        }));
        let events = self.events.clone();
        let progress_run = self.run.clone();
        let progress_parent = tool_id.clone();
        call_ctx.agent_progress = Some(Arc::new(move |agent| {
            let _ = events.try_send(desktop::AgentEvent::FanOut {
                run: progress_run.clone(),
                parent: progress_parent.clone(),
                agent,
            });
        }));
        let events = self.events.clone();
        let progress_run = self.run.clone();
        let progress_tool = tool_id.clone();
        call_ctx.call_progress = Some(Arc::new(move |progress| {
            let _ = events.try_send(desktop::AgentEvent::ToolCallUpdate {
                run: progress_run.clone(),
                id: progress_tool.clone(),
                patch: desktop::ToolCallPatch {
                    status: Some(desktop::ToolStatus::InProgress),
                    progress: Some(progress),
                    ..Default::default()
                },
            });
        }));
        let effect_intent = self.exec.effect_intent(&args);
        let mut outcome = self.exec.invoke(args.clone(), &call_ctx).await;
        let terminates_run = self.exec.terminates_run();
        if terminates_run && !outcome.is_error {
            let unresolved = self
                .ctx
                .session
                .lock()
                .await
                .effects
                .completion_prompt(&self.run);
            if let Some(reminder) = unresolved {
                outcome = ToolOutcome::error(reminder);
            }
        }
        if !outcome.is_error {
            if let Some(intent) = effect_intent {
                let receipt = self.ctx.session.lock().await.effects.register(
                    self.run.clone(),
                    call_id.to_string(),
                    self.exec.name(),
                    intent,
                );
                crate::effects::attach_pending_receipt(&mut outcome, &receipt);
            }
        }
        if !outcome.is_error {
            for signal in std::mem::take(&mut outcome.signals) {
                let event = match signal {
                    ToolSignal::ExecutionChecklist {
                        checklist,
                        explanation,
                    } => desktop::AgentEvent::ExecutionChecklistUpdated {
                        run: self.run.clone(),
                        checklist,
                        explanation,
                    },
                    ToolSignal::ProposedPlan(plan) => desktop::AgentEvent::ProposedPlanUpdated {
                        run: self.run.clone(),
                        plan,
                    },
                    ToolSignal::Goal(mut goal) => {
                        goal.run = Some(self.run.clone());
                        desktop::AgentEvent::GoalUpdated {
                            run: self.run.clone(),
                            goal,
                        }
                    }
                };
                let _ = self.events.send(event).await;
            }
        }

        if !hooks.post_tool_use.is_empty() {
            let hook_args = redaction::persisted_tool_args(self.exec.name(), &args);
            let post = crate::hooks::run_post_tool_use(
                self.ctx.executor.as_ref(),
                self.ctx.sandbox.root(),
                &hooks.post_tool_use,
                self.exec.name(),
                &hook_args,
                &outcome,
                &signal,
            )
            .await;
            if let Some(reason) = post.block_reason {
                if !outcome.details.is_object() {
                    outcome.details = json!({});
                }
                outcome.details["_clark_post_tool_original_content"] =
                    Value::String(outcome.content.clone());
                outcome.content = format!(
                    "PostToolUse hook rejected the tool result after execution: {reason}. Inspect canonical state and correct the effect before continuing."
                );
                outcome.is_error = true;
            } else if let Some(feedback) = post.feedback_message {
                if !outcome.details.is_object() {
                    outcome.details = json!({});
                }
                outcome.details["_clark_post_tool_original_content"] =
                    Value::String(outcome.content.clone());
                outcome.content = feedback;
            } else if !post.additional_contexts.is_empty() {
                outcome.content = format!(
                    "{}\n\n[hook context]\n{}",
                    outcome.content,
                    post.additional_contexts.join("\n")
                );
            }
        }

        if !outcome.is_error {
            for artifact in &outcome.artifacts {
                let _ = self
                    .events
                    .send(desktop::AgentEvent::Artifact {
                        run: self.run.clone(),
                        artifact: produced_artifact_to_desktop(artifact, &tool_id),
                    })
                    .await;
            }
        }

        if !self.image_policy.native_image_support && !outcome.images.is_empty() {
            // Keep the actual bytes in structured result metadata for the UI,
            // but do not inject them into a non-vision coding model's wire
            // transcript. A separate vision-only request gives the model the
            // visual facts it needs to continue safely.
            outcome.content.push_str(
                &crate::attachments::describe_tool_images(
                    &outcome.images,
                    self.image_policy.vision.as_ref(),
                    &signal,
                )
                .await,
            );
            store_tool_images(&mut outcome.details, &outcome.images);
        }

        Ok(tool_result_from_outcome(
            outcome,
            self.image_policy.native_image_support,
            terminates_run,
        ))
    }
}

fn tool_result_from_outcome(
    outcome: ToolOutcome,
    native_image_support: bool,
    terminates_run: bool,
) -> ca::ToolResult {
    let mut result = if outcome.is_error {
        ca::ToolResult::error(outcome.content)
    } else {
        ca::ToolResult::text(outcome.content)
    };
    if native_image_support {
        for image in &outcome.images {
            result
                .content
                .push(ca::ToolResultBlock::Image(ca::ImageContent {
                    source: format!("data:{};base64,{}", image.mime_type, image.data_base64),
                    media_type: Some(image.mime_type.clone()),
                    alt: image.alt.clone(),
                }));
        }
    }
    result.details = outcome.details;
    if !outcome.locations.is_empty() {
        if !result.details.is_object() {
            result.details = json!({});
        }
        result.details["locations"] = json!(outcome.locations);
    }
    result.terminate = terminates_run && !result.is_error;
    result
}

fn produced_artifact_to_desktop(
    artifact: &ProducedArtifact,
    tool_call: &ToolCallId,
) -> desktop::Artifact {
    desktop::Artifact {
        id: artifact.id.clone(),
        title: artifact.title.clone(),
        kind: artifact.kind,
        mime_type: artifact.mime_type.clone(),
        uri: artifact.uri.clone(),
        tool_call: Some(tool_call.clone()),
    }
}

#[cfg(test)]
#[path = "agent_adapter_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_adapter_translate_tests.rs"]
mod translate_tests;

#[cfg(test)]
#[path = "agent_adapter_reasoning_tests.rs"]
mod reasoning_tests;
