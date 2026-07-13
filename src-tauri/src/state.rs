//! Host-side application state.
//!
//! Holds the live sessions — each with its own provider instance and projected
//! snapshot, so any number of conversations can stream concurrently. A
//! snapshot is the source of truth the webview renders for one session; prompt
//! streams fold events into it and the host emits it (tagged with its session
//! id via `Snapshot::session`) to the UI.

use std::collections::HashMap;
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
}

impl AppState {
    pub fn new() -> Self {
        Self {
            pending_provider: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            remotes: Arc::new(Mutex::new(HashMap::new())),
            cloud_token: Arc::new(RwLock::new(None)),
        }
    }

    /// The live entry for `session_id`, if any.
    pub async fn session_entry(&self, session_id: &str) -> Option<Arc<Mutex<HostSession>>> {
        self.sessions.lock().await.get(session_id).cloned()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
