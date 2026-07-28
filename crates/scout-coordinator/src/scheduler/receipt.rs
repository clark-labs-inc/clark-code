use rusqlite::{params, Connection, OptionalExtension};
use scout_scheduler::{ScheduleManifest, SchedulerReceipt, TaskStatus};
use sha2::{Digest, Sha256};

use super::storage::SchedulerScope;

pub(super) fn verified_receipt(
    connection: &Connection,
    scope: SchedulerScope<'_>,
) -> Result<Option<SchedulerReceipt>, String> {
    let Some((receipt, stored_state_sha256)) = compute_receipt(connection, scope)? else {
        return Ok(None);
    };
    if receipt.state_sha256 != stored_state_sha256 {
        return Err("normalized scheduler receipt does not match persisted metadata".into());
    }
    Ok(Some(receipt))
}

pub(super) fn unverified_receipt(
    connection: &Connection,
    scope: SchedulerScope<'_>,
) -> Result<Option<SchedulerReceipt>, String> {
    compute_receipt(connection, scope).map(|value| value.map(|(receipt, _)| receipt))
}

fn compute_receipt(
    connection: &Connection,
    scope: SchedulerScope<'_>,
) -> Result<Option<(SchedulerReceipt, String)>, String> {
    let metadata = connection
        .query_row(
            "SELECT manifest_json, generation, state_sha256
             FROM scheduler_manifests
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND manifest_id = ?3",
            params![scope.tenant_id, scope.enterprise_id, scope.manifest_id],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((manifest_json, generation, stored_state_sha256)) = metadata else {
        return Ok(None);
    };
    let manifest: ScheduleManifest =
        serde_json::from_slice(&manifest_json).map_err(|error| error.to_string())?;
    if manifest.manifest_id != scope.manifest_id || manifest.enterprise_id != scope.enterprise_id {
        return Err("normalized scheduler manifest scope disagrees with its row key".into());
    }

    let mut hasher = Sha256::new();
    hasher.update(b"[\"scout-scheduler-state-v1\",");
    hasher.update(&manifest_json);
    hasher.update(b",");
    hasher.update(generation.to_string().as_bytes());
    hasher.update(b",{");

    let mut tasks = 0_usize;
    let mut pending = 0_usize;
    let mut leased = 0_usize;
    let mut retry_wait = 0_usize;
    let mut terminal = 0_usize;
    let mut complete_terminal = 0_usize;
    let mut first = true;
    let mut statement = connection
        .prepare(
            "SELECT task_id, spec_json, status_json, attempts, fence, state_kind
             FROM scheduler_tasks
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND manifest_id = ?3
             ORDER BY task_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![scope.tenant_id, scope.enterprise_id, scope.manifest_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, u16>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (task_id, spec_json, status_json, attempts, fence, state_kind) =
            row.map_err(|error| error.to_string())?;
        if !first {
            hasher.update(b",");
        }
        first = false;
        hasher.update(serde_json::to_vec(&task_id).map_err(|error| error.to_string())?);
        hasher.update(b":{\"spec\":");
        hasher.update(&spec_json);
        hasher.update(b",\"status\":");
        hasher.update(&status_json);
        hasher.update(b",\"attempts\":");
        hasher.update(attempts.to_string().as_bytes());
        hasher.update(b",\"fence\":");
        hasher.update(fence.to_string().as_bytes());
        hasher.update(b"}");
        tasks += 1;
        match state_kind.as_str() {
            "pending" => pending += 1,
            "leased" => leased += 1,
            "retry_wait" => retry_wait += 1,
            "terminal" => {
                terminal += 1;
                let status: TaskStatus =
                    serde_json::from_slice(&status_json).map_err(|error| error.to_string())?;
                let TaskStatus::Terminal { disposition, .. } = status else {
                    return Err(
                        "normalized scheduler terminal index disagrees with its status".into(),
                    );
                };
                if disposition.is_complete() {
                    complete_terminal += 1;
                }
            }
            _ => return Err("normalized scheduler has an invalid task state kind".into()),
        }
    }
    drop(statement);
    hasher.update(b"},");

    let mut in_flight = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut statement = connection
        .prepare(
            "SELECT quota_key, task_id
             FROM scheduler_tasks
             WHERE tenant_id = ?1 AND enterprise_id = ?2
               AND manifest_id = ?3 AND state_kind = 'leased'
             ORDER BY quota_key, task_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![scope.tenant_id, scope.enterprise_id, scope.manifest_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (quota_key, task_id) = row.map_err(|error| error.to_string())?;
        in_flight.entry(quota_key).or_default().push(task_id);
    }
    drop(statement);

    hasher.update(b"{");
    let mut first = true;
    let mut quota_rows = 0_usize;
    let mut statement = connection
        .prepare(
            "SELECT quota_key, next_start_at_ms, in_flight
             FROM scheduler_quotas
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND manifest_id = ?3
             ORDER BY quota_key",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![scope.tenant_id, scope.enterprise_id, scope.manifest_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, usize>(2)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (quota_key, next_start_at_ms, stored_in_flight) =
            row.map_err(|error| error.to_string())?;
        let members = in_flight.remove(&quota_key).unwrap_or_default();
        if stored_in_flight != members.len() {
            return Err("normalized scheduler quota in-flight count is inconsistent".into());
        }
        if !first {
            hasher.update(b",");
        }
        first = false;
        quota_rows += 1;
        hasher.update(serde_json::to_vec(&quota_key).map_err(|error| error.to_string())?);
        hasher.update(b":{\"in_flight\":[");
        for (index, task_id) in members.iter().enumerate() {
            if index > 0 {
                hasher.update(b",");
            }
            hasher.update(serde_json::to_vec(task_id).map_err(|error| error.to_string())?);
        }
        hasher.update(b"],\"next_start_at_ms\":");
        hasher.update(next_start_at_ms.to_string().as_bytes());
        hasher.update(b"}");
    }
    if quota_rows != manifest.quota_policies.len() || !in_flight.is_empty() {
        return Err("normalized scheduler quota coverage is incomplete".into());
    }
    hasher.update(b"}]");

    let gap_terminal = terminal - complete_terminal;
    Ok(Some((
        SchedulerReceipt {
            manifest_id: manifest.manifest_id,
            generation,
            tasks,
            pending,
            leased,
            retry_wait,
            terminal,
            complete_terminal,
            gap_terminal,
            sealed: terminal == tasks,
            complete: terminal == tasks && terminal == complete_terminal,
            state_sha256: format!("{:x}", hasher.finalize()),
        },
        stored_state_sha256,
    )))
}
