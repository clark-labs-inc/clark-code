use rusqlite::{params, Transaction};
use scout_scheduler::TaskSpec;

use super::model::{canonical_sha256, BindingImage, SchedulerImage, TaskRecordImage};

mod read;

pub(super) use read::load_scheduler;

#[derive(Clone, Copy)]
pub(super) struct SchedulerScope<'a> {
    pub(super) tenant_id: &'a str,
    pub(super) enterprise_id: &'a str,
    pub(super) manifest_id: &'a str,
}

pub(super) fn insert_scheduler(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    image: &SchedulerImage,
) -> Result<(), String> {
    let receipt = image.receipt()?;
    transaction
        .execute(
            "INSERT INTO scheduler_manifests (
                 tenant_id, enterprise_id, manifest_id, manifest_json,
                 generation, state_sha256
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                serde_json::to_vec(&image.manifest).map_err(|error| error.to_string())?,
                receipt.generation,
                receipt.state_sha256,
            ],
        )
        .map_err(|error| error.to_string())?;
    for record in image.tasks.values() {
        insert_task(transaction, scope, record)?;
        if let Some(lease) = record.lease() {
            insert_attempt(
                transaction,
                scope,
                record,
                lease.machine_id,
                lease.expires_at_ms,
            )?;
        }
    }
    for (key, runtime) in &image.quotas {
        let policy = image
            .manifest
            .quota_policies
            .get(key)
            .ok_or_else(|| "scheduler quota runtime has no policy".to_string())?;
        transaction
            .execute(
                "INSERT INTO scheduler_quotas (
                     tenant_id, enterprise_id, manifest_id, quota_key,
                     policy_json, next_start_at_ms, in_flight, revision
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
                params![
                    scope.tenant_id,
                    scope.enterprise_id,
                    scope.manifest_id,
                    key.as_str(),
                    serde_json::to_vec(policy).map_err(|error| error.to_string())?,
                    runtime.next_start_at_ms,
                    runtime.in_flight.len(),
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn persist_mutation(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    before: &SchedulerImage,
    after: &SchedulerImage,
    operation_kind: &str,
) -> Result<(), String> {
    if before.manifest != after.manifest {
        return Err("scheduler mutation changed its immutable manifest".into());
    }
    if before
        .tasks
        .keys()
        .any(|task_id| !after.tasks.contains_key(task_id))
    {
        return Err("scheduler mutation removed a durable task".into());
    }
    for (task_id, record) in &after.tasks {
        match before.tasks.get(task_id) {
            Some(previous) if previous == record => {}
            Some(previous) => {
                update_task(transaction, scope, record)?;
                persist_attempt_transition(transaction, scope, previous, record, operation_kind)?;
            }
            None => insert_task(transaction, scope, record)?,
        }
    }
    for (key, runtime) in &after.quotas {
        if before.quotas.get(key) == Some(runtime) {
            continue;
        }
        let updated = transaction
            .execute(
                "UPDATE scheduler_quotas
                 SET next_start_at_ms = ?5, in_flight = ?6,
                     revision = revision + 1
                 WHERE tenant_id = ?1 AND enterprise_id = ?2
                   AND manifest_id = ?3 AND quota_key = ?4",
                params![
                    scope.tenant_id,
                    scope.enterprise_id,
                    scope.manifest_id,
                    key.as_str(),
                    runtime.next_start_at_ms,
                    runtime.in_flight.len(),
                ],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("normalized scheduler quota disappeared during mutation".into());
        }
    }
    let receipt = after.receipt()?;
    if after.generation == before.generation {
        if before != after {
            return Err("scheduler state changed without advancing its generation".into());
        }
        return Ok(());
    }
    let updated = transaction
        .execute(
            "UPDATE scheduler_manifests
             SET generation = ?5, state_sha256 = ?6
             WHERE tenant_id = ?1 AND enterprise_id = ?2
               AND manifest_id = ?3 AND generation = ?4",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                before.generation,
                receipt.generation,
                receipt.state_sha256,
            ],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err("scheduler generation changed concurrently".into());
    }
    Ok(())
}

fn ensure_binding(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    spec: &TaskSpec,
) -> Result<String, String> {
    let binding = BindingImage::from_spec(spec);
    let binding_id = binding.id()?;
    let encoded = serde_json::to_vec(&binding).map_err(|error| error.to_string())?;
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO scheduler_bindings (
                 tenant_id, enterprise_id, manifest_id, binding_id, binding_json
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                &binding_id,
                &encoded,
            ],
        )
        .map_err(|error| error.to_string())?;
    if inserted == 1 {
        return Ok(binding_id);
    }
    let stored = transaction
        .query_row(
            "SELECT binding_json FROM scheduler_bindings
             WHERE tenant_id = ?1 AND enterprise_id = ?2
               AND manifest_id = ?3 AND binding_id = ?4",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                binding_id,
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .map_err(|error| error.to_string())?;
    if stored != encoded {
        return Err("scheduler binding id collides with different content".into());
    }
    Ok(binding_id)
}

fn insert_task(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    record: &TaskRecordImage,
) -> Result<(), String> {
    let binding_id = ensure_binding(transaction, scope, &record.spec)?;
    let columns = record.columns();
    transaction
        .execute(
            "INSERT INTO scheduler_tasks (
                 tenant_id, enterprise_id, manifest_id, task_id, binding_id,
                 spec_json, status_json, attempts, fence, priority,
                 ready_at_ms, state_kind, target_id, quota_key,
                 lease_machine_id, lease_expires_at_ms, revision
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                 ?11, ?12, ?13, ?14, ?15, ?16, 1
             )",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                record.spec.task_id.as_str(),
                binding_id,
                serde_json::to_vec(&record.spec).map_err(|error| error.to_string())?,
                serde_json::to_vec(&record.status).map_err(|error| error.to_string())?,
                record.attempts,
                record.fence,
                record.spec.priority,
                columns.ready_at_ms,
                columns.state_kind,
                record.spec.target_id.as_str(),
                record.spec.quota_key().as_str(),
                columns.lease_machine_id,
                columns.lease_expires_at_ms,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn update_task(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    record: &TaskRecordImage,
) -> Result<(), String> {
    let binding_id = ensure_binding(transaction, scope, &record.spec)?;
    let columns = record.columns();
    let updated = transaction
        .execute(
            "UPDATE scheduler_tasks
             SET binding_id = ?5, spec_json = ?6, status_json = ?7,
                 attempts = ?8, fence = ?9, priority = ?10,
                 ready_at_ms = ?11, state_kind = ?12, target_id = ?13,
                 quota_key = ?14, lease_machine_id = ?15,
                 lease_expires_at_ms = ?16, revision = revision + 1
             WHERE tenant_id = ?1 AND enterprise_id = ?2
               AND manifest_id = ?3 AND task_id = ?4",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                record.spec.task_id.as_str(),
                binding_id,
                serde_json::to_vec(&record.spec).map_err(|error| error.to_string())?,
                serde_json::to_vec(&record.status).map_err(|error| error.to_string())?,
                record.attempts,
                record.fence,
                record.spec.priority,
                columns.ready_at_ms,
                columns.state_kind,
                record.spec.target_id.as_str(),
                record.spec.quota_key().as_str(),
                columns.lease_machine_id,
                columns.lease_expires_at_ms,
            ],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err("normalized scheduler task disappeared during mutation".into());
    }
    Ok(())
}

pub(super) fn insert_attempt(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    record: &TaskRecordImage,
    machine_id: &str,
    expires_at_ms: u64,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT INTO scheduler_attempts (
                 tenant_id, enterprise_id, manifest_id, task_id, fence,
                 machine_id, attempt, lease_expires_at_ms, attempt_state,
                 result_sha256
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'leased', NULL)",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                record.spec.task_id.as_str(),
                record.fence,
                machine_id,
                record.attempts,
                expires_at_ms,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn persist_attempt_transition(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    before: &TaskRecordImage,
    after: &TaskRecordImage,
    operation_kind: &str,
) -> Result<(), String> {
    let previous = before.lease();
    let next = after.lease();
    if let Some(previous) = &previous {
        let same_lease = next.as_ref().is_some_and(|next| {
            next.machine_id == previous.machine_id && next.fence == previous.fence
        });
        if same_lease {
            if next.as_ref().expect("checked").expires_at_ms != previous.expires_at_ms {
                let updated = transaction
                    .execute(
                        "UPDATE scheduler_attempts
                         SET lease_expires_at_ms = ?7
                         WHERE tenant_id = ?1 AND enterprise_id = ?2
                           AND manifest_id = ?3 AND task_id = ?4 AND fence = ?5
                           AND machine_id = ?6 AND attempt_state = 'leased'",
                        params![
                            scope.tenant_id,
                            scope.enterprise_id,
                            scope.manifest_id,
                            before.spec.task_id.as_str(),
                            previous.fence,
                            previous.machine_id,
                            next.as_ref().expect("checked").expires_at_ms,
                        ],
                    )
                    .map_err(|error| error.to_string())?;
                if updated != 1 {
                    return Err("scheduler heartbeat lost its active attempt row".into());
                }
            }
            return Ok(());
        }
        let attempt_state = if operation_kind == "complete" {
            "completed"
        } else {
            "reaped"
        };
        let result_sha256 = canonical_sha256(&(
            "scout-scheduler-attempt-result-v1",
            attempt_state,
            &after.spec.task_id,
            previous.fence,
            &after.status,
        ))?;
        let updated = transaction
            .execute(
                "UPDATE scheduler_attempts
                 SET attempt_state = ?7, result_sha256 = ?8
                 WHERE tenant_id = ?1 AND enterprise_id = ?2
                   AND manifest_id = ?3 AND task_id = ?4 AND fence = ?5
                   AND machine_id = ?6 AND attempt_state = 'leased'",
                params![
                    scope.tenant_id,
                    scope.enterprise_id,
                    scope.manifest_id,
                    before.spec.task_id.as_str(),
                    previous.fence,
                    previous.machine_id,
                    attempt_state,
                    result_sha256,
                ],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("scheduler mutation lost its active attempt row".into());
        }
    }
    if let Some(next) = next {
        insert_attempt(
            transaction,
            scope,
            after,
            next.machine_id,
            next.expires_at_ms,
        )?;
    }
    Ok(())
}
