//! Transactional local outbox for Desktop trajectory delivery.
//!
//! Cloud remains authoritative. SQLite only holds the last acknowledged cloud
//! snapshot plus event batches that have not yet been covered by a later cloud
//! snapshot. Stable event IDs make replay to the cloud idempotent.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_core::{RunStatus, Snapshot};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::{AppendRequest, CloudTrajectoryConfig};

mod barrier;
mod recovery;
mod schema;
mod storage;
pub(crate) use barrier::wait_for_acknowledged_prefix;
pub(crate) use recovery::interrupt_live_runs;
pub(crate) use storage::migrate_legacy_database;
use storage::{open, owner_key, reclaim_free_pages, sql_error};

#[derive(Clone)]
pub struct TrajectoryOutbox {
    path: PathBuf,
    owner_key: String,
    conversation_id: String,
}

#[derive(Clone)]
pub struct PendingBatch {
    pub local_seq: i64,
    pub batch_id: String,
    pub request: AppendRequest,
}

#[derive(Debug)]
pub struct RecoveredSnapshot {
    pub snapshot: Snapshot,
    pub pending: bool,
    /// Cached conversation metadata used only when the cloud GET is
    /// temporarily unavailable. It lets the WebView publish a recovered
    /// terminal snapshot once connectivity returns without inventing a new
    /// conversation identity.
    pub metadata: Option<Value>,
    /// Recovery replayed local events or synthesized an interruption that the
    /// cloud snapshot has not yet absorbed. Keep this separate from `pending`:
    /// event delivery can be acknowledged before the corresponding full
    /// snapshot is published.
    pub needs_snapshot_publication: bool,
}

impl TrajectoryOutbox {
    pub fn new(path: PathBuf, owner_scope: &str, conversation_id: &str) -> Self {
        Self {
            path,
            owner_key: owner_key(owner_scope),
            conversation_id: conversation_id.to_string(),
        }
    }

    pub async fn initialize(
        &self,
        config: &CloudTrajectoryConfig,
        base_snapshot: &Snapshot,
        base_rev: i64,
    ) -> Result<(), String> {
        let this = self.clone();
        let config = config.clone();
        let snapshot = base_snapshot.clone();
        blocking(move || this.initialize_sync(&config, &snapshot, base_rev)).await
    }

    pub async fn enqueue(&self, request: &AppendRequest) -> Result<PendingBatch, String> {
        let this = self.clone();
        let request = request.clone();
        blocking(move || this.enqueue_sync(request)).await
    }

    pub async fn pending(&self) -> Result<Vec<PendingBatch>, String> {
        let this = self.clone();
        blocking(move || this.pending_sync()).await
    }

    pub async fn acknowledge(&self, batch_id: &str) -> Result<(), String> {
        let this = self.clone();
        let batch_id = batch_id.to_string();
        blocking(move || this.acknowledge_sync(&batch_id)).await
    }

    fn initialize_sync(
        &self,
        config: &CloudTrajectoryConfig,
        snapshot: &Snapshot,
        base_rev: i64,
    ) -> Result<(), String> {
        let conn = open(&self.path)?;
        let now = now_ms();
        let metadata = json!({
            "id": self.conversation_id,
            "title": config.title,
            "provider": config.provider,
            "project": config.project,
            "repositoryFingerprint": config.repository_fingerprint,
            "remoteHost": config.remote_host,
            "mode": config.mode,
            "specialistContext": config.metadata.get("specialistContext").cloned(),
            "rev": base_rev,
            "archived": false,
            "titleLocked": false,
        });
        let checkpoint_seq = snapshot.history_checkpoint.unwrap_or_default();
        let local_live = snapshot.runs.values().any(|run| {
            matches!(
                run.status,
                RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingInput
            )
        });
        let metadata = serde_json::to_vec(&metadata).map_err(|e| e.to_string())?;
        let snapshot = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
        conn.execute(
            r#"INSERT INTO journal_conversation
               (owner_key, conversation_id, metadata_json, base_snapshot_json, base_rev, checkpoint_seq, local_live, created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
               ON CONFLICT(owner_key, conversation_id) DO UPDATE SET
                 metadata_json = excluded.metadata_json,
                 base_snapshot_json = CASE WHEN NOT EXISTS (
                   SELECT 1 FROM trajectory_outbox
                   WHERE owner_key = excluded.owner_key AND conversation_id = excluded.conversation_id
                 ) THEN excluded.base_snapshot_json ELSE journal_conversation.base_snapshot_json END,
                 base_rev = CASE WHEN NOT EXISTS (
                   SELECT 1 FROM trajectory_outbox
                   WHERE owner_key = excluded.owner_key AND conversation_id = excluded.conversation_id
                 ) THEN excluded.base_rev ELSE journal_conversation.base_rev END,
                 checkpoint_seq = CASE WHEN NOT EXISTS (
                   SELECT 1 FROM trajectory_outbox
                   WHERE owner_key = excluded.owner_key AND conversation_id = excluded.conversation_id
                 ) THEN excluded.checkpoint_seq ELSE journal_conversation.checkpoint_seq END,
                 local_live = CASE WHEN NOT EXISTS (
                   SELECT 1 FROM trajectory_outbox
                   WHERE owner_key = excluded.owner_key AND conversation_id = excluded.conversation_id
                 ) THEN excluded.local_live ELSE journal_conversation.local_live END"#,
            params![self.owner_key, self.conversation_id, metadata, snapshot, base_rev, checkpoint_seq, local_live, now],
        )
        .map_err(sql_error)?;
        Ok(())
    }

    fn enqueue_sync(&self, request: AppendRequest) -> Result<PendingBatch, String> {
        let mut conn = open(&self.path)?;
        let tx = conn.transaction().map_err(sql_error)?;
        let batch_id = uuid::Uuid::new_v4().to_string();
        let max_recorded = request
            .events
            .iter()
            .map(|event| event.recorded_at_unix_ms)
            .max()
            .unwrap_or_else(now_ms);
        let request_json = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        tx.execute(
            r#"INSERT INTO trajectory_outbox
               (batch_id, owner_key, conversation_id, max_recorded_at_ms, request_json, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![batch_id, self.owner_key, self.conversation_id, max_recorded, request_json, now_ms()],
        )
        .map_err(sql_error)?;
        let local_seq = tx.last_insert_rowid();
        tx.execute(
            "UPDATE journal_conversation SET updated_at_ms = ?3 WHERE owner_key = ?1 AND conversation_id = ?2",
            params![self.owner_key, self.conversation_id, now_ms()],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(PendingBatch {
            local_seq,
            batch_id,
            request,
        })
    }

    fn pending_sync(&self) -> Result<Vec<PendingBatch>, String> {
        let conn = open(&self.path)?;
        let mut query = conn
            .prepare(
                r#"SELECT local_seq, batch_id, request_json FROM trajectory_outbox
                   WHERE owner_key = ?1 AND conversation_id = ?2 AND acknowledged = 0
                   ORDER BY local_seq ASC LIMIT 250"#,
            )
            .map_err(sql_error)?;
        let rows = query
            .query_map(params![self.owner_key, self.conversation_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(sql_error)?;
        let mut batches = Vec::new();
        for row in rows {
            let (local_seq, batch_id, bytes) = row.map_err(sql_error)?;
            let request = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            batches.push(PendingBatch {
                local_seq,
                batch_id,
                request,
            });
        }
        Ok(batches)
    }

    fn acknowledge_sync(&self, batch_id: &str) -> Result<(), String> {
        let mut conn = open(&self.path)?;
        let tx = conn.transaction().map_err(sql_error)?;
        tx.execute(
            r#"UPDATE trajectory_outbox SET acknowledged = 1
               WHERE batch_id = ?1 AND owner_key = ?2 AND conversation_id = ?3"#,
            params![batch_id, self.owner_key, self.conversation_id],
        )
        .map_err(sql_error)?;
        tx.execute(
            r#"DELETE FROM trajectory_outbox
               WHERE batch_id = ?1 AND acknowledged = 1 AND local_seq <= (
                   SELECT checkpoint_seq FROM journal_conversation
                   WHERE owner_key = ?2 AND conversation_id = ?3
               )"#,
            params![batch_id, self.owner_key, self.conversation_id],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(())
    }
}

pub async fn checkpoint_snapshot(
    path: PathBuf,
    owner_scope: String,
    conversation_id: String,
    metadata: Value,
    snapshot: Snapshot,
    rev: i64,
    checkpoint_seq: i64,
    local_live: bool,
) -> Result<(), String> {
    blocking(move || {
        let mut conn = open(&path)?;
        let tx = conn.transaction().map_err(sql_error)?;
        let now = now_ms();
        tx.execute(
            r#"INSERT INTO journal_conversation
               (owner_key, conversation_id, metadata_json, base_snapshot_json, base_rev, checkpoint_seq, local_live, created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
               ON CONFLICT(owner_key, conversation_id) DO UPDATE SET
                 metadata_json = excluded.metadata_json,
                 base_snapshot_json = excluded.base_snapshot_json,
                 base_rev = excluded.base_rev,
                 checkpoint_seq = excluded.checkpoint_seq,
                 local_live = excluded.local_live,
                 updated_at_ms = excluded.updated_at_ms"#,
            params![owner_key(&owner_scope), conversation_id, serde_json::to_vec(&metadata).map_err(|e| e.to_string())?, serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?, rev, checkpoint_seq, local_live, now],
        )
        .map_err(sql_error)?;
        tx.execute(
            r#"DELETE FROM trajectory_outbox
               WHERE owner_key = ?1 AND conversation_id = ?2
                 AND acknowledged = 1 AND local_seq <= ?3"#,
            params![owner_key(&owner_scope), conversation_id, checkpoint_seq],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        reclaim_free_pages(&conn)?;
        Ok(())
    })
    .await
}

pub async fn recover_snapshot(
    path: PathBuf,
    owner_scope: String,
    conversation_id: String,
    cloud: Option<(Snapshot, i64)>,
) -> Result<Option<RecoveredSnapshot>, String> {
    blocking(move || recovery::recover_sync(&path, &owner_scope, &conversation_id, cloud)).await
}

pub async fn merge_local_summaries(
    path: PathBuf,
    owner_scope: String,
    cloud: Vec<Value>,
    cloud_available: bool,
) -> Result<Vec<Value>, String> {
    blocking(move || merge_summaries_sync(&path, &owner_scope, cloud, cloud_available)).await
}

pub async fn delete_conversation(
    path: PathBuf,
    owner_scope: String,
    conversation_id: String,
) -> Result<(), String> {
    blocking(move || {
        let conn = open(&path)?;
        conn.execute(
            "DELETE FROM journal_conversation WHERE owner_key = ?1 AND conversation_id = ?2",
            params![owner_key(&owner_scope), conversation_id],
        )
        .map_err(sql_error)?;
        Ok(())
    })
    .await
}

pub async fn set_archived(
    path: PathBuf,
    owner_scope: String,
    conversation_id: String,
    archived: bool,
) -> Result<(), String> {
    blocking(move || {
        let conn = open(&path)?;
        let row: Option<Vec<u8>> = conn
            .query_row(
                "SELECT metadata_json FROM journal_conversation WHERE owner_key = ?1 AND conversation_id = ?2",
                params![owner_key(&owner_scope), conversation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let Some(bytes) = row else { return Ok(()) };
        let mut metadata: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        metadata["archived"] = archived.into();
        conn.execute(
            "UPDATE journal_conversation SET metadata_json = ?3, updated_at_ms = ?4 WHERE owner_key = ?1 AND conversation_id = ?2",
            params![owner_key(&owner_scope), conversation_id, serde_json::to_vec(&metadata).map_err(|e| e.to_string())?, now_ms()],
        )
        .map_err(sql_error)?;
        Ok(())
    })
    .await
}

/// Preserve a divergent branch for eventual trajectory upload/diagnostics, but
/// never overlay it onto the authoritative snapshot after a CAS conflict.
pub async fn quarantine_snapshot_branch(
    path: PathBuf,
    owner_scope: String,
    conversation_id: String,
) -> Result<(), String> {
    blocking(move || {
        let conn = open(&path)?;
        conn.execute(
            r#"UPDATE trajectory_outbox SET replayable = 0
               WHERE owner_key = ?1 AND conversation_id = ?2"#,
            params![owner_key(&owner_scope), conversation_id],
        )
        .map_err(sql_error)?;
        Ok(())
    })
    .await
}

fn merge_summaries_sync(
    path: &Path,
    owner_scope: &str,
    mut cloud: Vec<Value>,
    cloud_available: bool,
) -> Result<Vec<Value>, String> {
    let conn = open(path)?;
    let cloud_ids = cloud
        .iter()
        .filter_map(|value| value.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<std::collections::HashSet<_>>();
    let mut local_only = Vec::new();
    let mut local_specialists = std::collections::HashMap::new();
    let mut query = conn
        .prepare(
            r#"SELECT conversation_id, metadata_json, base_snapshot_json,
                      created_at_ms, updated_at_ms,
                      base_rev, EXISTS(SELECT 1 FROM trajectory_outbox o
                        WHERE o.owner_key = journal_conversation.owner_key
                          AND o.conversation_id = journal_conversation.conversation_id)
               FROM journal_conversation WHERE owner_key = ?1 ORDER BY updated_at_ms DESC"#,
        )
        .map_err(sql_error)?;
    let rows = query
        .query_map(params![owner_key(owner_scope)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })
        .map_err(sql_error)?;
    for row in rows {
        let (id, bytes, snapshot_bytes, created, updated, base_rev, has_outbox) =
            row.map_err(sql_error)?;
        let mut value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        value["createdAt"] = created.into();
        value["updatedAt"] = updated.into();
        let context = value
            .get("specialistContext")
            .filter(|context| !context.is_null())
            .cloned()
            .or_else(|| {
                serde_json::from_slice(&snapshot_bytes)
                    .ok()
                    .and_then(|snapshot| specialist_context_from_snapshot(&snapshot))
            });
        if let Some(context) = context {
            value["specialistContext"] = context.clone();
            local_specialists.insert(id.clone(), context);
        }
        if !cloud_ids.contains(&id) && (!cloud_available || base_rev == 0 || has_outbox) {
            local_only.push(value);
        }
    }
    for value in &mut cloud {
        let missing_specialist = value.get("specialistContext").is_none_or(Value::is_null);
        if !missing_specialist {
            continue;
        }
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(context) = local_specialists.get(id) {
            value["specialistContext"] = context.clone();
        }
    }
    let mut values = cloud;
    values.extend(local_only);
    Ok(values)
}

fn specialist_context_from_snapshot(snapshot: &Value) -> Option<Value> {
    let timeline = snapshot.get("timeline")?.as_array()?;
    for item in timeline.iter().rev() {
        let Some(blocks) = item.get("blocks").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks.iter().rev() {
            if block.get("type").and_then(Value::as_str) != Some("skill_reference") {
                continue;
            }
            let Some(workflow) = block.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some((kind, _)) = workflow.split_once(':') else {
                continue;
            };
            if matches!(kind, "spec" | "scout" | "security" | "scientist" | "rsi") {
                return Some(json!({"kind": kind, "workflow": workflow}));
            }
        }
    }
    None
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("trajectory outbox task: {e}"))?
}

#[cfg(test)]
mod tests;
