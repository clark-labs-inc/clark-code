use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use agent_core::provider::EventStream;
use agent_core::{apply, AgentEvent, RunId, SessionId};
use futures::FutureExt;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use super::stream_batch;
use crate::runtime_registry::SessionKey;
use crate::state::{ActiveRunGuard, HostSession};
use crate::AppState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DrainExit {
    Settled,
    UnexpectedEof,
}

pub(super) async fn cancel_after_host_interruption(
    entry: &Arc<Mutex<HostSession>>,
    session: &SessionId,
    run: &RunId,
    reason: &'static str,
) {
    let result = {
        let mut live = entry.lock().await;
        live.provider.cancel(session, run).await
    };
    match result {
        Ok(()) => {}
        Err(agent_core::Error::RunNotActive(_)) => {
            tracing::info!(
                session_id = %session,
                run_id = %run,
                reason,
                "provider run had already settled after a host interruption"
            );
        }
        Err(error) => {
            tracing::warn!(
                session_id = %session,
                run_id = %run,
                reason,
                %error,
                "provider cancellation after a host interruption failed"
            );
        }
    }
}

fn batch_finishes_run(events: &[AgentEvent], expected: &RunId) -> bool {
    events
        .iter()
        .any(|event| matches!(event, AgentEvent::RunFinished { run, .. } if run == expected))
}

fn record_conversation_diagnostics(conversation_id: &str, events: &[AgentEvent]) {
    for event in events {
        match event {
            AgentEvent::RunFinished { run, outcome } => {
                let failure_kind = outcome
                    .failure_kind
                    .as_ref()
                    .map(|kind| format!("{kind:?}"));
                if outcome.status == agent_core::RunStatus::Failed {
                    tracing::error!(
                        event = "conversation_run_finished",
                        conversation_id,
                        run_id = %run,
                        status = ?outcome.status,
                        failure_kind = failure_kind.as_deref().unwrap_or("none"),
                        stop_reason = outcome.stop_reason.as_deref().unwrap_or(""),
                        has_error = outcome.error.is_some(),
                        "conversation run failed"
                    );
                } else {
                    tracing::info!(
                        event = "conversation_run_finished",
                        conversation_id,
                        run_id = %run,
                        status = ?outcome.status,
                        stop_reason = outcome.stop_reason.as_deref().unwrap_or(""),
                        "conversation run finished"
                    );
                }
            }
            AgentEvent::Error { code, run, .. } => {
                tracing::error!(
                    event = "conversation_provider_error",
                    conversation_id,
                    run_id = run.as_ref().map(RunId::as_str).unwrap_or(""),
                    error_code = code,
                    "provider emitted a conversation error"
                );
            }
            _ => {}
        }
    }
}

/// Seal one provider-owned run after the native stream boundary has failed.
///
/// The caller owns the session's projection gate. The terminal transition is
/// appended before it is rendered when the trajectory is available; if local
/// persistence itself is unavailable, the in-memory projection still settles
/// so Stop does not remain permanently armed, and restart recovery retains the
/// last durable prefix.
async fn seal_interrupted_provider_run(
    app: &AppHandle,
    entry: &Arc<Mutex<HostSession>>,
    conversation_id: &str,
    run: &RunId,
    stop_reason: &str,
    error: &str,
    persistence_warning: &str,
) -> Result<(), String> {
    let (trajectory, terminal_events) = {
        let session = entry.lock().await;
        let mut projected = session.snapshot.clone();
        let events = crate::trajectory::interrupt_run(&mut projected, run, stop_reason, error);
        (session.trajectory.clone(), events)
    };
    if terminal_events.is_empty() {
        return Ok(());
    }

    let (checkpoint, persistence_error) = match trajectory {
        Some(trajectory) => match trajectory.append(&terminal_events).await {
            Ok(checkpoint) => (Some(checkpoint), None),
            Err(error) => (None, Some(error)),
        },
        None => (
            None,
            Some("product cloud trajectory is not configured".into()),
        ),
    };
    let snapshot = {
        let mut session = entry.lock().await;
        for event in &terminal_events {
            apply(&mut session.snapshot, event);
        }
        if let Some(checkpoint) = checkpoint {
            session.snapshot.history_checkpoint = Some(checkpoint);
        }
        session.snapshot.clone()
    };
    record_conversation_diagnostics(conversation_id, &terminal_events);
    crate::snapshot_emit::emit_snapshot(app, &snapshot);

    if let Some(persistence_error) = persistence_error {
        tracing::error!(
            event = "conversation_run_terminal_persistence_failed",
            conversation_id,
            run_id = %run,
            %persistence_error,
            "provider stream interruption was projected but not durably appended"
        );
        let _ = app.emit("cloud-sync-warning", persistence_warning);
        return Err(persistence_error);
    }
    Ok(())
}

/// Re-check session ownership and seal an unexpected stream exit. Superseded
/// and explicitly closing sessions already have a separate terminal authority.
async fn seal_current_provider_run(
    app: &AppHandle,
    state: &AppState,
    entry: &Arc<Mutex<HostSession>>,
    session_key: &SessionKey,
    run: &RunId,
    stop_reason: &str,
    error: &str,
    persistence_warning: &str,
) {
    let _account_lifecycle = state.account_lifecycle.read().await;
    seal_current_provider_run_while_account_held(
        app,
        state,
        entry,
        session_key,
        run,
        stop_reason,
        error,
        persistence_warning,
    )
    .await;
}

pub(super) async fn seal_current_provider_run_while_account_held(
    app: &AppHandle,
    state: &AppState,
    entry: &Arc<Mutex<HostSession>>,
    session_key: &SessionKey,
    run: &RunId,
    stop_reason: &str,
    error: &str,
    persistence_warning: &str,
) {
    let projection_gate = entry.lock().await.projection_gate.clone();
    let _projection = projection_gate.lock().await;
    let still_current = state
        .runtime_registry
        .current_session_entry(session_key)
        .await
        .is_some_and(|live| Arc::ptr_eq(&live, entry));
    if !still_current || entry.lock().await.closing {
        return;
    }
    if let Err(persistence_error) = seal_interrupted_provider_run(
        app,
        entry,
        session_key.as_str(),
        run,
        stop_reason,
        error,
        persistence_warning,
    )
    .await
    {
        tracing::error!(
            conversation_id = session_key.as_str(),
            run_id = %run,
            %persistence_error,
            "failed to persist provider stream interruption"
        );
    }
}

/// Persist and project the remainder of one provider-owned run stream. Prompt
/// and explicit compaction share this boundary so both get identical
/// write-ahead durability, stale-session rejection, and snapshot emission.
pub(super) fn spawn_provider_stream(
    app: AppHandle,
    state: AppState,
    entry: Arc<Mutex<HostSession>>,
    session_key: SessionKey,
    run: RunId,
    stream: EventStream,
    run_guard: ActiveRunGuard,
) {
    tokio::spawn(async move {
        let _run_guard = run_guard;
        let drain = AssertUnwindSafe(drain_provider_stream(
            &app,
            &state,
            &entry,
            &session_key,
            &run,
            stream,
        ))
        .catch_unwind()
        .await;
        match drain {
            Ok(DrainExit::UnexpectedEof) => {
                seal_current_provider_run(
                    &app,
                    &state,
                    &entry,
                    &session_key,
                    &run,
                    "provider_stream_closed",
                    "The local provider stream ended before it recorded a terminal run state. Saved work is preserved; continue from the last checkpoint.",
                    "Clark Code recovered an interrupted local run, but could not safely save its final state to product cloud.",
                )
                .await;
            }
            Ok(DrainExit::Settled) => {}
            Err(_) => {
                tracing::error!(
                    event = "conversation_provider_stream_panicked",
                    conversation_id = session_key.as_str(),
                    run_id = %run,
                    "native provider stream projection panicked"
                );
                seal_current_provider_run(
                    &app,
                    &state,
                    &entry,
                    &session_key,
                    &run,
                    "provider_stream_panicked",
                    "Clark Code's local provider stream stopped unexpectedly before it recorded a terminal run state. Saved work is preserved; continue from the last checkpoint.",
                    "Clark Code recovered a failed local stream, but could not safely save its final state to product cloud.",
                )
                .await;
            }
        }
    });
}

/// Drain a provider stream until its typed terminal event or host-owned exit.
async fn drain_provider_stream(
    app: &AppHandle,
    state: &AppState,
    entry: &Arc<Mutex<HostSession>>,
    session_key: &SessionKey,
    run: &RunId,
    mut stream: EventStream,
) -> DrainExit {
    while let Some(events) = stream_batch::next_event_batch(&mut stream).await {
        let _account_lifecycle = state.account_lifecycle.read().await;
        record_conversation_diagnostics(session_key.as_str(), &events);
        let specialist_projections = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Trace {
                    source, payload, ..
                } if source == "product_projection" => Some(payload.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        // A forced close owns the same gate, so a late cancellation event
        // cannot reopen the snapshot after its terminal transition.
        let projection_gate = entry.lock().await.projection_gate.clone();
        let _projection = projection_gate.lock().await;
        // Stop if this session was closed or superseded by a reopen: the
        // captured provider must never clobber a newer session with the
        // same public conversation id.
        let still_current = state
            .runtime_registry
            .current_session_entry(session_key)
            .await
            .is_some_and(|live| Arc::ptr_eq(&live, entry));
        if !still_current {
            return DrainExit::Settled;
        }
        let (trajectory, closing) = {
            let session = entry.lock().await;
            (session.trajectory.clone(), session.closing)
        };
        if closing {
            return DrainExit::Settled;
        }
        let Some(trajectory) = trajectory else {
            let _ = seal_interrupted_provider_run(
                    app,
                    entry,
                    session_key.as_str(),
                    run,
                    "provider_trajectory_missing",
                    "Clark Code lost the durable trajectory for this run before it finished. Saved work is preserved; continue from the last checkpoint.",
                    "Clark Code stopped an interrupted local run, but its product cloud trajectory is unavailable.",
                )
                .await;
            return DrainExit::Settled;
        };
        let checkpoint = match trajectory.append(&events).await {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                tracing::error!(%error, "local trajectory outbox append failed; interrupting projection");
                let provider_session = entry.lock().await.session.id.clone();
                cancel_after_host_interruption(
                    entry,
                    &provider_session,
                    run,
                    "trajectory_append_failed",
                )
                .await;
                let _ = seal_interrupted_provider_run(
                        app,
                        entry,
                        session_key.as_str(),
                        run,
                        "trajectory_append_failed",
                        "Clark Code could not persist the next provider event batch and stopped this run at the last saved point.",
                        "Clark Code could not safely save the next part of this run, so it stopped at the last saved point.",
                    )
                    .await;
                return DrainExit::Settled;
            }
        };
        let snapshot = {
            let mut session = entry.lock().await;
            for event in &events {
                apply(&mut session.snapshot, event);
            }
            session.snapshot.history_checkpoint = Some(checkpoint);
            session.snapshot.clone()
        };
        crate::snapshot_emit::emit_snapshot(app, &snapshot);
        for payload in specialist_projections {
            match state
                .product
                .publish_projection(
                    &payload,
                    crate::product::ProductRequestContext { app, state },
                )
                .await
            {
                Ok(Some(receipt)) => {
                    let _ = app.emit("specialist-projection-published", receipt);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "specialist overview publication failed");
                    let _ = app.emit(
                            "cloud-sync-warning",
                            format!(
                                "The specialist run is retained locally, but its overview could not be published: {error}"
                            ),
                        );
                }
            }
        }
        // RunFinished is the provider contract's terminal boundary. Do not
        // keep the native update guard alive waiting for a provider stream
        // that failed to close after it already told the UI the run settled.
        if batch_finishes_run(&events, run) {
            return DrainExit::Settled;
        }
    }
    DrainExit::UnexpectedEof
}

#[cfg(test)]
mod tests {
    use super::batch_finishes_run;
    use agent_core::{AgentEvent, RunId, RunOutcome, RunStatus};

    #[test]
    fn terminal_run_event_ends_the_native_drain_boundary() {
        let started = [AgentEvent::RunStarted {
            run: RunId::new("run-1"),
        }];
        assert!(!batch_finishes_run(&started, &RunId::new("run-1")));

        let finished = [AgentEvent::RunFinished {
            run: RunId::new("run-1"),
            outcome: RunOutcome {
                status: RunStatus::Done,
                stop_reason: None,
                error: None,
                failure_kind: None,
                usage: None,
                execution: None,
            },
        }];
        assert!(batch_finishes_run(&finished, &RunId::new("run-1")));
        assert!(!batch_finishes_run(&finished, &RunId::new("run-2")));
    }
}
