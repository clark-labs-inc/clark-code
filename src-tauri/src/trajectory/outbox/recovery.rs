use std::path::Path;

use agent_core::{apply, AgentEvent, RunFailureKind, RunOutcome, RunStatus, Snapshot};
use rusqlite::{params, OptionalExtension};
use serde_json::json;

use super::{open, owner_key, sql_error, AppendRequest, RecoveredSnapshot, TrajectoryOutbox};
use crate::trajectory::{event_kind, reserve_timestamps, Conversation, EventRecord};

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
            r#"SELECT metadata_json, base_snapshot_json, base_rev, checkpoint_seq, local_live
               FROM journal_conversation WHERE owner_key = ?1 AND conversation_id = ?2"#,
            params![owner, conversation_id],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;

    let (metadata, mut snapshot, mut checkpoint_seq, allow_recovery, owned_live) =
        match (cloud, cached) {
            (Some((snapshot, _)), None) => {
                return Ok(Some(RecoveredSnapshot {
                    snapshot,
                    pending: false,
                }));
            }
            (
                Some((snapshot, cloud_rev)),
                Some((metadata, _, cached_rev, checkpoint, local_live)),
            ) if cloud_rev == cached_rev => (metadata, snapshot, checkpoint, true, local_live),
            (Some((snapshot, cloud_rev)), Some((metadata, _, _, _, _))) => {
                // Another device advanced the authority. Preserve this device's
                // batches for idempotent trajectory delivery, but never overlay the
                // divergent branch onto the newer cloud snapshot.
                let tx = conn.transaction().map_err(sql_error)?;
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
                        super::now_ms()
                    ],
                )
                .map_err(sql_error)?;
                tx.commit().map_err(sql_error)?;
                (metadata, snapshot, 0, false, false)
            }
            (None, Some((metadata, bytes, _, checkpoint, local_live))) => (
                metadata,
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
                checkpoint,
                true,
                local_live,
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

    if allow_recovery && (owned_live || replayed) {
        let events = interrupt_live_runs(&mut snapshot);
        if !events.is_empty() {
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
    Ok(Some(RecoveredSnapshot { snapshot, pending }))
}

fn interrupt_live_runs(snapshot: &mut Snapshot) -> Vec<AgentEvent> {
    let mut events = snapshot
        .runs
        .iter()
        .filter(|(_, run)| {
            matches!(
                run.status,
                RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingInput
            )
        })
        .map(|(run, _)| AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status: RunStatus::Failed,
                stop_reason: Some("desktop_restart".into()),
                error: Some("Clark restarted before this run finished. You can continue from the saved history.".into()),
                failure_kind: Some(RunFailureKind::RuntimeInterrupted),
                usage: None,
                execution: None,
            },
        })
        .collect::<Vec<_>>();
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
