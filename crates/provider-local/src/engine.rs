//! Thin launcher from agent-desktop's provider API into `agent_loop::run`.

mod error;
mod recovery;
mod steering;

pub(crate) use steering::EngineSteering;

use std::sync::Arc;

use agent_core::domain::{AgentEvent, RunFailureKind, RunOutcome, RunStatus, RunUsage};
use agent_core::ids::{RunId, SessionId};
use async_channel::Sender;
use tokio::sync::Mutex;

use crate::agent_adapter::{
    desktop_tool_registry, AgentLoopStream, DesktopEventSink, DesktopToolRegistryOptions,
    ToolImagePolicy,
};
use crate::compaction::{CheckpointCompactor, CompactionConfig};
use crate::llm::LlmClient;
use crate::loop_breaker::LoopProgressAdvisory;
use crate::loop_state::{GoalStatus, RunControl, SessionState};
use crate::root_execution::RootExecutionTrace;
use crate::tools::{ToolCtx, ToolRegistry};
use error::map_loop_error_with_completion_state;

/// What the goal loop does after a cleanly completed iteration.
enum GoalStep {
    /// Launch another continuation turn with this prompt text.
    Continue { text: String, note: String },
}

fn account_goal_iteration(
    goal: &mut crate::loop_state::SessionGoal,
    tokens: u64,
    elapsed_seconds: u64,
) {
    goal.tokens_used = goal.tokens_used.saturating_add(tokens);
    goal.time_used_seconds = goal.time_used_seconds.saturating_add(elapsed_seconds);
    goal.touch();
}

/// Everything `run_turn` needs, bundled to keep the spawned task signature sane.
pub(crate) struct TurnContext {
    pub llm: LlmClient,
    pub registry: Arc<ToolRegistry>,
    pub ctx: ToolCtx,
    pub session: Arc<Mutex<SessionState>>,
    pub control: Arc<Mutex<RunControl>>,
    pub session_id: SessionId,
    pub compaction: CompactionConfig,
    pub plan_execution_reminders: bool,
    pub hidden_plan_protocol: bool,
    /// Explicit Scout turns use a host-enforced tool and context boundary.
    pub scout_turn: bool,
    /// A per-turn system prompt when a workflow must exclude session-global
    /// context such as personal memory without mutating the durable session.
    pub turn_system_prompt: Option<String>,
    pub model: String,
    pub temperature: Option<f32>,
    pub user_text: String,
    pub user_content: agent_loop::UserContent,
    /// Per-turn host instructions translated to provider `developer` messages.
    pub developer_instructions: Vec<agent_loop::AgentMessage>,
    /// Provider-owned state transitions that become visible at the start of
    /// this run, after `RunStarted` and before model output.
    pub initial_events: Vec<AgentEvent>,
    /// When memories are enabled: post-turn durable-fact extraction context.
    pub memory_extraction: Option<crate::memory_extraction::ExtractionCtx>,
    pub run_cancellations: crate::provider::RunCancellationRegistry,
    pub tool_image_policy: ToolImagePolicy,
    pub runtime_plugin_packs: Vec<Arc<dyn crate::runtime_plugins::RuntimePluginPack>>,
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
/// `tx` while agent-loop owns the actual LLM/tool loop.
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
    // An explicit user turn resumes a previously blocked goal. Complete goals
    // remain terminal because only the user can create the next goal.
    let (starting_goal, completed_goal_id) = {
        let mut session = tc.session.lock().await;
        let completed = session
            .goal
            .as_ref()
            .filter(|goal| goal.status == GoalStatus::Complete)
            .map(|goal| goal.id.clone());
        let state = session.goal.as_mut().and_then(|goal| {
            if completed.is_some() {
                return None;
            }
            if goal.status == GoalStatus::Blocked {
                goal.status = GoalStatus::Active;
                goal.blocker_reason = None;
                goal.blocker_observations = 0;
                goal.last_blocker_continuation = None;
            }
            goal.touch();
            Some(goal.state(Some(&run)))
        });
        (state, completed)
    };
    let is_completed_before_run = |goal: &crate::loop_state::SessionGoal| {
        goal.status == GoalStatus::Complete
            && completed_goal_id.as_deref() == Some(goal.id.as_str())
    };
    if let Some(goal) = starting_goal {
        // The user may clear (or replace) the goal between this decision and
        // the emission. Re-check under the session lock so a stale receipt can
        // never resurrect a goal the user just removed.
        let still_current = tc
            .session
            .lock()
            .await
            .goal
            .as_ref()
            .is_some_and(|current| current.id == goal.id);
        if still_current {
            let _ = tx
                .send(AgentEvent::GoalUpdated {
                    run: run.clone(),
                    goal,
                })
                .await;
        }
    }

    let execution = match RootExecutionTrace::new(&tc.session_id, &run, tx.clone()) {
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

    if !tc.registry.tool_names().is_empty() {
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

    let hide_propose_plan = tc.hidden_plan_protocol && tc.session.lock().await.planning.plan_mode();
    let tools = Arc::new(desktop_tool_registry(
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
            hide_plan_mode_tools: hide_propose_plan,
        },
    ));
    // Documents the agent writes into this workspace become inline artifacts.
    let docs_dir = tc.ctx.sandbox.docs_root().map(std::path::Path::to_path_buf);
    let desktop_sink = Arc::new(
        DesktopEventSink::new(tx.clone(), run.clone(), tc.registry.clone(), docs_dir)
            .with_execution(execution.clone()),
    );
    let completed_transcript = desktop_sink.completed_transcript();
    let mut event_sinks = vec![desktop_sink.clone() as Arc<dyn agent_loop::EventSink>];
    for pack in &tc.runtime_plugin_packs {
        event_sinks.extend(pack.event_sinks());
    }
    let sink: Arc<dyn agent_loop::EventSink> = if event_sinks.len() == 1 {
        event_sinks.pop().expect("desktop event sink")
    } else {
        Arc::new(crate::runtime_plugins::CompositeEventSink::new(event_sinks))
    };

    // The stream adapter accumulates token/cost usage across the run's model
    // calls; it publishes cumulative usage after each call and the handle folds
    // the same authoritative totals into the run outcome at finish.
    let incidents = crate::incidents::ProviderIncidentTracker::new(run.clone(), tx.clone());
    let context_limit = crate::compaction::limit_of(&tc.compaction);
    let stream = AgentLoopStream::new(
        tc.llm.clone(),
        incidents.clone(),
        tx.clone(),
        tc.session.clone(),
        run.clone(),
        context_limit,
    );
    let usage = stream.usage();
    // Repeated same-action/same-result observations receive model-visible
    // progress advice, but never a heuristic host veto. Long-running polling
    // and external convergence remain governed by explicit cancellation and
    // typed tool/provider failures.
    let loop_advisory = Arc::new(LoopProgressAdvisory::new());
    // Parallel batches: read-only tools in one assistant turn run concurrently,
    // as expected by the local model. Mutating tools set `requires_exclusive_sandbox`, which
    // downgrades their whole batch to sequential, so edits/shell keep today's
    // one-at-a-time ordering and the permission gate never faces two
    // simultaneous prompts.
    let steering = Arc::new(EngineSteering::with_execution(execution.clone()));
    // Keep the handle so request-time compaction can become canonical history.
    let compactor = CheckpointCompactor::new(tc.llm.clone(), tc.compaction.clone());
    let mut builder = agent_loop::AgentBuilder::new()
        .stream(Arc::new(stream))
        .tools_arc(tools.clone())
        .event_sink(sink)
        .default_execution_mode(agent_loop::ExecutionMode::Parallel)
        .after_tool_call_arc(loop_advisory)
        .tool_gate_arc(tc.registry.deferred_tool_gate(tc.session.clone()))
        .follow_up(crate::effects::EffectCompletionGuard::new(
            tc.session.clone(),
            run.clone(),
        ))
        .model_id(tc.model.clone())
        .steering_arc(steering.clone())
        // Project every oversized re-fetchable result before the first model
        // request that can contain it. The complete event remains durable;
        // the model chooses whether a bounded result needs a narrower refetch.
        .context_transform(agent_loop::ToolResultBudget::new(tools))
        .context_transform(compactor.clone())
        // Transparent context-window recovery: a provider overflow mid-run
        // force-compacts the live transcript and retries the same call
        // (agent-loop ≥0.2.2), any number of times at any iteration —
        // replacing the old engine-level once-per-run restart.
        .overflow_recovery(compactor.clone());
    if tc.scout_turn {
        builder = builder.tool_gate_arc(Arc::new(crate::scout_policy::ScoutToolGate));
    }
    for pack in &tc.runtime_plugin_packs {
        for source in pack.steering_sources() {
            builder = builder.steering_arc(source);
        }
        for source in pack.follow_up_sources() {
            builder = builder.follow_up_arc(source);
        }
    }
    if tc.plan_execution_reminders {
        builder = builder
            .follow_up(crate::planning::PlanCompletionGuard::new(
                tc.session.clone(),
            ))
            // Register after compaction so approved-plan authority lands at
            // the recency edge of the exact provider request.
            .context_transform(crate::planning::PlanReminderTransform::new(
                tc.session.clone(),
            ));
    }
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

    let identity = agent_loop::RunIdentity::root()
        .with_run_id(run.as_str())
        .with_conversation_id(tc.session_id.as_str());
    // The turn consumes user_text; extraction needs its own copy afterwards.
    let extraction = tc.memory_extraction.map(|ctx| (ctx, tc.user_text.clone()));
    let prompt = agent_loop::AgentMessage::User {
        content: tc.user_content,
        timestamp: None,
    };

    // Expose the live steering queue: a user message sent while this run is
    // active is injected between tool batches instead of waiting for the end.
    tc.session.lock().await.steering = Some(steering.clone());

    // Drive the run and, when a session goal is active, CONTINUE it after each
    // clean completion with a goal-continuation turn (the thread-goal
    // loop): the run keeps going until the model proves the goal complete, gets
    // explicitly blocked, or the user cancels it. Context-window overflows are recovered
    // transparently inside `agent_loop::run` (the checkpoint compactor hook
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
                agent_loop::AgentContext::new(
                    tc.turn_system_prompt
                        .clone()
                        .unwrap_or_else(|| session.system_prompt.clone()),
                )
                .with_messages(session.transcript.clone())
                .with_identity(identity.clone())
            };
            let result = if prompts.is_empty() {
                agent_loop::run_continue(context, &config, cancel.clone()).await
            } else {
                agent_loop::run(
                    std::mem::take(&mut prompts),
                    context,
                    &config,
                    cancel.clone(),
                )
                .await
            };
            let error = match result {
                Ok(result) => break Ok(result),
                Err(error) => error,
            };
            let Some((failure_class, failure_message)) = recovery::candidate(&error) else {
                incidents.mark_failed();
                break Err(error);
            };
            let permission_pending = tc.control.lock().await.has_pending();
            if cancel.is_cancelled() || permission_pending || !execution.can_recover(failure_class)
            {
                incidents.mark_failed();
                break Err(error);
            }

            let usage_now = usage.snapshot();
            execution.record_usage_delta(accounted_usage, usage_now);
            accounted_usage = usage_now;
            let boundary = execution.recovery_boundary();
            if !execution.schedule_recovery(failure_class, failure_message.clone()) {
                incidents.mark_failed();
                break Err(error);
            }
            incidents.attach_execution_recovery(recovery::execution_recovery(boundary));
            let completed_background =
                recovery::await_background_completion(&tc.ctx.background, &cancel).await;
            {
                let mut session = tc.session.lock().await;
                compactor.commit_appended(&mut session.transcript, completed_transcript.drain());
                session
                    .transcript
                    .push(recovery::transcript_marker(&completed_background));
            }
            if completed_background.is_empty() {
                recovery::provider_backoff(&cancel).await;
            }
        };

        match attempt_result {
            Ok(result) => {
                {
                    let mut session = tc.session.lock().await;
                    compactor.commit_appended(
                        &mut session.transcript,
                        crate::agent_adapter::redaction::messages(result.messages),
                    );
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
                    let dormant = session.goal.as_ref().is_some_and(&is_completed_before_run);
                    let plan_mode = session.planning.plan_mode();
                    let next = match session.goal.as_mut() {
                        Some(_) if dormant => None,
                        Some(goal) => {
                            account_goal_iteration(
                                goal,
                                usage_now.saturating_sub(usage_before),
                                iteration_started.elapsed().as_secs(),
                            );
                            if goal.status != GoalStatus::Active
                                || plan_mode
                                || cancel.is_cancelled()
                            {
                                None
                            } else {
                                // Render BEFORE incrementing: the reminder
                                // numbers the turn it introduces.
                                let text = crate::prompt::goal_continuation_reminder(goal);
                                goal.continuations = goal.continuations.saturating_add(1);
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
                    if let Some(goal) = session.goal.as_mut().filter(|_| !dormant) {
                        goal.touch();
                    }
                    let state = if dormant {
                        None
                    } else {
                        session.goal.as_ref().map(|goal| goal.state(Some(&run)))
                    };
                    (next, state)
                };
                if let Some(goal) = goal_state {
                    let still_current = tc
                        .session
                        .lock()
                        .await
                        .goal
                        .as_ref()
                        .is_some_and(|current| current.id == goal.id);
                    if still_current {
                        let _ = tx
                            .send(AgentEvent::GoalUpdated {
                                run: run.clone(),
                                goal,
                            })
                            .await;
                    }
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
                        prompts = vec![agent_loop::AgentMessage::User {
                            content: agent_loop::UserContent::Text(text),
                            timestamp: None,
                        }];
                        continue 'goal;
                    }
                    None => break 'goal Ok(result.outcome),
                }
            }
            Err(error) => {
                // Failed iterations still consumed the user's time and model
                // usage. Record them before the terminal error path, otherwise
                // a long run can misleadingly collapse to 0s.
                let usage_now = usage
                    .snapshot()
                    .map(|u| u.input_tokens + u.output_tokens)
                    .unwrap_or(usage_before);
                let mut session = tc.session.lock().await;
                if let Some(goal) = session
                    .goal
                    .as_mut()
                    .filter(|goal| !is_completed_before_run(goal))
                {
                    account_goal_iteration(
                        goal,
                        usage_now.saturating_sub(usage_before),
                        iteration_started.elapsed().as_secs(),
                    );
                }
                break 'goal Err(error);
            }
        }
    };
    // The queue is only valid while this run drives the loop. Anything still
    // queued (the user steered during the final batch, where injection is
    // suppressed) is folded into the transcript below so the next turn sees it.
    tc.session.lock().await.steering = None;
    let leftover_steering = steering.drain_all();
    let final_usage = usage.snapshot();
    execution.record_usage_delta(accounted_usage, final_usage);

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
        }
        Err(error) => {
            // A post-tool final-answer request can fail after the model has
            // already committed a user-visible final answer or explicitly
            // completed this run's goal. Capture those receipts before
            // draining the transcript so the empty response is classified
            // against the completion state it followed.
            let final_answer_committed = completed_transcript.has_final_answer();
            let (unresolved_effects, goal_completed_this_run) = {
                let session = tc.session.lock().await;
                (
                    session.effects.unresolved_diagnostics(&run),
                    session.goal.as_ref().is_some_and(|goal| {
                        goal.status == GoalStatus::Complete && !is_completed_before_run(goal)
                    }),
                )
            };
            let aborted = matches!(&error, agent_loop::LoopError::Aborted);
            let mapped = map_loop_error_with_completion_state(
                error,
                final_answer_committed,
                goal_completed_this_run,
                &unresolved_effects,
            );
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
                // Run and goal lifecycles have separate authorities. A typed
                // run failure ends this attempt, but cannot bypass update_goal's
                // repeated-blocker contract and declare the standing objective
                // blocked after one observation. The goal remains resumable;
                // only explicit goal completion/blocking or a later user turn
                // changes its status.
                // Tell the model the turn was cut off and record that marker:
                // without it, the next turn continues as if the last
                // one finished cleanly, re-trusting steps that never ran.
                if aborted {
                    session.transcript.push(agent_loop::AgentMessage::User {
                        content: agent_loop::UserContent::Text(
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
                .filter(|goal| !is_completed_before_run(goal))
                .map(|goal| goal.state(Some(&run)));
            if let Some(goal) = goal_state {
                let still_current = tc
                    .session
                    .lock()
                    .await
                    .goal
                    .as_ref()
                    .is_some_and(|current| current.id == goal.id);
                if still_current {
                    let _ = tx
                        .send(AgentEvent::GoalUpdated {
                            run: run.clone(),
                            goal,
                        })
                        .await;
                }
            }
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
