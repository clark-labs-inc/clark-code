mod document_stream;
mod event_sink;
mod model_attempt_receipt;
mod partial_json;
mod proposed_plan_stream;
mod reasoning_stream;
pub(crate) mod redaction;
mod stream_progress;
mod streaming_tool_call;
mod tool_title;
mod tools;
mod translate;

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
use tool_title::tool_title;
use translate::*;

pub(crate) use event_sink::DesktopEventSink;
pub(crate) use tools::{desktop_tool_registry, DesktopToolRegistryOptions, ToolImagePolicy};
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
        // Pending effect receipts withhold answer text until the canonical
        // follow-up guard settles. They do not force the model's next action.
        let hold_completion_text = !session
            .lock()
            .await
            .effects
            .unresolved_diagnostics(&run)
            .is_empty();
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
            let stream_progress = StreamProgress::new(hold_completion_text);
            let turn = llm
                .stream_chat_with_observers(
                    &messages,
                    &tools,
                    &signal,
                    crate::llm::StreamObservers::new(
                        {
                            let stream_progress = stream_progress.clone();
                            move |delta: &str| stream_progress.observe_text(&chunk_tx, delta)
                        },
                        move |delta: &str| {
                            let _ = reasoning_tx.send(ca::StreamEvent::Chunk(
                                ca::AssistantStreamChunk::Reasoning {
                                    delta: delta.to_string(),
                                },
                            ));
                        },
                        {
                            let tool_tx = tx.clone();
                            let stream_progress = stream_progress.clone();
                            move |delta| stream_progress.observe_tool(&tool_tx, delta)
                        },
                        {
                            let events = events.clone();
                            let run = run.clone();
                            let incidents = incidents.clone();
                            move |context| {
                                emit_model_attempt_receipt(
                                    &events, &run, None, Some(&context), "retrying",
                                );
                                incidents.observe_retry(context);
                            }
                        },
                    ),
                )
                .await;

            match turn {
                Ok(turn) => {
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

#[cfg(test)]
#[path = "agent_adapter_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_adapter_translate_tests.rs"]
mod translate_tests;

#[cfg(test)]
#[path = "agent_adapter_reasoning_tests.rs"]
mod reasoning_tests;

#[cfg(test)]
#[path = "agent_adapter/projection_tests.rs"]
mod projection_tests;

#[cfg(test)]
#[path = "agent_adapter/document_tests.rs"]
mod document_tests;
