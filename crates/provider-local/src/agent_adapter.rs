mod document_stream;
mod event_sink;
mod final_answer_stream;
mod model_attempt_receipt;
mod partial_json;
mod proposed_plan_stream;
mod reasoning_stream;
pub(crate) mod redaction;
mod required_tool_text;
mod stream_progress;
mod streaming_tool_call;
mod tool_call_stream;
mod tool_title;
mod translate;

pub(crate) const TOOL_PROTOCOL_RECOVERY_EXHAUSTED: &str = "tool_protocol_recovery_exhausted:";

use std::sync::Arc;

use agent_core::domain as desktop;
use agent_core::ids::{RunId, SessionId, ToolCallId};
use agent_loop as ca;
use async_channel::Sender;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::config::AuxiliaryModelConfig;
use crate::llm::{AssistantTurn, LlmClient};
#[cfg(test)]
use crate::llm::{ChatContent, ContentPart};
use crate::loop_state::{RunControl, SessionState};
use crate::permissions::{PermissionGate, PermissionOutcome};
use crate::tools::{
    ProducedArtifact, ToolCtx, ToolExecutor, ToolOutcome, ToolRegistry, ToolSignal,
};

use model_attempt_receipt::emit_model_attempt_receipt;
#[cfg(test)]
use proposed_plan_stream::ProposedPlanStreamFilter;
use stream_progress::StreamProgress;
use tool_call_stream::ToolCallStreamGate;
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
pub(crate) struct AgentLoopStream {
    llm: LlmClient,
    totals: Arc<UsageTotals>,
    incidents: crate::incidents::ProviderIncidentTracker,
    events: Sender<desktop::AgentEvent>,
    session: Arc<Mutex<SessionState>>,
    run: RunId,
    context_limit: Option<u64>,
}

impl AgentLoopStream {
    pub fn new(
        llm: LlmClient,
        incidents: crate::incidents::ProviderIncidentTracker,
        events: Sender<desktop::AgentEvent>,
        session: Arc<Mutex<SessionState>>,
        run: RunId,
        context_limit: Option<u64>,
    ) -> Self {
        Self {
            llm,
            totals: Arc::new(UsageTotals::default()),
            incidents,
            events,
            session,
            run,
            context_limit,
        }
    }

    /// Handle the engine holds to fold totals into the run outcome.
    pub fn usage(&self) -> Arc<UsageTotals> {
        self.totals.clone()
    }
}

#[async_trait]
impl ca::StreamFn for AgentLoopStream {
    async fn stream(
        &self,
        request: ca::StreamRequest,
        signal: CancellationToken,
    ) -> BoxStream<'static, ca::StreamEvent> {
        let llm = self.llm.clone();
        let totals = self.totals.clone();
        let incidents = self.incidents.clone();
        let events = self.events.clone();
        let session = self.session.clone();
        let run = self.run.clone();
        let context_limit = self.context_limit;
        let (force_tool_call, stream_terminal_tool) = {
            let session = session.lock().await;
            let autonomous_goal_in_progress = session
                .goal
                .as_ref()
                .is_some_and(|goal| goal.status != crate::loop_state::GoalStatus::Complete);
            let force_tool_call = request.force_tool_call
                && !session.planning.plan_mode()
                && !autonomous_goal_in_progress
                && request
                    .tools
                    .iter()
                    .any(|tool| tool.name == crate::tools::final_answer::FINAL_ANSWER_TOOL);
            let stream_terminal_tool =
                force_tool_call && session.effects.unresolved_diagnostics(&run).is_empty();
            (force_tool_call, stream_terminal_tool)
        };
        let mut messages = to_wire_messages(&request.system_prompt, &request.messages);
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
            let stream_progress = StreamProgress::new(force_tool_call);
            let mut discarded_usage = Vec::new();
            let mut repair_attempts = 0_u8;
            let turn = loop {
                let chunk_tx = chunk_tx.clone();
                let reasoning_tx = reasoning_tx.clone();
                let incidents_for_retry = incidents.clone();
                let tool_call_gate = Arc::new(std::sync::Mutex::new(ToolCallStreamGate::new(
                    stream_terminal_tool,
                )));
                let response = llm
                    .stream_chat_observed_with_tool_choice(
                        &messages,
                        &tools,
                        crate::llm::StreamChatOptions {
                            cancel: &signal,
                            force_tool_call,
                            forced_tool_name: (repair_attempts >= 2)
                                .then_some(crate::tools::final_answer::FINAL_ANSWER_TOOL),
                        },
                        crate::llm::StreamObservers::new(
                            {
                                let stream_progress = stream_progress.clone();
                                move |delta: &str| stream_progress.observe_text(&chunk_tx, delta)
                            },
                            move |delta: &str| {
                                // Reasoning is a typed progress channel, not the
                                // user-facing answer contract. Keep it live even
                                // while required-tool prose remains quarantined.
                                let _ = reasoning_tx.send(ca::StreamEvent::Chunk(
                                    ca::AssistantStreamChunk::Reasoning {
                                        delta: delta.to_string(),
                                    },
                                ));
                            },
                            {
                                let tool_tx = tx.clone();
                                let tool_call_gate = tool_call_gate.clone();
                                let stream_progress = stream_progress.clone();
                                move |delta: crate::llm::WireToolCallDelta| {
                                    stream_progress.observe_tool(&tool_tx, &tool_call_gate, delta)
                                }
                            },
                            {
                                let events = events.clone();
                                let run = run.clone();
                                move |context| {
                                    emit_model_attempt_receipt(
                                        &events,
                                        &run,
                                        None,
                                        Some(&context),
                                        "retrying",
                                    );
                                    incidents_for_retry.observe_retry(context);
                                }
                            },
                        ),
                    )
                    .await;

                let Ok(invalid) = &response else {
                    break response;
                };
                if !force_tool_call || !invalid.tool_calls.is_empty() {
                    break response;
                }

                let payload = invalid
                    .response_metadata
                    .as_ref()
                    .and_then(|metadata| serde_json::to_value(metadata).ok())
                    .unwrap_or(Value::Null);
                emit_model_attempt_receipt(
                    &events,
                    &run,
                    invalid.response_metadata.as_ref(),
                    None,
                    "discarded",
                );
                let _ = events
                    .send(desktop::AgentEvent::Trace {
                        run: Some(run.clone()),
                        source: "model_tool_protocol_recovery".to_string(),
                        payload: json!({
                            "contract": "structured_tool_protocol",
                            "tool_choice": "auto",
                            "repair_attempt": repair_attempts,
                            "response": payload,
                        }),
                    })
                    .await;
                if let Some(usage) = invalid.usage {
                    discarded_usage.push(usage);
                }
                stream_progress.reset_attempt();
                if repair_attempts == 0 {
                    repair_attempts = 1;
                    messages.push(crate::llm::ChatMessage::system(
                        "Your previous response did not call a structured tool and was discarded because this turn still needs an action or typed delivery. Call exactly one appropriate available tool now. Do not return a prose-only response.",
                    ));
                    continue;
                }
                if repair_attempts == 1 {
                    repair_attempts = 2;
                    messages.push(crate::llm::ChatMessage::system(
                        "Your second prose-only response was also discarded. The completed work and tool receipts are preserved. Deliver the complete user-facing result now by calling final_answer exactly once.",
                    ));
                    continue;
                }
                break Err(crate::llm::LlmError::Provider(format!(
                    "{} model returned no structured tool call after broad auto guidance and a named final_answer repair attempt",
                    TOOL_PROTOCOL_RECOVERY_EXHAUSTED
                )));
            };

            for usage in discarded_usage {
                let mut usage = totals.add(usage);
                usage.context_limit = context_limit;
                let _ = events
                    .send(desktop::AgentEvent::RunUsageUpdated {
                        run: run.clone(),
                        usage,
                    })
                    .await;
            }

            match turn {
                Ok(mut turn) => {
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
                        } else {
                            stream_progress.finish_ordinary_turn(&tx, &turn.text);
                        }
                    }
                    // If a response ended while the stream filter was holding
                    // a possible tag prefix, release it. A malformed marker
                    // must remain visible rather than disappearing.
                    stream_progress.finish_filter(&tx);
                    incidents.mark_recovered();
                    if let Some(metadata) = &turn.response_metadata {
                        emit_model_attempt_receipt(&events, &run, Some(metadata), None, "accepted");
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
                    // Some reasoning models over compatible gateways end a turn with
                    // its whole output in the OpenRouter `reasoning` field —
                    // empty `content`, no `tool_calls`, `finish_reason: stop`.
                    // Our accumulator reads only `content`/`tool_calls`, so that
                    // lands here as a genuinely empty turn. Reporting it as a
                    // normal `Done` ends the run with nothing ("second message
                    // did nothing"). Surface it as a zero-output transport so
                    // agent-loop replays the turn with its built-in recovery
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
                        emit_model_attempt_receipt(
                            &events,
                            &run,
                            turn.response_metadata.as_ref(),
                            None,
                            "empty",
                        );
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
                        emit_model_attempt_receipt(
                            &events,
                            &run,
                            Some(metadata),
                            None,
                            "quarantined",
                        );
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
                        emit_model_attempt_receipt(&events, &run, None, Some(&context), "failed");
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
    pub vision: Option<AuxiliaryModelConfig>,
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
    /// they are provider-local outcomes, not `agent-loop` stream events.
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
        let static_terminator = self.exec.terminates_run();
        if static_terminator && !outcome.is_error {
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
        let terminates_run = static_terminator
            || outcome.signals.iter().any(|signal| {
                matches!(
                    signal,
                    ToolSignal::Goal(goal)
                        if goal.status == crate::loop_state::GoalStatus::Complete
                )
            });
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
                outcome.details["_agent_post_tool_original_content"] =
                    Value::String(outcome.content.clone());
                outcome.content = format!(
                    "PostToolUse hook rejected the tool result after execution: {reason}. Inspect canonical state and correct the effect before continuing."
                );
                outcome.is_error = true;
            } else if let Some(feedback) = post.feedback_message {
                if !outcome.details.is_object() {
                    outcome.details = json!({});
                }
                outcome.details["_agent_post_tool_original_content"] =
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
