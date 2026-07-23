mod event_sink;
mod tool_title;
mod translate;

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

use crate::config::AgenticClarkConfig;
use crate::llm::LlmClient;
#[cfg(test)]
use crate::llm::{ChatContent, ContentPart};
use crate::loop_state::{RunControl, SessionState};
use crate::permissions::{PermissionGate, PermissionOutcome};
use crate::tools::{ProducedArtifact, ToolCtx, ToolExecutor, ToolRegistry, ToolSignal};

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
    incidents: crate::incidents::ProviderIncidentTracker,
}

impl ClarkAgentStream {
    pub fn new(llm: LlmClient, incidents: crate::incidents::ProviderIncidentTracker) -> Self {
        Self {
            llm,
            totals: Arc::new(UsageTotals::default()),
            incidents,
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
        let incidents = self.incidents.clone();
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
                .stream_chat_observed(
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
                    {
                        let incidents = incidents.clone();
                        move |context| incidents.observe_retry(context)
                    },
                )
                .await;

            match turn {
                Ok(turn) => {
                    incidents.mark_recovered();
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

pub(crate) struct DesktopToolRegistryOptions {
    pub session: Arc<Mutex<SessionState>>,
    pub control: Arc<Mutex<RunControl>>,
    pub session_id: SessionId,
    pub run: RunId,
    pub events: Sender<desktop::AgentEvent>,
    pub execution: Option<crate::root_execution::RootExecutionTrace>,
    pub image_policy: ToolImagePolicy,
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
            let post = crate::hooks::run_post_tool_use(
                self.ctx.executor.as_ref(),
                self.ctx.sandbox.root(),
                &hooks.post_tool_use,
                self.exec.name(),
                &args,
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

        // Cap what one tool result may occupy in the model's context. Without
        // this, a single huge read/grep/shell dump rides the transcript in
        // full until compaction. Middle-out: the head and tail survive, the
        // cut is labeled with the original size.
        if let Some(truncated) = crate::truncation::truncate_middle(
            &outcome.content,
            crate::truncation::DEFAULT_TOOL_RESULT_MAX_CHARS,
        ) {
            outcome.content = truncated;
        }
        let mut result = if outcome.is_error {
            ca::ToolResult::error(outcome.content)
        } else {
            ca::ToolResult::text(outcome.content)
        };
        if self.image_policy.native_image_support {
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
        Ok(result)
    }
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
