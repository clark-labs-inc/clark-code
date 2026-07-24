use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_core::AgentEvent;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

use crate::commands::clark_rest_base;

mod outbox;
pub(crate) use outbox::{
    checkpoint_snapshot, delete_conversation, interrupt_live_runs, merge_local_summaries,
    quarantine_snapshot_branch, recover_snapshot, set_archived, wait_for_acknowledged_prefix,
};

pub(crate) fn outbox_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("cloud-history-outbox.sqlite3"))
        .map_err(|error| format!("resolve cloud history outbox: {error}"))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudTrajectoryConfig {
    pub endpoint: String,
    pub token: String,
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
    /// App-wide Clark JWT (see `AppState::cloud_token`), read per attempt so a
    /// token refreshed mid-session reaches every subsequent request.
    token: Arc<RwLock<Option<String>>>,
    app: AppHandle,
    http: reqwest::Client,
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
        token: Arc<RwLock<Option<String>>>,
        app: AppHandle,
        outbox_path: std::path::PathBuf,
    ) -> Result<Self, String> {
        let outbox = outbox::TrajectoryOutbox::new(outbox_path, &owner_scope, &conversation_id);
        Ok(Self {
            conversation_id,
            owner_scope: owner_scope.clone(),
            config,
            token,
            app,
            http: crate::commands::clark_http_client()?,
            outbox,
            flush_lock: Arc::new(Mutex::new(())),
            flush_scheduled: Arc::new(AtomicBool::new(false)),
            flush_retry_scheduled: Arc::new(AtomicBool::new(false)),
            flush_retry_attempt: Arc::new(AtomicUsize::new(0)),
        })
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
                let event_value = serde_json::to_value(event)
                    .map_err(|error| format!("serialize trajectory event: {error}"))?;
                let event_kind = event_kind(&event_value);
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
                    let _ = client
                        .app
                        .emit("cloud-conversation-deleted", &client.conversation_id);
                } else {
                    let _ = client.app.emit(
                        "cloud-sync-warning",
                        "Clark saved this run locally and will sync it when the cloud is reachable.",
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
        let url = format!(
            "{}/api/desktop/conversations/{}/trajectory",
            clark_rest_base(&self.config.endpoint)?,
            urlencoding::encode(&self.conversation_id)
        );

        // Transient failures use only the short prefix of this schedule; a 401
        // unlocks the longer tail so the frontend has time to refresh the JWT
        // (asked for via `cloud-auth-expired`) before the token is re-read.
        let mut last_error = String::new();
        let mut auth_retry = false;
        for (attempt, delay) in [0_u64, 250, 1_000, 2_000, 3_000, 4_000]
            .into_iter()
            .enumerate()
        {
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            let token = self.token.read().await.clone().unwrap_or_default();
            let token_owner = crate::commands::jwt_subject(&token).map_err(|_| {
                "cloud_account_changed: Clark signed out before this pending run could sync"
                    .to_string()
            })?;
            if token_owner != self.owner_scope {
                return Err(
                    "cloud_account_changed: a different Clark account is now signed in".into(),
                );
            }
            match self
                .http
                .post(&url)
                .bearer_auth(&token)
                .json(request)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => {
                    let status = response.status();
                    let detail = response.text().await.unwrap_or_default();
                    last_error = format!(
                        "trajectory endpoint returned {status}: {}",
                        detail.chars().take(300).collect::<String>()
                    );
                    if status == StatusCode::UNAUTHORIZED {
                        if !auth_retry {
                            auth_retry = true;
                            let _ = self.app.emit("cloud-auth-expired", ());
                        }
                    } else if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
                        return Err(
                            "cloud_deleted: this conversation was deleted on another device".into(),
                        );
                    } else if status.is_client_error()
                        && status != StatusCode::TOO_MANY_REQUESTS
                        && status != StatusCode::REQUEST_TIMEOUT
                    {
                        break;
                    }
                }
                Err(error) => last_error = format!("trajectory request failed: {error}"),
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

#[cfg(test)]
mod tests {
    use super::event_kind;
    use serde_json::json;

    #[test]
    fn trace_kind_preserves_provider_event_type() {
        assert_eq!(
            event_kind(&json!({
                "event": "trace",
                "source": "clark_agent",
                "payload": {"type": "context_transform_applied"}
            })),
            "trace.clark_agent.context_transform_applied"
        );
    }
}
