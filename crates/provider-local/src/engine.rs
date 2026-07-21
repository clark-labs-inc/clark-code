//! Thin launcher from clark-desktop's provider API into `clark_agent::run`.

mod error;

use std::sync::Arc;

use agent_core::domain::{AgentEvent, RunFailureKind, RunOutcome, RunStatus, RunUsage};
use agent_core::ids::{RunId, SessionId};
use agent_orchestration::FailureClass;
use async_channel::Sender;
use tokio::sync::Mutex;

use crate::agent_adapter::{
    desktop_tool_registry, ClarkAgentStream, DesktopEventSink, DesktopToolRegistryOptions,
    ToolImagePolicy,
};
use crate::compaction::{CheckpointCompactor, CompactionConfig};
use crate::llm::LlmClient;
use crate::loop_breaker::LoopBreaker;
use crate::loop_state::{GoalStatus, RunControl, SessionState};
use crate::root_execution::{RootExecutionConfig, RootExecutionTrace};
use crate::tools::{ToolCtx, ToolRegistry};
use error::map_loop_error;

/// Turns of head-room before the hard `max_iterations` cap at which the
/// built-in graceful wrap-up fires. When crossed, the loop injects a
/// one-shot "stop and deliver your final result" steer, so a run that would
/// otherwise slam into the cap instead ends with a summary of what it did
/// and what's left (reported as a clean finish, not a failure). Sized
/// against the 1000-turn cap in [`crate::config`].
const GRACE_ITERATIONS: usize = 40;

/// Hard cap on engine-launched goal-continuation turns within one run — the
/// circuit breaker against a goal that never converges. Each continuation is
/// itself bounded by `max_iterations` + the LoopBreaker, so this bounds the
/// outer autonomy loop, not the work inside a turn.
const MAX_GOAL_CONTINUATIONS: u32 = 24;

/// What the goal loop does after a cleanly completed iteration.
enum GoalStep {
    /// Launch another continuation turn with this prompt text.
    Continue { text: String, note: String },
    /// Stop the run and surface this note (cap reached).
    Stop(String),
}

/// Steering queue shared between the provider (`Provider::steer` pushes) and
/// the active run (clark-agent drains it between tool batches). A queue —
/// not a raw channel — because leftovers must be recoverable: when the run
/// ends before injecting a message (a terminal batch suppresses steering),
/// the engine folds the remainder into the session transcript so the next
/// turn still sees what the user said.
pub(crate) struct EngineSteering {
    queue: std::sync::Mutex<std::collections::VecDeque<clark_agent::AgentMessage>>,
    execution: Option<RootExecutionTrace>,
}

impl Default for EngineSteering {
    fn default() -> Self {
        Self {
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            execution: None,
        }
    }
}

impl EngineSteering {
    fn with_execution(execution: RootExecutionTrace) -> Self {
        Self {
            execution: Some(execution),
            ..Self::default()
        }
    }

    pub fn push_user_text(&self, text: String) {
        if let Some(execution) = &self.execution {
            execution.steering();
        }
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
    pub user_content: clark_agent::UserContent,
    /// Per-turn host instructions translated to provider `developer` messages.
    pub developer_instructions: Vec<clark_agent::AgentMessage>,
    /// Provider-owned state transitions that become visible at the start of
    /// this run, after `RunStarted` and before model output.
    pub initial_events: Vec<AgentEvent>,
    /// When memories are enabled: post-turn durable-fact extraction context.
    pub memory_extraction: Option<crate::memory_extraction::ExtractionCtx>,
    pub execution: RootExecutionConfig,
    pub run_cancellations: crate::provider::RunCancellationRegistry,
    pub tool_image_policy: ToolImagePolicy,
}

struct RootFinishContext {
    executor: Arc<dyn crate::exec::Executor>,
    root: std::path::PathBuf,
    session: Arc<Mutex<SessionState>>,
}

impl RootFinishContext {
    fn from_turn(tc: &TurnContext) -> Self {
        Self {
            executor: tc.ctx.executor.clone(),
            root: tc.ctx.sandbox.root().to_path_buf(),
            session: tc.session.clone(),
        }
    }
}

/// Drive one user turn to completion, emitting normalized Desktop events into
/// `tx` while clark-agent owns the actual LLM/tool loop.
pub(crate) async fn run_turn(tc: TurnContext, tx: Sender<AgentEvent>, run: RunId) {
    struct CancellationRegistration {
        registry: crate::provider::RunCancellationRegistry,
        run: RunId,
    }
    impl Drop for CancellationRegistration {
        fn drop(&mut self) {
            self.registry.remove(&self.run);
        }
    }
    let _cancellation_registration = CancellationRegistration {
        registry: tc.run_cancellations.clone(),
        run: run.clone(),
    };
    let cancel = tc.ctx.cancel.clone();
    let _ = tx.send(AgentEvent::RunStarted { run: run.clone() }).await;
    for event in tc.initial_events.iter().cloned() {
        let _ = tx.send(event).await;
    }
    // An explicit user turn resumes a previously blocked goal. Budget-limited
    // and complete goals remain terminal because only the user can grant more
    // runway or create the next goal.
    let starting_goal = {
        let mut session = tc.session.lock().await;
        session.goal.as_mut().map(|goal| {
            if goal.status == GoalStatus::Blocked {
                goal.status = GoalStatus::Active;
                goal.blocker_reason = None;
                goal.blocker_observations = 0;
                goal.last_blocker_continuation = None;
            }
            goal.touch();
            goal.state(Some(&run))
        })
    };
    if let Some(goal) = starting_goal {
        let _ = tx
            .send(AgentEvent::GoalUpdated {
                run: run.clone(),
                goal,
            })
            .await;
    }

    let execution = match RootExecutionTrace::new(&tc.session_id, &run, &tc.execution, tx.clone()) {
        Ok(execution) => execution,
        Err(error) => {
            let message = format!("failed to create root execution ledger: {error}");
            let _ = tx
                .send(AgentEvent::Error {
                    code: "local_execution_state".into(),
                    message: message.clone(),
                    run: Some(run.clone()),
                })
                .await;
            finish(
                &tx,
                &run,
                RunOutcome {
                    status: RunStatus::Failed,
                    stop_reason: None,
                    error: Some(message),
                    failure_kind: Some(RunFailureKind::LocalState),
                    usage: None,
                    execution: None,
                },
            )
            .await;
            return;
        }
    };
    tc.session.lock().await.active_execution = Some(execution.clone());
    let finish_context = RootFinishContext::from_turn(&tc);

    match crate::checkpoint::create_checkpoint(tc.ctx.executor.as_ref(), tc.ctx.sandbox.root())
        .await
    {
        Ok(Some(id)) => {
            execution.checkpoint(id.clone());
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
        finish_root(
            &tx,
            &run,
            &execution,
            &finish_context,
            RunStatus::Cancelled,
            None,
            None,
            None,
            None,
        )
        .await;
        return;
    }

    let tools = desktop_tool_registry(
        tc.registry.clone(),
        tc.ctx.clone(),
        DesktopToolRegistryOptions {
            session: tc.session.clone(),
            control: tc.control.clone(),
            session_id: tc.session_id.clone(),
            run: run.clone(),
            events: tx.clone(),
            execution: Some(execution.clone()),
            image_policy: tc.tool_image_policy.clone(),
        },
    );
    // Documents the agent writes into this workspace become inline artifacts.
    let docs_dir = tc.ctx.sandbox.docs_root().map(std::path::Path::to_path_buf);
    let sink = Arc::new(
        DesktopEventSink::new(tx.clone(), run.clone(), tc.registry.clone(), docs_dir)
            .with_execution(execution.clone()),
    );
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
    let steering = Arc::new(EngineSteering::with_execution(execution.clone()));
    // Keep the handle so request-time compaction can become canonical history.
    let compactor = CheckpointCompactor::new(tc.llm.clone(), tc.compaction.clone());
    let mut builder = clark_agent::AgentBuilder::new()
        .stream(Arc::new(stream))
        .tools(tools)
        .event_sink(sink)
        .default_execution_mode(clark_agent::ExecutionMode::Parallel)
        .max_iterations(tc.max_iterations as usize)
        .grace_iterations(GRACE_ITERATIONS)
        .before_tool_call_arc(loop_breaker.clone())
        .after_tool_call_arc(loop_breaker.clone())
        .tool_gate_arc(tc.registry.deferred_tool_gate(tc.session.clone()))
        .model_id(tc.model.clone())
        .steering_arc(steering.clone())
        .context_transform(compactor.clone())
        // Transparent context-window recovery: a provider overflow mid-run
        // force-compacts the live transcript and retries the same call
        // (clark-agent ≥0.2.2), any number of times at any iteration —
        // replacing the old engine-level once-per-run restart.
        .overflow_recovery(compactor.clone());
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
            finish_root(
                &tx,
                &run,
                &execution,
                &finish_context,
                RunStatus::Failed,
                None,
                Some(message),
                Some(RunFailureKind::LocalState),
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
        content: tc.user_content,
        timestamp: None,
    };

    // Expose the live steering queue: a user message sent while this run is
    // active is injected between tool batches instead of waiting for the end.
    tc.session.lock().await.steering = Some(steering.clone());

    // Drive the run and, when a session goal is active, CONTINUE it after each
    // clean completion with a goal-continuation turn (the Codex thread-goal
    // loop): the run keeps going until the model proves the goal complete, gets
    // blocked, or the budget runs out. Context-window overflows are recovered
    // transparently inside `clark_agent::run` (the checkpoint compactor hook
    // registered above), so there is no overflow bookkeeping here. Steering and
    // cancel keep working throughout — it is all one desktop run.
    let mut prompts = tc.developer_instructions;
    prompts.push(prompt);
    let mut accounted_usage: Option<RunUsage> = None;
    let run_result = 'goal: loop {
        let iteration_started = std::time::Instant::now();
        let usage_before = usage
            .snapshot()
            .map(|u| u.input_tokens + u.output_tokens)
            .unwrap_or(0);
        let attempt_result = loop {
            let context = {
                let session = tc.session.lock().await;
                clark_agent::AgentContext::new(session.system_prompt.clone())
                    .with_messages(session.transcript.clone())
                    .with_identity(identity.clone())
            };
            let result = if prompts.is_empty() {
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
            let Err(error) = result else {
                break result;
            };
            let Some((failure_class, failure_message)) = recovery_candidate(&error) else {
                break Err(error);
            };
            let permission_pending = tc.control.lock().await.has_pending();
            if cancel.is_cancelled()
                || permission_pending
                || !completed_transcript.has_commit_boundary()
                || !execution.can_recover(failure_class)
            {
                break Err(error);
            }

            let usage_now = usage.snapshot();
            execution.record_usage_delta(accounted_usage, usage_now);
            accounted_usage = usage_now;
            if !execution.schedule_recovery(failure_class, failure_message) {
                break Err(error);
            }
            {
                let mut session = tc.session.lock().await;
                compactor.commit_appended(&mut session.transcript, completed_transcript.drain());
                session.transcript.push(recovery_marker());
            }
            let _ = tx
                .send(AgentEvent::MessageChunk {
                    run: run.clone(),
                    role: agent_core::domain::Role::System,
                    delta: agent_core::domain::ContentBlock::text(
                        "A transient provider interruption ended at a safe tool boundary. \
                         Clark preserved completed work and is resuming once.",
                    ),
                })
                .await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        };

        match attempt_result {
            Ok(result) if result.outcome.is_complete() => {
                {
                    let mut session = tc.session.lock().await;
                    compactor.commit_appended(&mut session.transcript, result.messages);
                }
                // The completed-transcript observer saw the same messages the
                // loop just returned; reset it so a LATER failed continuation
                // can't fold duplicates into the transcript.
                let _ = completed_transcript.drain();

                // Goal bookkeeping: account this iteration's tokens/time, then
                // decide whether the run continues.
                let usage_now = usage
                    .snapshot()
                    .map(|u| u.input_tokens + u.output_tokens)
                    .unwrap_or(usage_before);
                let (next, goal_state) = {
                    let mut session = tc.session.lock().await;
                    let plan_mode = session.planning.plan_mode();
                    let next = match session.goal.as_mut() {
                        Some(goal) => {
                            goal.tokens_used += usage_now.saturating_sub(usage_before);
                            goal.time_used_seconds += iteration_started.elapsed().as_secs();
                            if goal.status != GoalStatus::Active
                                || plan_mode
                                || cancel.is_cancelled()
                            {
                                None
                            } else if goal.continuations >= MAX_GOAL_CONTINUATIONS {
                                goal.status = GoalStatus::Blocked;
                                goal.blocker_reason = Some(format!(
                                    "Reached the {MAX_GOAL_CONTINUATIONS}-continuation safety limit"
                                ));
                                Some(GoalStep::Stop(format!(
                                    "The goal ran for {MAX_GOAL_CONTINUATIONS} continuation \
                                     turns without completing — it is now marked blocked. \
                                     Review the progress above and send a message to continue."
                                )))
                            } else if goal
                                .token_budget
                                .is_some_and(|budget| goal.tokens_used >= budget)
                            {
                                goal.status = GoalStatus::BudgetLimited;
                                goal.continuations += 1;
                                Some(GoalStep::Continue {
                                    text: crate::prompt::goal_budget_limit_reminder(goal),
                                    note: format!(
                                        "Goal budget exhausted ({} tokens) — wrapping up.",
                                        goal.tokens_used
                                    ),
                                })
                            } else {
                                // Render BEFORE incrementing: the reminder
                                // numbers the turn it introduces.
                                let text = crate::prompt::goal_continuation_reminder(goal);
                                goal.continuations += 1;
                                let note = format!(
                                    "Goal turn {}: continuing toward the objective ({} \
                                     tokens used).",
                                    goal.continuations, goal.tokens_used
                                );
                                Some(GoalStep::Continue { text, note })
                            }
                        }
                        None => None,
                    };
                    if let Some(goal) = session.goal.as_mut() {
                        goal.touch();
                    }
                    let state = session.goal.as_ref().map(|goal| goal.state(Some(&run)));
                    (next, state)
                };
                if let Some(goal) = goal_state {
                    let _ = tx
                        .send(AgentEvent::GoalUpdated {
                            run: run.clone(),
                            goal,
                        })
                        .await;
                }
                match next {
                    Some(GoalStep::Continue { text, note }) => {
                        let _ = tx
                            .send(AgentEvent::MessageChunk {
                                run: run.clone(),
                                role: agent_core::domain::Role::System,
                                delta: agent_core::domain::ContentBlock::text(note),
                            })
                            .await;
                        prompts = vec![clark_agent::AgentMessage::User {
                            content: clark_agent::UserContent::Text(text),
                            timestamp: None,
                        }];
                        continue 'goal;
                    }
                    Some(GoalStep::Stop(note)) => {
                        let _ = tx
                            .send(AgentEvent::MessageChunk {
                                run: run.clone(),
                                role: agent_core::domain::Role::System,
                                delta: agent_core::domain::ContentBlock::text(note),
                            })
                            .await;
                        break 'goal Ok(result.outcome);
                    }
                    None => break 'goal Ok(result.outcome),
                }
            }
            Ok(result) => {
                // HitMaxIterations: fold what we have and stop any goal.
                {
                    let mut session = tc.session.lock().await;
                    compactor.commit_appended(&mut session.transcript, result.messages);
                    if let Some(goal) = session.goal.as_mut() {
                        if goal.status == GoalStatus::Active {
                            goal.status = GoalStatus::Blocked;
                            goal.blocker_reason =
                                Some("The agent loop reached its per-turn step limit".to_string());
                            goal.touch();
                        }
                    }
                }
                let goal_state = tc
                    .session
                    .lock()
                    .await
                    .goal
                    .as_ref()
                    .map(|goal| goal.state(Some(&run)));
                if let Some(goal) = goal_state {
                    let _ = tx
                        .send(AgentEvent::GoalUpdated {
                            run: run.clone(),
                            goal,
                        })
                        .await;
                }
                let _ = completed_transcript.drain();
                break 'goal Ok(result.outcome);
            }
            Err(error) => break 'goal Err(error),
        }
    };
    // The queue is only valid while this run drives the loop. Anything still
    // queued (the user steered during the final batch, where injection is
    // suppressed) is folded into the transcript below so the next turn sees it.
    tc.session.lock().await.steering = None;
    let leftover_steering = steering.drain_all();
    let final_usage = usage.snapshot();
    execution.record_usage_delta(accounted_usage, final_usage);

    let context_limit = crate::compaction::limit_of(&tc.compaction);
    match run_result {
        Ok(outcome) => {
            {
                let mut session = tc.session.lock().await;
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
                finish_root(
                    &tx,
                    &run,
                    &execution,
                    &finish_context,
                    RunStatus::Done,
                    Some(outcome.label().to_string()),
                    None,
                    None,
                    with_limit(final_usage, context_limit),
                )
                .await;
            } else {
                // Only `HitMaxIterations` lands here — a natural finish and
                // the graceful wrap-up both count as complete above. So this
                // is specifically "ran out of steps without wrapping up",
                // which almost always means a stuck approach. Say that, and
                // point at the saved transcript, instead of a bare count.
                finish_root(
                    &tx,
                    &run,
                    &execution,
                    &finish_context,
                    RunStatus::Failed,
                    Some(outcome.label().to_string()),
                    Some(format!(
                        "I hit my safety limit of {} steps before finishing — usually a sign I \
                         got stuck repeating an approach that wasn't working. Everything so far is \
                         saved above; send a follow-up to continue, or nudge me toward a different \
                         approach.",
                        tc.max_iterations
                    )),
                    Some(RunFailureKind::LocalState),
                    with_limit(final_usage, context_limit),
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
                let mut completed = completed_transcript.drain();
                completed.extend(leftover_steering);
                compactor.commit_appended(&mut session.transcript, completed);
                // A goal must never auto-continue into a wall: a failed run
                // blocks it (Codex does the same "to prevent automatic
                // continuation from looping and consuming tokens"). A user
                // cancel merely pauses pursuit — also expressed as Blocked,
                // resumed by the user's next explicit ask.
                if let Some(goal) = session.goal.as_mut() {
                    if goal.status == GoalStatus::Active {
                        goal.status = GoalStatus::Blocked;
                        goal.blocker_reason =
                            Some(if matches!(error, clark_agent::LoopError::Aborted) {
                                "The user stopped the run before it finished".to_string()
                            } else {
                                "The provider run failed before the goal finished".to_string()
                            });
                        goal.touch();
                    }
                }
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
            let goal_state = tc
                .session
                .lock()
                .await
                .goal
                .as_ref()
                .map(|goal| goal.state(Some(&run)));
            if let Some(goal) = goal_state {
                let _ = tx
                    .send(AgentEvent::GoalUpdated {
                        run: run.clone(),
                        goal,
                    })
                    .await;
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
            finish_root(
                &tx,
                &run,
                &execution,
                &finish_context,
                mapped.status,
                None,
                mapped.run_error,
                mapped.failure_kind,
                with_limit(final_usage, context_limit),
            )
            .await;
        }
    }
}

fn recovery_candidate(error: &clark_agent::LoopError) -> Option<(FailureClass, String)> {
    match error {
        clark_agent::LoopError::Stream(clark_agent::StreamError::Transient(message)) => {
            Some((FailureClass::TransientTransport, message.clone()))
        }
        clark_agent::LoopError::Stream(clark_agent::StreamError::ProviderRateLimited(message)) => {
            Some((FailureClass::RateLimited, message.clone()))
        }
        _ => None,
    }
}

fn recovery_marker() -> clark_agent::AgentMessage {
    clark_agent::AgentMessage::User {
        content: clark_agent::UserContent::Text(
            "[runtime recovery — the previous model stream failed after every started tool had a \
             terminal receipt. Completed transcript and current workspace state were preserved. \
             Re-read any state you depend on, do not repeat completed writes, and continue from \
             the current repository.]"
                .to_string(),
        ),
        timestamp: None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn finish_root(
    tx: &Sender<AgentEvent>,
    run: &RunId,
    execution: &RootExecutionTrace,
    context: &RootFinishContext,
    status: RunStatus,
    stop_reason: Option<String>,
    error: Option<String>,
    failure_kind: Option<RunFailureKind>,
    usage: Option<RunUsage>,
) {
    let terminal = match status {
        RunStatus::Done => agent_orchestration::ExecutionState::Completed,
        RunStatus::Cancelled => agent_orchestration::ExecutionState::Cancelled,
        RunStatus::Failed => agent_orchestration::ExecutionState::Failed,
        _ => agent_orchestration::ExecutionState::Blocked,
    };
    let reason = error.clone().or_else(|| stop_reason.clone());
    let execution_summary = execution
        .finalize(context.executor.as_ref(), &context.root, terminal, reason)
        .await;
    context.session.lock().await.active_execution = None;
    finish(
        tx,
        run,
        RunOutcome {
            status,
            stop_reason,
            error,
            failure_kind,
            usage,
            execution: Some(execution_summary),
        },
    )
    .await;
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

async fn finish(tx: &Sender<AgentEvent>, run: &RunId, outcome: RunOutcome) {
    let _ = tx
        .send(AgentEvent::RunFinished {
            run: run.clone(),
            outcome,
        })
        .await;
    tx.close();
}

#[cfg(test)]
include!("engine_tests.rs");
