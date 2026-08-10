//! Host-side application state.
//!
//! Holds the live sessions — each with its own provider instance and projected
//! snapshot, so any number of conversations can stream concurrently. A
//! snapshot is the source of truth the webview renders for one session; prompt
//! streams fold events into it and the host emits it (tagged with its session
//! id via `Snapshot::session`) to the UI.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use agent_core::{Provider, Session, Snapshot};
use tokio::sync::{Mutex, RwLock};

use crate::product::{NeutralProduct, ProductIntegration};
use crate::runtime_registry::{AccountKey, RuntimeRegistry};
use crate::session_credentials::SessionCredentials;
use crate::trajectory::CloudTrajectoryClient;

/// One live session: its provider instance, identity, and projected snapshot.
///
/// Each session owns a whole provider (providers are effectively
/// single-session), so sessions never contend — dropping the entry drops the
/// provider and with it any running agent loop.
pub struct HostSession {
    /// Native-validated desktop account that owns this session. `None` is valid
    /// only before a local session is attached to cloud history.
    pub(crate) account: Option<AccountKey>,
    pub provider: Box<dyn Provider>,
    pub session: Session,
    pub snapshot: Snapshot,
    pub trajectory: Option<CloudTrajectoryClient>,
    /// Serializes snapshot projection with an explicit close. Without this,
    /// an asynchronous provider cancellation can append a stale event after a
    /// terminal close transition has been written.
    pub projection_gate: Arc<Mutex<()>>,
    /// A close owns the terminal transition; later provider stream batches must
    /// be ignored rather than reopening the rendered run.
    pub closing: bool,
}

/// Shared, thread-safe host state managed by Tauri.
#[derive(Clone)]
pub struct AppState {
    pub(crate) product: Arc<dyn ProductIntegration>,
    /// Sole authority for live sessions, projections, trajectory clients,
    /// account-bound durable workers, skill caches, and reconnect state. The
    /// WebView can hold only public conversation ids and opaque capabilities.
    pub(crate) runtime_registry: Arc<RuntimeRegistry>,
    /// Account-partitioned secrets persisted in Clark Code's authenticated,
    /// app-encrypted private file. No operating-system credential vault is used.
    pub(crate) credentials: Arc<SessionCredentials>,
    /// Same-account work holds shared admission while sign-in/sign-out takes
    /// the exclusive transition. Independent requests never queue behind one
    /// another, but an account generation still changes atomically.
    pub(crate) account_lifecycle: Arc<RwLock<()>>,
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
        Self::with_product(Arc::new(NeutralProduct))
    }

    pub fn with_product(product: Arc<dyn ProductIntegration>) -> Self {
        let credential_policy = product.credential_envelope_policy();
        Self {
            product,
            runtime_registry: Arc::new(RuntimeRegistry::new()),
            credentials: Arc::new(SessionCredentials::with_policy(credential_policy)),
            account_lifecycle: Arc::new(RwLock::new(())),
            update_draining: Arc::new(AtomicBool::new(false)),
            active_runs: Arc::new(AtomicUsize::new(0)),
        }
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
