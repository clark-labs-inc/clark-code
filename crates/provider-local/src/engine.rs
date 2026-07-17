//! Thin launcher from clark-desktop's provider API into `clark_agent::run`.

use std::sync::Arc;

use agent_core::domain::{AgentEvent, RunOutcome, RunStatus};
use agent_core::ids::{RunId, SessionId};
use async_channel::Sender;
use tokio::sync::Mutex;

use crate::agent_adapter::{desktop_tool_registry, ClarkAgentStream, DesktopEventSink};
use crate::compaction::{CheckpointCompactor, CompactionConfig};
use crate::llm::LlmClient;
use crate::loop_breaker::LoopBreaker;
use crate::loop_state::{RunControl, SessionState};
use crate::tools::{ToolCtx, ToolRegistry};

/// Turns of head-room before the hard `max_iterations` cap at which the
/// built-in graceful wrap-up fires. When crossed, the loop injects a
/// one-shot "stop and deliver your final result" steer, so a run that would
/// otherwise slam into the cap instead ends with a summary of what it did
/// and what's left (reported as a clean finish, not a failure). Sized
/// against the 1000-turn cap in [`crate::config`].
const GRACE_ITERATIONS: usize = 40;

/// Steering queue shared between the provider (`Provider::steer` pushes) and
/// the active run (clark-agent drains it between tool batches). A queue —
/// not a raw channel — because leftovers must be recoverable: when the run
/// ends before injecting a message (a terminal batch suppresses steering),
/// the engine folds the remainder into the session transcript so the next
/// turn still sees what the user said.
#[derive(Default)]
pub(crate) struct EngineSteering {
    queue: std::sync::Mutex<std::collections::VecDeque<clark_agent::AgentMessage>>,
}

impl EngineSteering {
    pub fn push_user_text(&self, text: String) {
        self.queue.lock().expect("steering queue lock").push_back(
            clark_agent::AgentMessage::User {
                content: clark_agent::UserContent::Text(text),
                timestamp: None,
            },
        );
    }

    fn drain_all(&self) -> Vec<clark_agent::AgentMessage> {
        self.queue
            .lock()
            .expect("steering queue lock")
            .drain(..)
            .collect()
    }
}

impl clark_agent::Plugin for EngineSteering {
    fn name(&self) -> &'static str {
        "desktop_steering"
    }
    fn capabilities(&self) -> clark_agent::PluginCapabilities {
        clark_agent::PluginCapabilities::steering()
    }
}

#[async_trait::async_trait]
impl clark_agent::SteeringSource for EngineSteering {
    async fn next_steering_messages(&self) -> Vec<clark_agent::AgentMessage> {
        self.drain_all()
    }
}

/// Everything `run_turn` needs, bundled to keep the spawned task signature sane.
pub(crate) struct TurnContext {
    pub llm: LlmClient,
    pub registry: Arc<ToolRegistry>,
    pub ctx: ToolCtx,
    pub session: Arc<Mutex<SessionState>>,
    pub control: Arc<Mutex<RunControl>>,
    pub session_id: SessionId,
    pub max_iterations: u32,
    pub compaction: CompactionConfig,
    pub model: String,
    pub temperature: Option<f32>,
    pub user_text: String,
    /// When memories are enabled: post-turn durable-fact extraction context.
    pub memory_extraction: Option<crate::memory_extraction::ExtractionCtx>,
}

/// Drive one user turn to completion, emitting normalized Desktop events into
/// `tx` while clark-agent owns the actual LLM/tool loop.
pub(crate) async fn run_turn(tc: TurnContext, tx: Sender<AgentEvent>, run: RunId) {
    let cancel = tc.ctx.cancel.clone();
    let _ = tx.send(AgentEvent::RunStarted { run: run.clone() }).await;

    match crate::checkpoint::create_checkpoint(tc.ctx.executor.as_ref(), tc.ctx.sandbox.root())
        .await
    {
        Ok(Some(id)) => {
            let _ = tx
                .send(AgentEvent::Checkpoint {
                    run: run.clone(),
                    id,
                })
                .await;
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "working-tree checkpoint unavailable"),
    }

    if cancel.is_cancelled() {
        finish(&tx, &run, RunStatus::Cancelled, None, None, None).await;
        return;
    }

    let tools = desktop_tool_registry(
        tc.registry.clone(),
        tc.ctx.clone(),
        tc.session.clone(),
        tc.control.clone(),
        tc.session_id.clone(),
        run.clone(),
        tx.clone(),
    );
    // Documents the agent writes into this workspace become inline artifacts.
    let docs_dir = tc.ctx.sandbox.docs_root().map(std::path::Path::to_path_buf);
    let sink = Arc::new(DesktopEventSink::new(
        tx.clone(),
        run.clone(),
        tc.registry.clone(),
        docs_dir,
    ));
    let completed_transcript = sink.completed_transcript();

    // The stream adapter accumulates token/cost usage across the run's model
    // calls; the handle folds those totals into the run outcome at finish.
    let stream = ClarkAgentStream::new(tc.llm.clone());
    let usage = stream.usage();
    // Breaks stuck same-action/same-result loops early (nudge → hard block)
    // so the raised iteration cap can't be burned on a spinning agent. One
    // instance, shared across the before- and after-tool-call hook lists.
    let loop_breaker = Arc::new(LoopBreaker::new());
    // Parallel batches: read-only tools in one assistant turn run concurrently
    // (Codex's model). Mutating tools set `requires_exclusive_sandbox`, which
    // downgrades their whole batch to sequential, so edits/shell keep today's
    // one-at-a-time ordering and the permission gate never faces two
    // simultaneous prompts.
    let steering = Arc::new(EngineSteering::default());
    let mut builder = clark_agent::AgentBuilder::new()
        .stream(Arc::new(stream))
        .tools(tools)
        .event_sink(sink)
        .default_execution_mode(clark_agent::ExecutionMode::Parallel)
        .max_iterations(tc.max_iterations as usize)
        .grace_iterations(GRACE_ITERATIONS)
        .before_tool_call_arc(loop_breaker.clone())
        .after_tool_call_arc(loop_breaker.clone())
        .model_id(tc.model.clone())
        .steering_arc(steering.clone())
        .context_transform(CheckpointCompactor::new(
            tc.llm.clone(),
            tc.compaction.clone(),
        ));
    if let Some(temperature) = tc.temperature {
        builder = builder.temperature(temperature);
    }
    let config = match builder.build() {
        Ok(config) => config,
        Err(error) => {
            let message = format!("failed to build local agent loop: {error}");
            let _ = tx
                .send(AgentEvent::Error {
                    code: "local_agent_config".into(),
                    message: message.clone(),
                    run: Some(run.clone()),
                })
                .await;
            finish(
                &tx,
                &run,
                RunStatus::Failed,
                None,
                Some(message),
                usage.snapshot(),
            )
            .await;
            return;
        }
    };

    let identity = clark_agent::RunIdentity::root()
        .with_run_id(run.as_str())
        .with_conversation_id(tc.session_id.as_str());
    // The turn consumes user_text; extraction needs its own copy afterwards.
    let extraction = tc.memory_extraction.map(|ctx| (ctx, tc.user_text.clone()));
    let prompt = clark_agent::AgentMessage::User {
        content: clark_agent::UserContent::Text(tc.user_text),
        timestamp: None,
    };

    // Expose the live steering queue: a user message sent while this run is
    // active is injected between tool batches instead of waiting for the end.
    tc.session.lock().await.steering = Some(steering.clone());

    // Drive the run, recovering ONCE from a context-window overflow: preserve
    // the progress the user already saw, force-compact the transcript, and
    // continue the same turn — instead of dying with "model_error" at the
    // window edge (which is what happens when the model's real window is
    // smaller than the compaction threshold assumes).
    let mut prompts = vec![prompt];
    let mut recovered_from_overflow = false;
    let run_result = loop {
        let context = {
            let session = tc.session.lock().await;
            clark_agent::AgentContext::new(session.system_prompt.clone())
                .with_messages(session.transcript.clone())
                .with_identity(identity.clone())
        };
        let attempt = if prompts.is_empty() {
            clark_agent::run_continue(context, &config, cancel.clone()).await
        } else {
            clark_agent::run(
                std::mem::take(&mut prompts),
                context,
                &config,
                cancel.clone(),
            )
            .await
        };
        match attempt {
            Err(error)
                if is_context_overflow(&error)
                    && !recovered_from_overflow
                    && !cancel.is_cancelled() =>
            {
                recovered_from_overflow = true;
                // Fold what completed so far into the transcript, then shrink.
                {
                    let mut session = tc.session.lock().await;
                    let progress = completed_transcript.drain();
                    session.transcript.extend(progress);
                }
                let snapshot = tc.session.lock().await.transcript.clone();
                match crate::compaction::force_compact(&tc.llm, &tc.compaction, &snapshot, &cancel)
                    .await
                {
                    Some(compacted) => {
                        tc.session.lock().await.transcript = compacted;
                        let _ = tx
                            .send(AgentEvent::MessageChunk {
                                run: run.clone(),
                                role: agent_core::domain::Role::System,
                                delta: agent_core::domain::ContentBlock::text(
                                    "The conversation hit the model's context limit — earlier \
                                     turns were summarized so this task can continue.",
                                ),
                            })
                            .await;
                        continue;
                    }
                    None => break Err(error),
                }
            }
            other => break other,
        }
    };
    // The queue is only valid while this run drives the loop. Anything still
    // queued (the user steered during the final batch, where injection is
    // suppressed) is folded into the transcript below so the next turn sees it.
    tc.session.lock().await.steering = None;
    let leftover_steering = steering.drain_all();

    let context_limit = crate::compaction::limit_of(&tc.compaction);
    match run_result {
        Ok(result) => {
            let outcome = result.outcome;
            {
                let mut session = tc.session.lock().await;
                session.transcript.extend(result.messages);
                session.transcript.extend(leftover_steering);
            }
            // Post-turn, off the latency path: extract durable facts the model
            // may not have saved itself. Detached — the turn never waits on it.
            if let Some((ctx, user_text)) = extraction {
                tokio::spawn(async move {
                    crate::memory_extraction::extract_and_store(ctx, &user_text).await;
                });
            }
            if outcome.is_complete() {
                finish(
                    &tx,
                    &run,
                    RunStatus::Done,
                    Some(outcome.label().to_string()),
                    None,
                    with_limit(usage.snapshot(), context_limit),
                )
                .await;
            } else {
                // Only `HitMaxIterations` lands here — a natural finish and
                // the graceful wrap-up both count as complete above. So this
                // is specifically "ran out of steps without wrapping up",
                // which almost always means a stuck approach. Say that, and
                // point at the saved transcript, instead of a bare count.
                finish(
                    &tx,
                    &run,
                    RunStatus::Failed,
                    Some(outcome.label().to_string()),
                    Some(format!(
                        "I hit my safety limit of {} steps before finishing — usually a sign I \
                         got stuck repeating an approach that wasn't working. Everything so far is \
                         saved above; send a follow-up to continue, or nudge me toward a different \
                         approach.",
                        tc.max_iterations
                    )),
                    with_limit(usage.snapshot(), context_limit),
                )
                .await;
            }
        }
        Err(error) => {
            // The core returns its message tail only on success. Preserve the
            // typed prompt/steering messages and every complete assistant/tool
            // turn observed before the failure so a follow-up continues from
            // the work the user can already see instead of starting from an
            // empty model transcript.
            {
                let mut session = tc.session.lock().await;
                session.transcript.extend(completed_transcript.drain());
                session.transcript.extend(leftover_steering);
                // Tell the model the turn was cut off (Codex records the same
                // marker): without it, the next turn continues as if the last
                // one finished cleanly, re-trusting steps that never ran.
                if matches!(error, clark_agent::LoopError::Aborted) {
                    session.transcript.push(clark_agent::AgentMessage::User {
                        content: clark_agent::UserContent::Text(
                            "[runtime note — the user stopped the previous turn before it \
                             finished; some of its steps may be incomplete. Take stock of the \
                             current state before continuing.]"
                                .to_string(),
                        ),
                        timestamp: None,
                    });
                }
            }
            let mapped = map_loop_error(error);
            if let Some((code, message)) = mapped.ui_error.clone() {
                let _ = tx
                    .send(AgentEvent::Error {
                        code,
                        message,
                        run: Some(run.clone()),
                    })
                    .await;
            }
            finish(
                &tx,
                &run,
                mapped.status,
                None,
                mapped.run_error,
                with_limit(usage.snapshot(), context_limit),
            )
            .await;
        }
    }
}

fn is_context_overflow(error: &clark_agent::LoopError) -> bool {
    matches!(
        error,
        clark_agent::LoopError::Stream(clark_agent::StreamError::ContextOverflow(_))
    )
}

/// Stamp the engine's auto-compaction threshold onto the run usage so the UI
/// can show an honest context meter (percent of the number that actually
/// triggers compaction, not a hardcoded guess).
fn with_limit(
    usage: Option<agent_core::domain::RunUsage>,
    limit: Option<u64>,
) -> Option<agent_core::domain::RunUsage> {
    usage.map(|mut usage| {
        usage.context_limit = limit;
        usage
    })
}

#[derive(Clone)]
struct MappedLoopError {
    status: RunStatus,
    run_error: Option<String>,
    ui_error: Option<(String, String)>,
}

fn map_loop_error(error: clark_agent::LoopError) -> MappedLoopError {
    match error {
        clark_agent::LoopError::Aborted => MappedLoopError {
            status: RunStatus::Cancelled,
            run_error: None,
            ui_error: None,
        },
        clark_agent::LoopError::Stream(stream) => map_stream_error(stream),
        clark_agent::LoopError::ToolFatal { tool, reason } => {
            let message = format!("fatal tool `{tool}` error: {reason}");
            MappedLoopError::failed("tool_fatal", message)
        }
        clark_agent::LoopError::InvalidContinuation(message) => {
            MappedLoopError::failed("local_agent_state", message)
        }
        clark_agent::LoopError::EmptyOutcomeBudgetExhausted { budget, observed } => {
            MappedLoopError::failed(
                "empty_agent_response",
                format!(
                    "empty assistant outcome retry budget exhausted: observed {observed}, budget {budget}"
                ),
            )
        }
    }
}

fn map_stream_error(error: clark_agent::StreamError) -> MappedLoopError {
    match error {
        clark_agent::StreamError::Fatal(message)
            if message.starts_with("insufficient_credits:") =>
        {
            MappedLoopError::failed("insufficient_credits", message)
        }
        clark_agent::StreamError::Transient(message)
        | clark_agent::StreamError::ProviderRateLimited(message)
        | clark_agent::StreamError::ZeroOutputTransport(message)
        | clark_agent::StreamError::Fatal(message)
        | clark_agent::StreamError::ContextOverflow(message) => {
            MappedLoopError::failed("model_error", message)
        }
        clark_agent::StreamError::Empty => MappedLoopError::failed(
            "model_error",
            "model returned an empty response".to_string(),
        ),
    }
}

impl MappedLoopError {
    fn failed(code: &str, message: String) -> Self {
        Self {
            status: RunStatus::Failed,
            run_error: Some(message.clone()),
            ui_error: Some((code.to_string(), message)),
        }
    }
}

async fn finish(
    tx: &Sender<AgentEvent>,
    run: &RunId,
    status: RunStatus,
    stop_reason: Option<String>,
    error: Option<String>,
    usage: Option<agent_core::domain::RunUsage>,
) {
    let _ = tx
        .send(AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status,
                stop_reason,
                error,
                usage,
            },
        })
        .await;
    tx.close();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clark_agent::SteeringSource;

    #[tokio::test]
    async fn steering_queue_injects_in_order_and_recovers_leftovers() {
        let steering = EngineSteering::default();
        steering.push_user_text("first".into());
        steering.push_user_text("second".into());

        // The loop drains via the SteeringSource seam…
        let drained = steering.next_steering_messages().await;
        assert_eq!(drained.len(), 2);
        let texts: Vec<_> = drained
            .iter()
            .map(|m| match m {
                clark_agent::AgentMessage::User {
                    content: clark_agent::UserContent::Text(t),
                    ..
                } => t.as_str(),
                other => panic!("expected user text, got {other:?}"),
            })
            .collect();
        assert_eq!(texts, vec!["first", "second"]);

        // …and anything left after the run ends is recoverable, not lost.
        steering.push_user_text("too late".into());
        assert_eq!(steering.drain_all().len(), 1);
        assert!(steering.drain_all().is_empty());
    }
}
