//! Host-side application state.
//!
//! Holds the live provider, the active session, and the projected snapshot. The
//! snapshot is the single source of truth the webview renders; prompt streams
//! fold events into it and the host emits it to the UI.

use std::collections::HashMap;
use std::sync::Arc;

use agent_core::{Provider, Session, Snapshot};
use tokio::sync::Mutex;

use crate::ssh::RemoteConn;

/// One active provider + session. (Multi-session/tabs is a later phase.)
#[derive(Default)]
pub struct HostSession {
    pub provider: Option<Box<dyn Provider>>,
    pub session: Option<Session>,
    pub snapshot: Snapshot,
}

/// Shared, thread-safe host state managed by Tauri.
#[derive(Clone)]
pub struct AppState {
    pub session: Arc<Mutex<HostSession>>,
    /// Live remote-project connections, keyed by an id handed to the frontend.
    /// Holding a [`RemoteConn`] keeps its SSH channels (and the remote server +
    /// tunnel) alive; removing it tears them down.
    pub remotes: Arc<Mutex<HashMap<String, RemoteConn>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(HostSession::default())),
            remotes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
