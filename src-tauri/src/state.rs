//! Host-side application state.
//!
//! Holds the live sessions — each with its own provider instance and projected
//! snapshot, so any number of conversations can stream concurrently. A
//! snapshot is the source of truth the webview renders for one session; prompt
//! streams fold events into it and the host emits it (tagged with its session
//! id via `Snapshot::session`) to the UI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use agent_core::{Provider, Session, Snapshot};
use tokio::sync::{Mutex, RwLock};

use crate::ssh::RemoteConn;
use crate::trajectory::CloudTrajectoryClient;

/// One live session: its provider instance, identity, and projected snapshot.
///
/// Each session owns a whole provider (providers are effectively
/// single-session), so sessions never contend — dropping the entry drops the
/// provider and with it any running agent loop.
pub struct HostSession {
    pub provider: Box<dyn Provider>,
    pub session: Session,
    pub snapshot: Snapshot,
    pub trajectory: Option<CloudTrajectoryClient>,
}

/// Shared, thread-safe host state managed by Tauri.
#[derive(Clone)]
pub struct AppState {
    /// A provider that has connected but not yet bound a session — the
    /// `provider_connect` → `session_new`/`session_load` handshake parks it
    /// here. The frontend serializes connects, so one slot suffices.
    pub pending_provider: Arc<Mutex<Option<Box<dyn Provider>>>>,
    /// Live sessions, keyed by the id the frontend addresses them with (the
    /// conversation id — see `session_new`'s `bind_id`). Per-entry locks so a
    /// streaming session never blocks the others.
    pub sessions: Arc<Mutex<HashMap<String, Arc<Mutex<HostSession>>>>>,
    /// Live remote-project connections, keyed by an id handed to the frontend.
    /// Holding a [`RemoteConn`] keeps its SSH channels (and the remote server +
    /// tunnel) alive; removing it tears them down.
    pub remotes: Arc<Mutex<HashMap<String, RemoteConn>>>,
    /// The app-wide Clark cloud JWT, shared by every trajectory client so a
    /// frontend refresh (via `update_cloud_token`) reaches in-flight retries.
    pub cloud_token: Arc<RwLock<Option<String>>>,
    /// Once set, no new provider runs may start. Existing runs retain their
    /// guards and drain normally before the updater installs and restarts.
    update_draining: Arc<AtomicBool>,
    /// Exact native run count. Snapshot-derived UI state can briefly lag a
    /// prompt start/end, so the updater gates on this counter at the process
    /// boundary instead.
    active_runs: Arc<AtomicUsize>,
}

/// RAII ownership for one provider run. Moving this guard into the stream task
/// makes every return/error path decrement the native active-run count.
pub(crate) struct ActiveRunGuard {
    active_runs: Arc<AtomicUsize>,
}

impl Drop for ActiveRunGuard {
    fn drop(&mut self) {
        self.active_runs.fetch_sub(1, Ordering::SeqCst);
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            pending_provider: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            remotes: Arc::new(Mutex::new(HashMap::new())),
            cloud_token: Arc::new(RwLock::new(None)),
            update_draining: Arc::new(AtomicBool::new(false)),
            active_runs: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The live entry for `session_id`, if any.
    pub async fn session_entry(&self, session_id: &str) -> Option<Arc<Mutex<HostSession>>> {
        self.sessions.lock().await.get(session_id).cloned()
    }

    /// Start the update drain and return the exact number of provider runs that
    /// still own the process. New runs are rejected until the drain is cancelled
    /// (installation failure) or the app restarts.
    pub fn begin_update_drain(&self) -> usize {
        self.update_draining.store(true, Ordering::SeqCst);
        self.active_runs.load(Ordering::SeqCst)
    }

    pub fn cancel_update_drain(&self) {
        self.update_draining.store(false, Ordering::SeqCst);
    }

    /// Atomically join the live-run set unless an update drain has begun. The
    /// second latch check closes the race with `begin_update_drain`.
    pub(crate) fn try_start_run(&self) -> Option<ActiveRunGuard> {
        if self.update_draining.load(Ordering::SeqCst) {
            return None;
        }
        self.active_runs.fetch_add(1, Ordering::SeqCst);
        if self.update_draining.load(Ordering::SeqCst) {
            self.active_runs.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(ActiveRunGuard {
            active_runs: self.active_runs.clone(),
        })
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;

    #[test]
    fn update_drain_rejects_new_runs_until_cancelled() {
        let state = AppState::new();
        let run = state.try_start_run().expect("first run should start");

        assert_eq!(state.begin_update_drain(), 1);
        assert!(state.try_start_run().is_none());

        drop(run);
        assert_eq!(state.begin_update_drain(), 0);

        state.cancel_update_drain();
        assert!(state.try_start_run().is_some());
    }
}
