//! Native ownership for durable remote-worker runtimes.
//!
//! Account-partitioned session, project, worker, reconnect, claim, and skill
//! cache state lives here. The WebView receives only opaque handles; every use
//! re-authorizes them against the Agent Desktop account validated by the native host.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use agent_core::SessionId;
use code_remote::{RemoteWorker, RemoteWorkerInfo, RemoteWorkerSlot, RemoteWorkerSpec};
use tokio::sync::{Mutex, OwnedRwLockWriteGuard, RwLock};
use zeroize::Zeroizing;

use crate::state::HostSession;

const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(500);
const CIRCUIT_FAILURE_THRESHOLD: u8 = 3;
const CIRCUIT_OPEN_DURATION: Duration = Duration::from_secs(10);
const MAX_CIRCUIT_ENTRIES: usize = 128;

mod claims;
mod shutdown;

fn retryable_connect_error(error: &code_remote::RemoteWorkerError) -> bool {
    matches!(
        error,
        code_remote::RemoteWorkerError::Io(_)
            | code_remote::RemoteWorkerError::Artifact(_)
            | code_remote::RemoteWorkerError::Transport(_)
            | code_remote::RemoteWorkerError::Startup(_)
            | code_remote::RemoteWorkerError::Disconnected(_)
            | code_remote::RemoteWorkerError::Timeout(_)
            | code_remote::RemoteWorkerError::ProcessExit(_)
    )
}

async fn connect_worker(
    spec: RemoteWorkerSpec,
    credentials: HashMap<String, String>,
) -> Result<Arc<RemoteWorker>, code_remote::RemoteWorkerError> {
    let first = RemoteWorker::connect_with_credentials(spec.clone(), credentials.clone()).await;
    match first {
        Ok(worker) => Ok(Arc::new(worker)),
        Err(error) if retryable_connect_error(&error) => {
            tokio::time::sleep(CONNECT_RETRY_DELAY).await;
            RemoteWorker::connect_with_credentials(spec, credentials)
                .await
                .map(Arc::new)
        }
        Err(error) => Err(error),
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AccountKey(String);

impl AccountKey {
    pub(crate) fn new(owner_scope: impl Into<String>) -> Result<Self, String> {
        let owner_scope = owner_scope.into();
        if owner_scope.is_empty()
            || owner_scope.len() > 256
            || owner_scope.chars().any(char::is_control)
        {
            return Err("Agent Desktop account identity is invalid".into());
        }
        Ok(Self(owner_scope))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SessionKey(SessionId);

impl SessionKey {
    pub(crate) fn from_session(session_id: &SessionId) -> Result<Self, String> {
        Self::validate(session_id.as_str())?;
        Ok(Self(session_id.clone()))
    }

    pub(crate) fn parse(raw: impl Into<String>) -> Result<Self, String> {
        let session_id = SessionId::new(raw);
        Self::from_session(&session_id)
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn validate(raw: &str) -> Result<(), String> {
        if raw.is_empty() || raw.len() > 512 || raw.chars().any(char::is_control) {
            return Err("Agent Desktop session identity is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SessionPartitionKey {
    account: Option<AccountKey>,
    session: SessionKey,
}

impl SessionPartitionKey {
    fn new(account: Option<&AccountKey>, session: SessionKey) -> Self {
        Self {
            account: account.cloned(),
            session,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProjectKey(String);

impl ProjectKey {
    fn parse(raw: impl Into<String>) -> Result<Self, String> {
        let raw = raw.into();
        if raw.is_empty()
            || raw.len() > 128
            || !raw
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("remote project identity is invalid".into());
        }
        Ok(Self(raw))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// One indivisible, native-owned desktop account binding. Keeping the validated
/// origin, account identity, and bearer in the same lock prevents observers
/// from ever combining fields from two sign-in generations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CloudAccountState {
    pub(crate) rest_base: String,
    pub(crate) account: AccountKey,
    pub(crate) token: Zeroizing<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WorkerHandle(String);

impl WorkerHandle {
    fn generate() -> Self {
        Self(format!("worker-{}", uuid::Uuid::new_v4().simple()))
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        let suffix = raw
            .strip_prefix("worker-")
            .ok_or("remote worker handle is invalid")?;
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("remote worker handle is invalid".into());
        }
        Ok(Self(raw.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) struct WorkerRuntime {
    key: WorkerKey,
    account: AccountKey,
    project_id: ProjectKey,
    project_root: PathBuf,
    worker: Arc<RemoteWorkerSlot>,
    info: RemoteWorkerInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerConnectionKind {
    Started,
    Reused,
    Replaced,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct WorkerKey(String);

impl WorkerKey {
    fn from_spec(account: &AccountKey, spec: &RemoteWorkerSpec) -> Result<Self, String> {
        use sha2::{Digest, Sha256};

        let config = serde_json::to_vec(&spec.worker_config).map_err(|error| error.to_string())?;
        let mut digest = Sha256::new();
        for part in [
            account.0.as_bytes(),
            spec.host.as_bytes(),
            spec.project_id.as_bytes(),
            spec.remote_root.as_os_str().as_encoded_bytes(),
            spec.trajectory_root.as_os_str().as_encoded_bytes(),
            config.as_slice(),
        ] {
            digest.update((part.len() as u64).to_le_bytes());
            digest.update(part);
        }
        Ok(Self(format!("{:x}", digest.finalize())))
    }
}

#[derive(Clone)]
struct CircuitState {
    consecutive_failures: u8,
    opened_until: Option<Instant>,
    touched_at: Instant,
}

#[derive(Default)]
struct ConnectCircuits {
    states: HashMap<WorkerKey, CircuitState>,
}

impl ConnectCircuits {
    fn permit(&mut self, key: &WorkerKey, now: Instant) -> Result<(), String> {
        let Some(state) = self.states.get_mut(key) else {
            return Ok(());
        };
        state.touched_at = now;
        if state.opened_until.is_some_and(|until| until > now) {
            return Err(
                "remote worker reconnect is temporarily paused after repeated failures".into(),
            );
        }
        if state.opened_until.take().is_some() {
            state.consecutive_failures = 0;
        }
        Ok(())
    }

    fn record_failure(&mut self, key: WorkerKey, now: Instant) {
        let state = self.states.entry(key).or_insert(CircuitState {
            consecutive_failures: 0,
            opened_until: None,
            touched_at: now,
        });
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        state.touched_at = now;
        if state.consecutive_failures >= CIRCUIT_FAILURE_THRESHOLD {
            state.opened_until = Some(now + CIRCUIT_OPEN_DURATION);
        }
        while self.states.len() > MAX_CIRCUIT_ENTRIES {
            let Some(oldest) = self
                .states
                .iter()
                .min_by_key(|(_, state)| state.touched_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.states.remove(&oldest);
        }
    }

    fn record_success(&mut self, key: &WorkerKey) {
        self.states.remove(key);
    }
}

impl WorkerRuntime {
    pub(crate) fn project_id(&self) -> &ProjectKey {
        &self.project_id
    }

    pub(crate) fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub(crate) fn worker(&self) -> Arc<RemoteWorkerSlot> {
        self.worker.clone()
    }

    pub(crate) fn info(&self) -> &RemoteWorkerInfo {
        &self.info
    }
}

#[derive(Default)]
pub(crate) struct RuntimeRegistry {
    cloud_account: Arc<RwLock<Option<CloudAccountState>>>,
    command_claims: Mutex<HashMap<String, claims::CommandClaim>>,
    sessions: Mutex<HashMap<SessionPartitionKey, Arc<Mutex<HostSession>>>>,
    workers: Mutex<HashMap<WorkerHandle, Arc<WorkerRuntime>>>,
    handles_by_key: Mutex<HashMap<WorkerKey, WorkerHandle>>,
    connect_gates: Mutex<HashMap<WorkerKey, Weak<Mutex<()>>>>,
    connect_circuits: Mutex<ConnectCircuits>,
    skill_catalogs: Mutex<HashMap<Option<AccountKey>, Arc<provider_local::SkillCatalogService>>>,
}

impl RuntimeRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn cloud_account(&self) -> Option<CloudAccountState> {
        self.cloud_account.read().await.clone()
    }

    pub(crate) async fn current_skill_catalogs(&self) -> Arc<provider_local::SkillCatalogService> {
        let account = self
            .cloud_account
            .read()
            .await
            .as_ref()
            .map(|current| current.account.clone());
        self.skill_catalogs
            .lock()
            .await
            .entry(account)
            .or_insert_with(|| Arc::new(provider_local::SkillCatalogService::new()))
            .clone()
    }

    /// Exclusive account-generation transition. Readers block until the
    /// durable and live sides of a sign-in/sign-out transaction agree.
    pub(crate) async fn cloud_account_generation_write(
        &self,
    ) -> OwnedRwLockWriteGuard<Option<CloudAccountState>> {
        self.cloud_account.clone().write_owned().await
    }

    pub(crate) async fn set_cloud_account(&self, account: Option<CloudAccountState>) {
        *self.cloud_account_generation_write().await = account;
    }

    pub(crate) async fn current_session_entry(
        &self,
        session_id: &SessionKey,
    ) -> Option<Arc<Mutex<HostSession>>> {
        let account = self.cloud_account.read().await;
        let key = SessionPartitionKey::new(
            account.as_ref().map(|current| &current.account),
            session_id.clone(),
        );
        self.sessions.lock().await.get(&key).cloned()
    }

    pub(crate) async fn bind_session(
        &self,
        account: Option<AccountKey>,
        session_id: SessionKey,
        entry: Arc<Mutex<HostSession>>,
    ) -> Result<Option<Arc<Mutex<HostSession>>>, String> {
        if entry.lock().await.account != account {
            return Err("session account does not match its registry partition".into());
        }
        Ok(self.sessions.lock().await.insert(
            SessionPartitionKey::new(account.as_ref(), session_id),
            entry,
        ))
    }

    pub(crate) async fn remove_current_session_if_same(
        &self,
        session_id: &SessionKey,
        expected: &Arc<Mutex<HostSession>>,
    ) -> Option<Arc<Mutex<HostSession>>> {
        let account = self.cloud_account.read().await;
        let key = SessionPartitionKey::new(
            account.as_ref().map(|current| &current.account),
            session_id.clone(),
        );
        let mut sessions = self.sessions.lock().await;
        if sessions
            .get(&key)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            sessions.remove(&key)
        } else {
            None
        }
    }

    pub(crate) async fn session_entries(&self) -> Vec<(SessionKey, Arc<Mutex<HostSession>>)> {
        self.sessions
            .lock()
            .await
            .iter()
            .map(|(key, entry)| (key.session.clone(), entry.clone()))
            .collect()
    }

    /// Unpublish every session in this account before their providers are
    /// awaited. New commands fail closed as soon as this method returns.
    pub(crate) async fn take_account_sessions(
        &self,
        account: &AccountKey,
    ) -> Vec<Arc<Mutex<HostSession>>> {
        let mut sessions = self.sessions.lock().await;
        let owned = sessions
            .keys()
            .filter(|key| key.account.as_ref() == Some(account))
            .cloned()
            .collect::<Vec<_>>();
        let mut removed = Vec::new();
        for key in owned {
            if let Some(entry) = sessions.remove(&key) {
                removed.push(entry);
            }
        }
        removed
    }

    async fn connect_gate(&self, key: &WorkerKey) -> Arc<Mutex<()>> {
        let mut gates = self.connect_gates.lock().await;
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(key).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(Mutex::new(()));
        gates.insert(key.clone(), Arc::downgrade(&gate));
        gate
    }

    async fn connect_new_worker(
        &self,
        key: &WorkerKey,
        spec: RemoteWorkerSpec,
        credentials: HashMap<String, String>,
    ) -> Result<Arc<RemoteWorker>, String> {
        self.connect_circuits
            .lock()
            .await
            .permit(key, Instant::now())?;
        match connect_worker(spec, credentials).await {
            Ok(worker) => {
                self.connect_circuits.lock().await.record_success(key);
                Ok(worker)
            }
            Err(error) => {
                self.connect_circuits
                    .lock()
                    .await
                    .record_failure(key.clone(), Instant::now());
                tracing::warn!(
                    event = "remote_worker_bootstrap_failed",
                    category = error.category(),
                    "remote worker bootstrap failed"
                );
                Err(error.to_string())
            }
        }
    }

    pub(crate) async fn connect(
        &self,
        account: AccountKey,
        spec: RemoteWorkerSpec,
        credentials: HashMap<String, String>,
    ) -> Result<(WorkerHandle, Arc<WorkerRuntime>, WorkerConnectionKind), String> {
        spec.validate().map_err(|error| error.to_string())?;
        let project_id = ProjectKey::parse(spec.project_id.clone())?;
        let key = WorkerKey::from_spec(&account, &spec)?;
        let gate = self.connect_gate(&key).await;
        let _connecting = gate.lock().await;
        if let Some(handle) = self.handles_by_key.lock().await.get(&key).cloned() {
            if let Some(runtime) = self.workers.lock().await.get(&handle).cloned() {
                if runtime.worker.health_check().await.is_ok() {
                    return Ok((handle, runtime, WorkerConnectionKind::Reused));
                }
                let worker = self
                    .connect_new_worker(&key, spec.clone(), credentials)
                    .await?;
                let info = worker.info().clone();
                let old_worker = runtime.worker.replace(worker).await;
                let replacement = Arc::new(WorkerRuntime {
                    key,
                    account,
                    project_id,
                    project_root: spec.remote_root.clone(),
                    info,
                    worker: runtime.worker.clone(),
                });
                self.workers
                    .lock()
                    .await
                    .insert(handle.clone(), replacement.clone());
                let _ = old_worker.disconnect().await;
                return Ok((handle, replacement, WorkerConnectionKind::Replaced));
            }
        }
        let worker = self
            .connect_new_worker(&key, spec.clone(), credentials)
            .await?;
        let info = worker.info().clone();
        let runtime = Arc::new(WorkerRuntime {
            key: key.clone(),
            account,
            project_id,
            project_root: spec.remote_root.clone(),
            info,
            worker: Arc::new(RemoteWorkerSlot::new(worker)),
        });
        let handle = WorkerHandle::generate();
        self.workers
            .lock()
            .await
            .insert(handle.clone(), runtime.clone());
        self.handles_by_key.lock().await.insert(key, handle.clone());
        Ok((handle, runtime, WorkerConnectionKind::Started))
    }

    pub(crate) async fn resolve(
        &self,
        account: &AccountKey,
        handle: &WorkerHandle,
    ) -> Result<Arc<WorkerRuntime>, String> {
        let runtime = self
            .workers
            .lock()
            .await
            .get(handle)
            .cloned()
            .ok_or("remote worker is no longer available")?;
        if &runtime.account != account {
            return Err("remote worker belongs to a different desktop account".into());
        }
        Ok(runtime)
    }

    /// Remove the account partition before awaiting process shutdown. New
    /// lookups fail closed immediately and no registry mutex crosses an await.
    pub(crate) async fn disconnect_account(&self, account: &AccountKey) {
        self.remove_account_command_claims(account).await;
        self.skill_catalogs
            .lock()
            .await
            .remove(&Some(account.clone()));
        let removed = {
            let mut workers = self.workers.lock().await;
            let handles: Vec<_> = workers
                .iter()
                .filter(|(_, runtime)| &runtime.account == account)
                .map(|(handle, _)| handle.clone())
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| workers.remove(&handle))
                .collect::<Vec<_>>()
        };
        for runtime in removed {
            self.handles_by_key.lock().await.remove(&runtime.key);
            self.connect_circuits
                .lock()
                .await
                .record_success(&runtime.key);
            if let Err(error) = runtime.worker.disconnect().await {
                tracing::warn!(%error, "account worker shutdown failed");
            }
        }
    }

    pub(crate) async fn worker_count_for_account(&self, account: &AccountKey) -> usize {
        self.workers
            .lock()
            .await
            .values()
            .filter(|runtime| &runtime.account == account)
            .count()
    }

    #[cfg(test)]
    pub(crate) async fn worker_count(&self) -> usize {
        self.workers.lock().await.len()
    }
}

#[cfg(test)]
#[path = "runtime_registry/tests.rs"]
mod tests;
