use std::path::Path;

use agent_core::{apply, AgentEvent, GoalStatus, RunFailureKind, RunOutcome, RunStatus, Snapshot};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::{open, owner_key, sql_error, AppendRequest, RecoveredSnapshot, TrajectoryOutbox};
use crate::trajectory::{
    event_kind, normalize_snapshot_value, reserve_timestamps, Conversation, EventRecord,
};

pub(super) fn recover_sync(
    path: &Path,
    owner_scope: &str,
    conversation_id: &str,
    cloud: Option<(Snapshot, i64)>,
) -> Result<Option<RecoveredSnapshot>, String> {
    let mut conn = open(path)?;
    let owner = owner_key(owner_scope);
    let cached = conn
        .query_row(
            r#"SELECT metadata_json, base_snapshot_json, base_rev, checkpoint_seq, local_live,
                      created_at_ms, updated_at_ms
               FROM journal_conversation WHERE owner_key = ?1 AND conversation_id = ?2"#,
            params![owner, conversation_id],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;

    let (
        metadata,
        mut snapshot,
        mut checkpoint_seq,
        allow_recovery,
        owned_live,
        base_rev,
        created_at_ms,
        updated_at_ms,
    ) = match (cloud, cached) {
        (Some((snapshot, _)), None) => {
            return Ok(Some(RecoveredSnapshot {
                snapshot,
                pending: false,
                metadata: None,
                needs_snapshot_publication: false,
            }));
        }
        (
            Some((snapshot, cloud_rev)),
            Some((metadata, _, cached_rev, checkpoint, local_live, created_at_ms, updated_at_ms)),
        ) if cloud_rev == cached_rev => (
            metadata,
            snapshot,
            checkpoint,
            true,
            local_live,
            cached_rev,
            created_at_ms,
            updated_at_ms,
        ),
        (Some((snapshot, cloud_rev)), Some((metadata, _, _, _, _, created_at_ms, _))) => {
            // Another device advanced the authority. Preserve this device's
            // batches for idempotent trajectory delivery, but never overlay the
            // divergent branch onto the newer cloud snapshot.
            let tx = conn.transaction().map_err(sql_error)?;
            let updated_at_ms = super::now_ms();
            tx.execute(
                r#"UPDATE trajectory_outbox SET replayable = 0
                   WHERE owner_key = ?1 AND conversation_id = ?2"#,
                params![owner, conversation_id],
            )
            .map_err(sql_error)?;
            tx.execute(
                r#"UPDATE journal_conversation
                   SET base_snapshot_json = ?3, base_rev = ?4, checkpoint_seq = 0,
                       local_live = 0,
                       updated_at_ms = ?5
                   WHERE owner_key = ?1 AND conversation_id = ?2"#,
                params![
                    owner,
                    conversation_id,
                    serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?,
                    cloud_rev,
                    updated_at_ms
                ],
            )
            .map_err(sql_error)?;
            tx.commit().map_err(sql_error)?;
            (
                metadata,
                snapshot,
                0,
                false,
                false,
                cloud_rev,
                created_at_ms,
                updated_at_ms,
            )
        }
        (
            None,
            Some((metadata, bytes, base_rev, checkpoint, local_live, created_at_ms, updated_at_ms)),
        ) => (
            metadata,
            serde_json::from_value(normalize_snapshot_value(
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
            ))
            .map_err(|e| e.to_string())?,
            checkpoint,
            true,
            local_live,
            base_rev,
            created_at_ms,
            updated_at_ms,
        ),
        (None, None) => return Ok(None),
    };

    let mut query = conn
        .prepare(
            r#"SELECT local_seq, request_json FROM trajectory_outbox
               WHERE owner_key = ?1 AND conversation_id = ?2
                 AND replayable = 1 AND local_seq > ?3
               ORDER BY local_seq ASC"#,
        )
        .map_err(sql_error)?;
    let rows = query
        .query_map(params![owner, conversation_id, checkpoint_seq], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(sql_error)?;
    let mut replayed = false;
    for row in rows {
        let (local_seq, bytes) = row.map_err(sql_error)?;
        let request: AppendRequest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        for record in request.events {
            let event: AgentEvent = serde_json::from_value(record.payload["event"].clone())
                .map_err(|e| format!("decode journal event: {e}"))?;
            apply(&mut snapshot, &event);
        }
        checkpoint_seq = checkpoint_seq.max(local_seq);
        replayed = true;
    }
    drop(query);

    let mut needs_snapshot_publication = replayed;
    if allow_recovery && (owned_live || replayed) {
        let events = interrupt_live_runs(
            &mut snapshot,
            "desktop_restart",
            "Agent Desktop restarted before this run finished. You can continue from the saved history.",
            "Agent Desktop restarted before the goal finished. Continue from the saved history.",
        );
        if !events.is_empty() {
            needs_snapshot_publication = true;
            let conversation: Conversation =
                serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
            let first_timestamp = reserve_timestamps(events.len());
            let records = events
                .into_iter()
                .enumerate()
                .map(|(offset, event)| {
                    let event_value = serde_json::to_value(event).map_err(|e| e.to_string())?;
                    Ok(EventRecord {
                        event_id: uuid::Uuid::new_v4(),
                        run_id: event_value
                            .get("run")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        event_kind: event_kind(&event_value),
                        recorded_at_unix_ms: first_timestamp + offset as i64,
                        payload: json!({"schemaVersion": 1, "sessionId": conversation_id,
                            "appVersion": env!("CARGO_PKG_VERSION"), "event": event_value}),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let outbox = TrajectoryOutbox::new(path.to_path_buf(), owner_scope, conversation_id);
            checkpoint_seq = outbox
                .enqueue_sync(AppendRequest {
                    conversation,
                    events: records,
                })?
                .local_seq;
            conn.execute(
                r#"UPDATE journal_conversation SET local_live = 0
                   WHERE owner_key = ?1 AND conversation_id = ?2"#,
                params![owner, conversation_id],
            )
            .map_err(sql_error)?;
        }
    }
    snapshot.history_checkpoint = Some(checkpoint_seq);
    let pending = conn
        .query_row(
            r#"SELECT EXISTS(SELECT 1 FROM trajectory_outbox
               WHERE owner_key = ?1 AND conversation_id = ?2 AND acknowledged = 0)"#,
            params![owner, conversation_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(sql_error)?;
    Ok(Some(RecoveredSnapshot {
        snapshot,
        pending,
        metadata: Some(recovered_metadata(
            &metadata,
            conversation_id,
            base_rev,
            created_at_ms,
            updated_at_ms,
        )?),
        needs_snapshot_publication,
    }))
}

fn recovered_metadata(
    bytes: &[u8],
    conversation_id: &str,
    base_rev: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
) -> Result<Value, String> {
    let mut metadata: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let object = metadata
        .as_object_mut()
        .ok_or("decode cached conversation metadata: expected object")?;
    object.insert("id".into(), Value::String(conversation_id.into()));
    object.insert("rev".into(), Value::from(base_rev));
    object.insert("createdAt".into(), Value::from(created_at_ms));
    object.insert("updatedAt".into(), Value::from(updated_at_ms));
    Ok(metadata)
}

/// Turn any active run (and its active goal) into durable terminal events.
///
/// Restart recovery and an explicit host-side close use the same transition so
/// neither can leave another device rendering a session as still working.
pub(crate) fn interrupt_live_runs(
    snapshot: &mut Snapshot,
    stop_reason: &str,
    error: &str,
    goal_blocker_reason: &str,
) -> Vec<AgentEvent> {
    let interrupted_runs = snapshot
        .runs
        .iter()
        .filter(|(_, run)| {
            matches!(
                run.status,
                RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingInput
            )
        })
        .map(|(run, _)| run.clone())
        .collect::<Vec<_>>();
    let mut events = interrupted_runs
        .iter()
        .cloned()
        .map(|run| AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status: RunStatus::Failed,
                stop_reason: Some(stop_reason.into()),
                error: Some(error.into()),
                failure_kind: Some(RunFailureKind::RuntimeInterrupted),
                usage: None,
                execution: None,
            },
        })
        .collect::<Vec<_>>();
    let goal_run = snapshot
        .goal
        .as_ref()
        .filter(|goal| goal.status == GoalStatus::Active)
        .and_then(|goal| {
            goal.run
                .as_ref()
                .filter(|run| interrupted_runs.contains(run))
                .cloned()
                .or_else(|| interrupted_runs.last().cloned())
        });
    if let Some(run) = goal_run {
        let mut goal = snapshot.goal.clone().expect("active goal is present");
        goal.status = GoalStatus::Blocked;
        goal.blocker_reason = Some(goal_blocker_reason.into());
        goal.updated_at_ms = reserve_timestamps(1) as u64;
        events.push(AgentEvent::GoalUpdated { run, goal });
    }
    for item in snapshot.timeline.clone() {
        let agent_core::TimelineItem::ProviderIncident { run, id } = item else {
            continue;
        };
        let Some(incident) = snapshot.provider_incidents.get(&id) else {
            continue;
        };
        if !matches!(
            incident.status,
            agent_core::ProviderIncidentStatus::Observed
                | agent_core::ProviderIncidentStatus::Retrying
        ) {
            continue;
        }
        let mut interrupted = incident.clone();
        interrupted.status = agent_core::ProviderIncidentStatus::Interrupted;
        // Do not fabricate a completion time from the later reopen boundary.
        interrupted.completed_at_ms = None;
        events.push(AgentEvent::ProviderIncidentUpdated {
            run,
            incident: interrupted,
        });
    }
    for event in &events {
        apply(snapshot, event);
    }
    events
}
