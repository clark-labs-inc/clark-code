use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::trajectory::interrupt_live_runs;
use crate::AppState;

/// Close a live session without losing its terminal lifecycle transition.
///
/// Providers cancel asynchronously. Removing the session first would make the
/// stream task drop that cancellation, leaving the phone's cloud projection
/// indefinitely running. Write the shared interruption first, then stop and
/// remove the provider.
#[tauri::command]
pub async fn session_close(
    app: AppHandle,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!(session = %session_id, "session_close");
    let Some(entry) = state.session_entry(&session_id).await else {
        return Ok(());
    };
    let gate = entry.lock().await.projection_gate.clone();
    let _projection = gate.lock().await;

    // A reopen can replace the public conversation id while this call waits
    // for an in-flight projection. Never close the replacement session.
    let still_current = state
        .session_entry(&session_id)
        .await
        .is_some_and(|live| Arc::ptr_eq(&live, &entry));
    if !still_current {
        return Ok(());
    }

    let (provider_session_id, trajectory, terminal_events, snapshot) = {
        let mut live = entry.lock().await;
        if live.closing {
            return Ok(());
        }
        live.closing = true;
        let terminal_events = interrupt_live_runs(
            &mut live.snapshot,
            "desktop_session_closed",
            "Clark closed this session before the run finished. You can continue from the saved history.",
            "Clark closed this session before the goal finished. Continue from the saved history.",
        );
        (
            live.session.id.clone(),
            live.trajectory.clone(),
            terminal_events,
            live.snapshot.clone(),
        )
    };

    let mut terminal_error = None;
    if !terminal_events.is_empty() {
        if let Some(trajectory) = trajectory {
            match trajectory.append(&terminal_events).await {
                Ok(checkpoint) => {
                    let snapshot = {
                        let mut live = entry.lock().await;
                        live.snapshot.history_checkpoint = Some(checkpoint);
                        live.snapshot.clone()
                    };
                    let _ = app.emit("snapshot", snapshot);
                }
                Err(error) => {
                    terminal_error = Some(error);
                    let _ = app.emit(
                        "cloud-sync-warning",
                        "Clark stopped this session, but could not safely save its final state to Clark cloud.",
                    );
                    let _ = app.emit("snapshot", snapshot);
                }
            }
        } else {
            let _ = app.emit("snapshot", snapshot);
        }
    }

    let close_error = {
        let mut live = entry.lock().await;
        live.provider
            .close_session(&provider_session_id)
            .await
            .err()
            .map(|error| error.to_string())
    };
    let mut sessions = state.sessions.lock().await;
    if sessions
        .get(&session_id)
        .is_some_and(|live| Arc::ptr_eq(live, &entry))
    {
        sessions.remove(&session_id);
    }

    if let Some(error) = terminal_error {
        return Err(format!("save terminal session state: {error}"));
    }
    if let Some(error) = close_error {
        return Err(error);
    }
    Ok(())
}
