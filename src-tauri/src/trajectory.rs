use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_core::AgentEvent;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::state::AppState;

mod cloud_boundary;
mod cloud_compaction;
mod outbox;
use cloud_boundary::{ProductTrajectoryCloudBoundary, TrajectoryCloudBoundary};
pub(crate) use outbox::{
    checkpoint_snapshot, delete_conversation, interrupt_live_runs, merge_local_summaries,
    migrate_legacy_database, quarantine_snapshot_branch, recover_snapshot, set_archived,
    wait_for_acknowledged_prefix,
};

const BUILD_GIT_SHA: &str = env!("CLARK_BUILD_GIT_SHA");
const BUILD_GIT_DIRTY: &str = env!("CLARK_BUILD_GIT_DIRTY");
const AUTH_REFRESH_WAIT: Duration = Duration::from_secs(60);
const CLOUD_AUTH_EXPIRED_PREFIX: &str = "cloud_auth_expired:";

pub(crate) fn build_git_sha() -> &'static str {
    BUILD_GIT_SHA
}

pub(crate) fn build_git_dirty() -> bool {
    BUILD_GIT_DIRTY == "true"
}

pub(crate) fn outbox_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("cloud-history-outbox.sqlite3"))
        .map_err(|error| format!("resolve cloud history outbox: {error}"))
}

/// Normalize snapshots written by the pre-checklist planning schema before
/// they cross a native deserialization boundary. Cloud history is durable
/// across desktop releases, so this must live here as well as in the WebView:
/// native recovery can inspect a cloud snapshot before TypeScript sees it.
///
/// The old wire form used a top-level `plan` and timeline items tagged
/// `{"item":"plan"}`. Current snapshots use `execution_checklist` for both.
/// Unknown legacy phase statuses are intentionally coerced to `pending`; an
/// old label must never make an otherwise readable conversation unopenable.
pub(crate) fn normalize_snapshot_value(snapshot: Value) -> Value {
    agent_core::normalize_snapshot_value(snapshot)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudTrajectoryConfig {
    pub title: String,
    pub provider: String,
    pub project: Option<String>,
    pub repository_fingerprint: Option<String>,
    pub remote_host: Option<String>,
    pub mode: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Clone)]
pub struct CloudTrajectoryClient {
    conversation_id: String,
    owner_scope: String,
    config: CloudTrajectoryConfig,
    /// Atomic native account state, read per attempt so refresh reaches every
    /// request without mixing a bearer and account from different generations.
    state: AppState,
    cloud: Arc<dyn TrajectoryCloudBoundary>,
    outbox: outbox::TrajectoryOutbox,
    flush_lock: Arc<Mutex<()>>,
    flush_scheduled: Arc<AtomicBool>,
    flush_retry_scheduled: Arc<AtomicBool>,
    flush_retry_attempt: Arc<AtomicUsize>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Conversation {
    title: String,
    provider: String,
    project: Option<String>,
    repository_fingerprint: Option<String>,
    remote_host: Option<String>,
    mode: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EventRecord {
    event_id: uuid::Uuid,
    run_id: Option<String>,
    event_kind: String,
    recorded_at_unix_ms: i64,
    payload: Value,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct AppendRequest {
    conversation: Conversation,
    events: Vec<EventRecord>,
}

impl CloudTrajectoryClient {
    pub fn new(
        conversation_id: String,
        config: CloudTrajectoryConfig,
        owner_scope: String,
        state: AppState,
        app: AppHandle,
        outbox_path: std::path::PathBuf,
    ) -> Result<Self, String> {
        let cloud = Arc::new(ProductTrajectoryCloudBoundary::new(app, state.clone()));
        Ok(Self::with_cloud_boundary(
            conversation_id,
            config,
            owner_scope,
            state,
            cloud,
            outbox_path,
        ))
    }

    fn with_cloud_boundary(
        conversation_id: String,
        config: CloudTrajectoryConfig,
        owner_scope: String,
        state: AppState,
        cloud: Arc<dyn TrajectoryCloudBoundary>,
        outbox_path: std::path::PathBuf,
    ) -> Self {
        let outbox = outbox::TrajectoryOutbox::new(outbox_path, &owner_scope, &conversation_id);
        Self {
            conversation_id,
            owner_scope,
            config,
            state,
            cloud,
            outbox,
            flush_lock: Arc::new(Mutex::new(())),
            flush_scheduled: Arc::new(AtomicBool::new(false)),
            flush_retry_scheduled: Arc::new(AtomicBool::new(false)),
            flush_retry_attempt: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub async fn initialize(
        &self,
        base: &agent_core::Snapshot,
        base_rev: i64,
    ) -> Result<(), String> {
        self.outbox.initialize(&self.config, base, base_rev).await?;
        // A prior process may have exited after SQLite commit but before cloud
        // acknowledgement. Stable event IDs make this replay harmless.
        self.trigger_flush();
        Ok(())
    }

    /// Durably enqueue events, then trigger a single-flight background cloud
    /// flush. The render path waits only for SQLite, never for network backoff.
    pub async fn append(&self, events: &[AgentEvent]) -> Result<i64, String> {
        if events.is_empty() {
            return Ok(0);
        }
        let first_timestamp = reserve_timestamps(events.len());
        let records = events
            .iter()
            .enumerate()
            .map(|(offset, event)| {
                let mut event_value = serde_json::to_value(event)
                    .map_err(|error| format!("serialize trajectory event: {error}"))?;
                let event_kind = event_kind(&event_value);
                cloud_compaction::compact_event(&event_kind, &mut event_value);
                let run_id = event_value
                    .get("run")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Ok(EventRecord {
                    event_id: uuid::Uuid::new_v4(),
                    run_id,
                    event_kind,
                    recorded_at_unix_ms: first_timestamp + offset as i64,
                    payload: json!({
                        "schemaVersion": 1,
                        "sessionId": self.conversation_id,
                        "appVersion": env!("CARGO_PKG_VERSION"),
                        "buildGitSha": build_git_sha(),
                        "buildGitDirty": build_git_dirty(),
                        "platform": std::env::consts::OS,
                        "arch": std::env::consts::ARCH,
                        "metadata": self.config.metadata.clone(),
                        "event": event_value,
                    }),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let request = AppendRequest {
            conversation: Conversation {
                title: self.config.title.clone(),
                provider: self.config.provider.clone(),
                project: self.config.project.clone(),
                repository_fingerprint: self.config.repository_fingerprint.clone(),
                remote_host: self.config.remote_host.clone(),
                mode: self.config.mode.clone(),
            },
            events: records,
        };
        let batch = self.outbox.enqueue(&request).await?;
        self.trigger_flush();
        Ok(batch.local_seq)
    }

    fn trigger_flush(&self) {
        if self.flush_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let client = self.clone();
        tokio::spawn(async move {
            let _single_flight = client.flush_lock.lock().await;
            let result = client.flush_pending().await;
            client.flush_scheduled.store(false, Ordering::SeqCst);
            if let Err(error) = result {
                tracing::warn!(%error, "cloud trajectory flush deferred");
                if error.starts_with("cloud_deleted:") {
                    client
                        .cloud
                        .emit_conversation_deleted(&client.conversation_id);
                } else if should_emit_cloud_sync_warning(&error) {
                    client.cloud.emit_sync_warning(
                        "Clark Code saved this run locally and will sync it when the cloud is reachable.",
                    );
                }
                if !error.starts_with("cloud_account_changed:")
                    && !error.starts_with("cloud_deleted:")
                {
                    client.schedule_flush_retry();
                }
            } else if client
                .outbox
                .pending()
                .await
                .is_ok_and(|pending| !pending.is_empty())
            {
                client.flush_retry_attempt.store(0, Ordering::SeqCst);
                client.trigger_flush();
            } else {
                client.flush_retry_attempt.store(0, Ordering::SeqCst);
            }
        });
    }

    /// A durable batch must eventually retry even when no later provider event
    /// arrives. Otherwise a recovered terminal snapshot can wait forever at the
    /// full-snapshot durability barrier after a temporary network outage.
    fn schedule_flush_retry(&self) {
        if self.flush_retry_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let attempt = self.flush_retry_attempt.fetch_add(1, Ordering::SeqCst);
        let delay = Duration::from_secs((2_u64.saturating_pow(attempt.min(4) as u32)).min(30));
        let client = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            client.flush_retry_scheduled.store(false, Ordering::SeqCst);
            if client
                .outbox
                .pending()
                .await
                .is_ok_and(|pending| !pending.is_empty())
            {
                client.trigger_flush();
            }
        });
    }

    async fn flush_pending(&self) -> Result<(), String> {
        for batch in self.outbox.pending().await? {
            self.deliver(&batch.request).await?;
            self.outbox.acknowledge(&batch.batch_id).await?;
        }
        Ok(())
    }

    async fn deliver(&self, request: &AppendRequest) -> Result<(), String> {
        // Transient transport failures use only the short prefix of this
        // schedule. A 401 instead asks the frontend to refresh, then waits for
        // the native account generation to rotate before re-reading the token.
        let mut last_error = String::new();
        let mut auth_retry = false;
        for (attempt, delay) in [0_u64, 250, 1_000, 2_000, 3_000, 4_000]
            .into_iter()
            .enumerate()
        {
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            let account_generation = self.state.runtime_registry.cloud_account_generation();
            let current_account = self
                .state
                .runtime_registry
                .cloud_account()
                .await
                .filter(|account| account.account.as_str() == self.owner_scope)
                .ok_or_else(|| {
                    "cloud_account_changed: Clark Code signed out or changed accounts before this pending run could sync"
                        .to_string()
                })?;
            drop(current_account);
            match self.cloud.append(&self.conversation_id, request).await? {
                crate::commands::ProductCloudOutcome::Ok(_) => return Ok(()),
                crate::commands::ProductCloudOutcome::Unauthorized(error) => {
                    last_error = format!("{CLOUD_AUTH_EXPIRED_PREFIX} {error}");
                    if auth_retry {
                        return Err(last_error);
                    }
                    auth_retry = true;
                    self.cloud.emit_auth_expired();
                    if !self
                        .state
                        .runtime_registry
                        .wait_for_cloud_account_generation_change(
                            account_generation,
                            AUTH_REFRESH_WAIT,
                        )
                        .await
                    {
                        return Err(last_error);
                    }
                }
                crate::commands::ProductCloudOutcome::NotFound(_) => {
                    return Err(
                        "cloud_deleted: this conversation was deleted on another device".into(),
                    );
                }
                crate::commands::ProductCloudOutcome::Conflict(error)
                | crate::commands::ProductCloudOutcome::Rejected(error) => return Err(error),
                crate::commands::ProductCloudOutcome::Unavailable(error) => {
                    last_error = error;
                }
            }
            if !auth_retry && attempt == 2 {
                break;
            }
        }
        Err(last_error)
    }
}

static LAST_EVENT_TIMESTAMP_MS: AtomicI64 = AtomicI64::new(0);

fn reserve_timestamps(count: usize) -> i64 {
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let width = i64::try_from(count.max(1)).unwrap_or(i64::MAX);
    loop {
        let last = LAST_EVENT_TIMESTAMP_MS.load(Ordering::Relaxed);
        let first = wall.max(last.saturating_add(1));
        let next = first.saturating_add(width.saturating_sub(1));
        if LAST_EVENT_TIMESTAMP_MS
            .compare_exchange(last, next, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            return first;
        }
    }
}

fn event_kind(event: &Value) -> String {
    let outer = event
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if outer != "trace" {
        return outer.to_string();
    }
    let source = event
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("provider");
    let inner = event
        .pointer("/payload/type")
        .and_then(Value::as_str)
        .unwrap_or("event");
    format!("trace.{source}.{inner}")
}

fn should_emit_cloud_sync_warning(error: &str) -> bool {
    !error.starts_with(CLOUD_AUTH_EXPIRED_PREFIX) && !error.starts_with("cloud_account_changed:")
}

#[cfg(test)]
mod auth_refresh_e2e;

#[cfg(test)]
mod tests {
    use super::{
        build_git_dirty, build_git_sha, event_kind, normalize_snapshot_value,
        should_emit_cloud_sync_warning,
    };
    use serde_json::json;

    #[test]
    fn trace_kind_preserves_provider_event_type() {
        assert_eq!(
            event_kind(&json!({
                "event": "trace",
                "source": "agent_loop",
                "payload": {"type": "context_transform_applied"}
            })),
            "trace.agent_loop.context_transform_applied"
        );
    }

    #[test]
    fn build_provenance_is_always_available_to_trajectory_envelopes() {
        let revision = build_git_sha();
        assert!(
            revision == "unknown"
                || (revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
        );
        assert_eq!(build_git_dirty(), env!("CLARK_BUILD_GIT_DIRTY") == "true");
    }

    #[test]
    fn auth_refresh_does_not_masquerade_as_a_cloud_outage() {
        assert!(!should_emit_cloud_sync_warning(
            "cloud_auth_expired: request returned 401"
        ));
        assert!(!should_emit_cloud_sync_warning(
            "cloud_account_changed: signed out"
        ));
        assert!(should_emit_cloud_sync_warning(
            "Clark cloud transport failed: connection reset"
        ));
    }

    #[test]
    fn legacy_plan_snapshot_is_normalized_before_deserialization() {
        let normalized = normalize_snapshot_value(json!({
            "runs": {},
            "timeline": [{
                "item": "plan",
                "run": "run-1",
                "plan": {"phases": [
                    {"title": "Inspect", "status": "done", "priority": "high"},
                    {"title": "Implement", "status": "in-progress"}
                ]}
            }],
            "plan": {"phases": [{"title": "Inspect", "status": "completed"}]},
            "tool_calls": {},
            "artifacts": []
        }));

        assert_eq!(normalized["timeline"][0]["item"], "execution_checklist");
        assert_eq!(
            normalized["timeline"][0]["checklist"]["steps"][0]["status"],
            "completed"
        );
        assert_eq!(
            normalized["timeline"][0]["checklist"]["steps"][1]["status"],
            "in_progress"
        );
        assert!(normalized.get("plan").is_none());
        assert!(serde_json::from_value::<agent_core::Snapshot>(normalized).is_ok());
    }

    #[test]
    fn legacy_plan_without_phases_does_not_block_session_open() {
        let normalized = normalize_snapshot_value(json!({
            "runs": {},
            "timeline": [{"item": "plan"}],
            "plan": {},
            "tool_calls": {},
            "artifacts": []
        }));

        assert_eq!(normalized["timeline"][0]["checklist"]["steps"], json!([]));
        assert!(serde_json::from_value::<agent_core::Snapshot>(normalized).is_ok());
    }
}
