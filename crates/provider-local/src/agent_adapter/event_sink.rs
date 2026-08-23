//! Projection of typed agent-loop events into desktop events.

use std::collections::BTreeSet;
use std::sync::Arc;

use agent_core::domain as desktop;
use agent_core::ids::{RunId, ToolCallId};
use agent_loop as ca;
use async_channel::Sender;
use async_trait::async_trait;

/// Serialized transcript size detects compaction shrink even when the message
/// count stays flat after trading history for a summary and recent tail.
fn transcript_chars(messages: &[ca::AgentMessage]) -> usize {
    messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .sum()
}

use crate::tools::final_answer::{FINAL_ANSWER_DETAILS_KEY, FINAL_ANSWER_TOOL};
use crate::tools::ToolRegistry;

use super::{
    document_stream::{DocumentStreams, SettledDocument},
    final_answer_stream::FinalAnswerStreams,
    locations_from_details, markdown_artifact, mobile_screenshot_artifact, reasoning_stream,
    streaming_tool_call::{
        emit_execution_start, execution_update_event, streamed_tool_event, ExecutionStart,
        StreamingToolCalls,
    },
    tool_result_to_content,
};

pub(crate) struct DesktopEventSink {
    events: Sender<desktop::AgentEvent>,
    run: RunId,
    registry: Arc<ToolRegistry>,
    completed_transcript: CompletedRunTranscript,
    execution: Option<crate::root_execution::RootExecutionTrace>,
    /// Cached checkpoint application runs on every model request. Remember
    /// the summary at the checkpoint boundary so only a genuinely new
    /// checkpoint produces another user-visible notice.
    last_compaction_checkpoint: std::sync::Mutex<Option<String>>,
    final_answer_streams: std::sync::Mutex<FinalAnswerStreams>,
    document_streams: std::sync::Mutex<DocumentStreams>,
    streaming_tool_calls: std::sync::Mutex<StreamingToolCalls>,
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
            completed_transcript: CompletedRunTranscript::default(),
            execution: None,
            last_compaction_checkpoint: std::sync::Mutex::new(None),
            final_answer_streams: std::sync::Mutex::new(FinalAnswerStreams::default()),
            document_streams: std::sync::Mutex::new(DocumentStreams::default()),
            streaming_tool_calls: std::sync::Mutex::new(StreamingToolCalls::default()),
            docs_dir,
        }
    }

    pub fn with_execution(mut self, execution: crate::root_execution::RootExecutionTrace) -> Self {
        self.execution = Some(execution);
        self
    }

    pub fn completed_transcript(&self) -> CompletedRunTranscript {
        self.completed_transcript.clone()
    }

    fn mark_new_compaction_checkpoint(&self, after: &[ca::AgentMessage]) -> bool {
        let Some(signature) = after
            .first()
            .and_then(|message| serde_json::to_string(message).ok())
        else {
            return false;
        };
        let mut last = self
            .last_compaction_checkpoint
            .lock()
            .expect("compaction notice lock");
        if last.as_deref() == Some(signature.as_str()) {
            return false;
        }
        *last = Some(signature);
        true
    }
}

/// Canonical messages that reached a complete commit boundary during a run.
///
/// `agent_loop::run` returns its message tail only on success. On an error it
/// still emits typed lifecycle events, so retain the initial prompt/steering
/// messages plus complete `TurnEnd` assistant/tool-result groups. Assistant
/// `MessageEnd` alone is not a commit boundary: it can be a discarded
/// max-token attempt, a transport error, or a tool turn that never completed.
#[derive(Clone, Default)]
pub(crate) struct CompletedRunTranscript {
    messages: Arc<std::sync::Mutex<Vec<ca::AgentMessage>>>,
}

impl CompletedRunTranscript {
    fn observe(&self, event: &ca::AgentEvent) {
        let mut messages = self.messages.lock().expect("run transcript lock");
        match event {
            ca::AgentEvent::MessageEnd { message }
                if !matches!(message, ca::AgentMessage::Assistant { .. }) =>
            {
                messages.push(message.clone());
            }
            ca::AgentEvent::TurnEnd {
                message,
                tool_results,
            } if !matches!(
                message,
                ca::AgentMessage::Assistant {
                    stop_reason: ca::StopReason::Error | ca::StopReason::Aborted,
                    ..
                }
            ) =>
            {
                messages.push(message.clone());
                messages.extend(tool_results.iter().cloned());
            }
            _ => {}
        }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<ca::AgentMessage> {
        self.messages.lock().expect("run transcript lock").clone()
    }

    /// Whether this run already committed a user-visible final answer. A
    /// later hidden follow-up (for example effect verification) must not turn
    /// that answer into an "empty model response" if the follow-up itself
    /// produces no output.
    pub fn has_final_answer(&self) -> bool {
        self.messages
            .lock()
            .expect("run transcript lock")
            .iter()
            .any(|message| {
                matches!(
                    message,
                    ca::AgentMessage::Assistant {
                        content,
                        stop_reason: ca::StopReason::EndTurn,
                        ..
                    } if !content.plain_text().trim().is_empty()
                        && content.tool_calls().is_empty()
                )
            })
    }

    /// Take everything observed so far, leaving the tracker empty. The
    /// engine's overflow recovery folds progress into the session transcript
    /// mid-run; draining (instead of snapshotting) means a later fold after
    /// the retry can't duplicate the same messages.
    pub fn drain(&self) -> Vec<ca::AgentMessage> {
        std::mem::take(&mut *self.messages.lock().expect("run transcript lock"))
    }
}

#[async_trait]
impl ca::EventSink for DesktopEventSink {
    async fn emit(&self, event: ca::AgentEvent) {
        // Tool arguments remain redacted in durable traces, but the terminal
        // answer's private live bytes are needed briefly to render its content
        // stream. Capture only this typed delta before redaction and never
        // persist it outside the ordinary MessageChunk projection below.
        let private_tool_delta = match &event {
            ca::AgentEvent::MessageUpdate {
                chunk:
                    ca::AssistantStreamChunk::ToolCallDelta {
                        index,
                        id_delta,
                        name_delta,
                        arguments_delta,
                    },
                ..
            } => Some((
                *index,
                id_delta.clone(),
                name_delta.clone(),
                arguments_delta.clone(),
            )),
            _ => None,
        };
        let event = super::redaction::event(event);
        self.completed_transcript.observe(&event);
        if let Ok(payload) = serde_json::to_value(&event) {
            let _ = self
                .events
                .send(desktop::AgentEvent::Trace {
                    run: Some(self.run.clone()),
                    source: "agent_loop".to_string(),
                    payload,
                })
                .await;
        }
        match event {
            ca::AgentEvent::MessageStart {
                message: ca::AgentMessage::Assistant { .. },
            } => {
                self.final_answer_streams
                    .lock()
                    .expect("final answer stream lock")
                    .reset_message();
                self.streaming_tool_calls
                    .lock()
                    .expect("streaming tool calls lock")
                    .reset_message();
                self.document_streams
                    .lock()
                    .expect("document stream lock")
                    .reset_message();
            }
            // The in-loop compactor rewrote the model-visible transcript.
            // Surface it — a silent context rewrite reads as the agent
            // "forgetting" for no reason.
            ca::AgentEvent::ContextTransformApplied {
                plugin,
                before,
                after,
                ..
            } if plugin == "checkpoint_compactor"
                && transcript_chars(&after) < transcript_chars(&before) =>
            {
                if self.mark_new_compaction_checkpoint(&after) {
                    let _ = self
                        .events
                        .send(desktop::AgentEvent::MessageChunk {
                            run: self.run.clone(),
                            role: desktop::Role::System,
                            delta: desktop::ContentBlock::text(
                                "The conversation reached the model's context limit — earlier \
                                 turns were summarized so this task can continue.",
                            ),
                        })
                        .await;
                    let _ = self
                        .events
                        .send(desktop::AgentEvent::ContextCompacted {
                            run: self.run.clone(),
                            transcript: crate::resume::from_agent_messages(&after),
                        })
                        .await;
                }
            }
            ca::AgentEvent::MessageEnd {
                message:
                    ca::AgentMessage::Assistant {
                        content,
                        stop_reason: ca::StopReason::ToolUse,
                        ..
                    },
            } if !content.plain_text().trim().is_empty() && !content.tool_calls().is_empty() => {
                // The text and tool requests came from one provider response:
                // classify the streamed text before tool execution begins so
                // snapshots persist the provider-backed phase explicitly.
                let _ = self
                    .events
                    .send(desktop::AgentEvent::MessagePhase {
                        run: self.run.clone(),
                        phase: desktop::MessagePhase::Commentary,
                    })
                    .await;
            }
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
                    ca::AssistantStreamChunk::ToolCallDelta {
                        index,
                        id_delta,
                        name_delta,
                        arguments_delta,
                    },
                ..
            } => {
                let (index, id_delta, name_delta, arguments_delta) =
                    private_tool_delta.unwrap_or((index, id_delta, name_delta, arguments_delta));
                let delta = self
                    .final_answer_streams
                    .lock()
                    .expect("final answer stream lock")
                    .observe_delta(index, name_delta.as_deref(), arguments_delta.as_deref());
                if let Some(delta) = delta {
                    let _ = self
                        .events
                        .send(desktop::AgentEvent::MessageChunk {
                            run: self.run.clone(),
                            role: desktop::Role::Agent,
                            delta: desktop::ContentBlock::text(delta),
                        })
                        .await;
                }
                let document_delta = self
                    .document_streams
                    .lock()
                    .expect("document stream lock")
                    .observe_delta(
                        index,
                        id_delta.as_deref(),
                        name_delta.as_deref(),
                        arguments_delta.as_deref(),
                    );
                let announced = self
                    .streaming_tool_calls
                    .lock()
                    .expect("streaming tool calls lock")
                    .observe_delta(
                        index,
                        id_delta.as_deref(),
                        name_delta.as_deref(),
                        arguments_delta.as_deref(),
                    );
                let announced_id = announced.as_ref().map(|(id, _, _)| id.clone());
                if let Some((tool_call_id, tool_name, parsed_args)) = announced {
                    let _ = self
                        .events
                        .send(streamed_tool_event(
                            &self.run,
                            &self.registry,
                            tool_call_id,
                            tool_name,
                            parsed_args,
                        ))
                        .await;
                }
                // Ordered after the announce so the first delta patches a row
                // that exists. `announced_id` covers the same-delta case.
                if let Some(delta) = document_delta {
                    if let Some(id) = announced_id.or_else(|| {
                        self.streaming_tool_calls
                            .lock()
                            .expect("streaming tool calls lock")
                            .announced_id(index)
                    }) {
                        let _ = self
                            .events
                            .send(desktop::AgentEvent::ToolCallUpdate {
                                run: self.run.clone(),
                                id: ToolCallId::new(id),
                                patch: desktop::ToolCallPatch {
                                    append_input: delta,
                                    ..Default::default()
                                },
                            })
                            .await;
                    }
                }
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
            ca::AgentEvent::MessageUpdate {
                chunk: ca::AssistantStreamChunk::ReasoningDetails { delta },
                ..
            } => {
                reasoning_stream::emit_details(&self.events, &self.run, delta).await;
            }
            ca::AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                if tool_name == FINAL_ANSWER_TOOL {
                    self.final_answer_streams
                        .lock()
                        .expect("final answer stream lock")
                        .begin(&tool_call_id, &args);
                    return;
                }
                // Complete the streamed document from the validated arguments.
                // This still precedes the write, so it is the earliest point a
                // reader can have the whole thing. A faithful stream gets its
                // remainder appended; an untrusted one is replaced wholesale —
                // the reducer appends `append_input`, so appending a whole
                // payload onto a stale prefix would splice two drafts.
                let settled = self
                    .document_streams
                    .lock()
                    .expect("document stream lock")
                    .settle(&tool_call_id, &tool_name, &args);
                if let Some(settled) = settled {
                    let patch = match settled {
                        SettledDocument::Append(suffix) => desktop::ToolCallPatch {
                            append_input: suffix,
                            ..Default::default()
                        },
                        SettledDocument::Replace(payload) => desktop::ToolCallPatch {
                            replace_input: Some(payload),
                            ..Default::default()
                        },
                    };
                    let _ = self
                        .events
                        .send(desktop::AgentEvent::ToolCallUpdate {
                            run: self.run.clone(),
                            id: ToolCallId::new(tool_call_id.clone()),
                            patch,
                        })
                        .await;
                }
                emit_execution_start(
                    &self.events,
                    &self.run,
                    &self.registry,
                    self.execution.as_ref(),
                    &self.streaming_tool_calls,
                    ExecutionStart {
                        tool_call_id,
                        tool_name,
                        args,
                    },
                )
                .await;
            }
            ca::AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial,
                ..
            } => {
                let _ = self
                    .events
                    .send(execution_update_event(&self.run, tool_call_id, partial))
                    .await;
            }
            ca::AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
                ..
            } => {
                if tool_name == FINAL_ANSWER_TOOL {
                    if !is_error {
                        if let Some(answer) = result
                            .details
                            .get(FINAL_ANSWER_DETAILS_KEY)
                            .and_then(serde_json::Value::as_str)
                        {
                            let _ = self
                                .events
                                .send(desktop::AgentEvent::MessagePhase {
                                    run: self.run.clone(),
                                    phase: desktop::MessagePhase::FinalAnswer,
                                })
                                .await;
                            let suffix = self
                                .final_answer_streams
                                .lock()
                                .expect("final answer stream lock")
                                .finish(&tool_call_id, answer);
                            if !suffix.is_empty() {
                                let _ = self
                                    .events
                                    .send(desktop::AgentEvent::MessageChunk {
                                        run: self.run.clone(),
                                        role: desktop::Role::Agent,
                                        delta: desktop::ContentBlock::text(suffix),
                                    })
                                    .await;
                            }
                        }
                    }
                    return;
                }
                let locations = locations_from_details(&result.details);
                if let Some(execution) = &self.execution {
                    execution.tool_finished(
                        &tool_call_id,
                        if is_error {
                            agent_orchestration::ToolExecutionStatus::Failed
                        } else {
                            agent_orchestration::ToolExecutionStatus::Completed
                        },
                        locations
                            .iter()
                            .map(|location| location.path.clone())
                            .collect::<BTreeSet<_>>(),
                    );
                }
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
                            replace_content: Some(tool_result_to_content(&result)),
                            ..Default::default()
                        },
                    })
                    .await;
            }
            _ => {}
        }
    }
}
