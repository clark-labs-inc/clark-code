use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, params_from_iter, types::Value, Transaction};
use scout_adapter_protocol::TargetId;
use scout_scheduler::{
    LeaseClaim, QuotaPolicy, ScheduleManifest, SchedulerReceipt, TaskSpec, TaskStatus,
    TerminalDisposition,
};

use super::model::{canonical_sha256, TaskRecordImage};
use super::receipt;
use super::storage::{persist_attempt_transition, update_task, SchedulerScope};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuotaRuntime {
    policy: QuotaPolicy,
    next_start_at_ms: u64,
    in_flight: usize,
}

#[derive(Debug)]
struct Candidate {
    before: TaskRecordImage,
    after: TaskRecordImage,
    claim: LeaseClaim,
}

pub(super) fn claim(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    machine_id: &str,
    eligible_targets: &BTreeSet<TargetId>,
    now_ms: u64,
    max_tasks: usize,
) -> Result<(Vec<LeaseClaim>, SchedulerReceipt), String> {
    validate_claim(machine_id, eligible_targets, max_tasks)?;
    let before_receipt = receipt::verified_receipt(transaction, scope)?
        .ok_or_else(|| "scheduler manifest is not initialized".to_string())?;
    let (manifest, generation) = read_manifest(transaction, scope)?;
    let mut quotas = read_quotas(transaction, scope, &manifest)?;
    let original_quotas = quotas.clone();
    let expired = reap_expired(transaction, scope, &mut quotas, now_ms)?;
    let planned = plan_claims(
        transaction,
        scope,
        machine_id,
        eligible_targets,
        now_ms,
        max_tasks,
        &mut quotas,
    )?;
    for candidate in &planned {
        update_task(transaction, scope, &candidate.after)?;
        persist_attempt_transition(
            transaction,
            scope,
            &candidate.before,
            &candidate.after,
            "claim",
        )?;
    }
    persist_quotas(transaction, scope, &original_quotas, &quotas)?;

    if expired == 0 && planned.is_empty() {
        return Ok((Vec::new(), before_receipt));
    }
    let next_generation = generation
        .checked_add(1)
        .ok_or_else(|| "scheduler generation overflow".to_string())?;
    let updated = transaction
        .execute(
            "UPDATE scheduler_manifests SET generation = ?5
             WHERE tenant_id = ?1 AND enterprise_id = ?2
               AND manifest_id = ?3 AND generation = ?4",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                generation,
                next_generation,
            ],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err("scheduler generation changed concurrently".into());
    }
    let after_receipt = receipt::unverified_receipt(transaction, scope)?
        .ok_or_else(|| "scheduler manifest disappeared during claim".to_string())?;
    transaction
        .execute(
            "UPDATE scheduler_manifests SET state_sha256 = ?4
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND manifest_id = ?3",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                &after_receipt.state_sha256,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok((
        planned
            .into_iter()
            .map(|candidate| candidate.claim)
            .collect(),
        after_receipt,
    ))
}

fn validate_claim(
    machine_id: &str,
    eligible_targets: &BTreeSet<TargetId>,
    max_tasks: usize,
) -> Result<(), String> {
    if machine_id.is_empty()
        || machine_id.len() > 256
        || machine_id.trim() != machine_id
        || !machine_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err("scheduler machine id must contain 1 to 256 identifier characters".into());
    }
    if eligible_targets.is_empty() {
        return Err("scheduler claim requires at least one eligible target".into());
    }
    if max_tasks == 0 || max_tasks > 1_024 {
        return Err("scheduler claim size must be in 1..=1024".into());
    }
    Ok(())
}

fn read_manifest(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
) -> Result<(ScheduleManifest, u64), String> {
    let (manifest_json, generation) = transaction
        .query_row(
            "SELECT manifest_json, generation FROM scheduler_manifests
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND manifest_id = ?3",
            params![scope.tenant_id, scope.enterprise_id, scope.manifest_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, u64>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let manifest: ScheduleManifest =
        serde_json::from_slice(&manifest_json).map_err(|error| error.to_string())?;
    if manifest.manifest_id != scope.manifest_id || manifest.enterprise_id != scope.enterprise_id {
        return Err("normalized scheduler manifest scope disagrees with its row key".into());
    }
    Ok((manifest, generation))
}

fn read_quotas(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    manifest: &ScheduleManifest,
) -> Result<BTreeMap<String, QuotaRuntime>, String> {
    let mut quotas = BTreeMap::new();
    let mut statement = transaction
        .prepare(
            "SELECT quota_key, policy_json, next_start_at_ms, in_flight
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
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, usize>(3)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (key, policy_json, next_start_at_ms, in_flight) =
            row.map_err(|error| error.to_string())?;
        let policy: QuotaPolicy =
            serde_json::from_slice(&policy_json).map_err(|error| error.to_string())?;
        let expected = manifest
            .quota_policies
            .iter()
            .find(|(candidate, _)| candidate.as_str() == key)
            .map(|(_, policy)| *policy)
            .ok_or_else(|| "normalized scheduler has an unknown quota".to_string())?;
        if policy != expected || in_flight > usize::from(policy.max_in_flight) {
            return Err("normalized scheduler quota policy is inconsistent".into());
        }
        quotas.insert(
            key,
            QuotaRuntime {
                policy,
                next_start_at_ms,
                in_flight,
            },
        );
    }
    if quotas.len() != manifest.quota_policies.len() {
        return Err("normalized scheduler quota coverage is incomplete".into());
    }
    Ok(quotas)
}

fn reap_expired(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    quotas: &mut BTreeMap<String, QuotaRuntime>,
    now_ms: u64,
) -> Result<usize, String> {
    let mut statement = transaction
        .prepare(
            "SELECT spec_json, status_json, attempts, fence, quota_key
             FROM scheduler_tasks
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND manifest_id = ?3
               AND state_kind = 'leased' AND lease_expires_at_ms < ?4
             ORDER BY task_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                now_ms
            ],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, u16>(2)?,
                    row.get::<_, u64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    for (spec_json, status_json, attempts, fence, quota_key) in &rows {
        let spec: TaskSpec =
            serde_json::from_slice(spec_json).map_err(|error| error.to_string())?;
        spec.validate()?;
        let status: TaskStatus =
            serde_json::from_slice(status_json).map_err(|error| error.to_string())?;
        let TaskStatus::Leased {
            machine_id: _,
            fence: active_fence,
            expires_at_ms,
        } = &status
        else {
            return Err("normalized scheduler leased index disagrees with its status".into());
        };
        if active_fence != fence
            || *expires_at_ms >= now_ms
            || spec.quota_key().as_str() != quota_key
        {
            return Err("normalized scheduler expired lease columns are inconsistent".into());
        }
        let runtime = quotas
            .get_mut(quota_key)
            .ok_or_else(|| "scheduler task has no quota policy".to_string())?;
        runtime.in_flight = runtime
            .in_flight
            .checked_sub(1)
            .ok_or_else(|| "scheduler quota in-flight count underflow".to_string())?;
        let error_sha256 =
            canonical_sha256(&("scout-scheduler-lease-expired-v1", &spec.task_id, attempts))?;
        let next_status = if *attempts >= runtime.policy.max_attempts {
            TaskStatus::Terminal {
                disposition: TerminalDisposition::RetryExhausted,
                receipt_id: None,
                evidence_sha256: error_sha256,
            }
        } else {
            TaskStatus::RetryWait {
                not_before_ms: now_ms.saturating_add(retry_delay(runtime.policy, *attempts)),
                class: scout_scheduler::RetryClass::TransientTransport,
                error_sha256,
            }
        };
        let before = TaskRecordImage {
            spec: spec.clone(),
            status,
            attempts: *attempts,
            fence: *fence,
        };
        let after = TaskRecordImage {
            spec,
            status: next_status,
            attempts: *attempts,
            fence: *fence,
        };
        update_task(transaction, scope, &after)?;
        persist_attempt_transition(transaction, scope, &before, &after, "claim")?;
    }
    Ok(rows.len())
}

#[allow(clippy::too_many_arguments)]
fn plan_claims(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    machine_id: &str,
    eligible_targets: &BTreeSet<TargetId>,
    now_ms: u64,
    max_tasks: usize,
    quotas: &mut BTreeMap<String, QuotaRuntime>,
) -> Result<Vec<Candidate>, String> {
    let placeholders = std::iter::repeat_n("?", eligible_targets.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT task_id, spec_json, status_json, attempts, fence, quota_key, target_id
         FROM scheduler_tasks
         WHERE tenant_id = ? AND enterprise_id = ? AND manifest_id = ?
           AND state_kind IN ('pending', 'retry_wait') AND ready_at_ms <= ?
           AND target_id IN ({placeholders})
         ORDER BY priority DESC, ready_at_ms, task_id"
    );
    let mut values = vec![
        Value::Text(scope.tenant_id.to_owned()),
        Value::Text(scope.enterprise_id.to_owned()),
        Value::Text(scope.manifest_id.to_owned()),
        Value::Integer(i64::try_from(now_ms).map_err(|_| "scheduler claim time exceeds SQLite")?),
    ];
    values.extend(
        eligible_targets
            .iter()
            .map(|target| Value::Text(target.as_str().to_owned())),
    );
    let mut planned = Vec::new();
    let mut statement = transaction
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params_from_iter(values.iter()))
        .map_err(|error| error.to_string())?;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        if planned.len() >= max_tasks {
            break;
        }
        let task_id = row.get::<_, String>(0).map_err(|error| error.to_string())?;
        let spec_json = row
            .get::<_, Vec<u8>>(1)
            .map_err(|error| error.to_string())?;
        let status_json = row
            .get::<_, Vec<u8>>(2)
            .map_err(|error| error.to_string())?;
        let attempts = row.get::<_, u16>(3).map_err(|error| error.to_string())?;
        let fence = row.get::<_, u64>(4).map_err(|error| error.to_string())?;
        let quota_key = row.get::<_, String>(5).map_err(|error| error.to_string())?;
        let target_id = row.get::<_, String>(6).map_err(|error| error.to_string())?;
        let runtime = quotas
            .get_mut(&quota_key)
            .ok_or_else(|| "scheduler task has no quota policy".to_string())?;
        if runtime.in_flight >= usize::from(runtime.policy.max_in_flight)
            || now_ms < runtime.next_start_at_ms
        {
            continue;
        }
        let spec: TaskSpec =
            serde_json::from_slice(&spec_json).map_err(|error| error.to_string())?;
        spec.validate()?;
        let status: TaskStatus =
            serde_json::from_slice(&status_json).map_err(|error| error.to_string())?;
        if spec.task_id.as_str() != task_id
            || spec.target_id.as_str() != target_id
            || spec.quota_key().as_str() != quota_key
            || !eligible_targets.contains(&spec.target_id)
            || !matches!(
                status,
                TaskStatus::Pending { not_before_ms }
                    | TaskStatus::RetryWait { not_before_ms, .. }
                    if not_before_ms <= now_ms
            )
        {
            return Err("normalized scheduler claim candidate is inconsistent".into());
        }
        let expires_at_ms = now_ms
            .checked_add(runtime.policy.lease_duration_ms)
            .ok_or_else(|| "scheduler lease expiry overflow".to_string())?;
        let next_attempts = attempts
            .checked_add(1)
            .ok_or_else(|| "scheduler attempt counter overflow".to_string())?;
        let next_fence = fence
            .checked_add(1)
            .ok_or_else(|| "scheduler fence overflow".to_string())?;
        let next_status = TaskStatus::Leased {
            machine_id: machine_id.to_owned(),
            fence: next_fence,
            expires_at_ms,
        };
        runtime.in_flight += 1;
        runtime.next_start_at_ms = now_ms.saturating_add(runtime.policy.min_start_interval_ms);
        planned.push(Candidate {
            before: TaskRecordImage {
                spec: spec.clone(),
                status,
                attempts,
                fence,
            },
            after: TaskRecordImage {
                spec: spec.clone(),
                status: next_status,
                attempts: next_attempts,
                fence: next_fence,
            },
            claim: LeaseClaim {
                task: spec,
                machine_id: machine_id.to_owned(),
                fence: next_fence,
                attempt: next_attempts,
                expires_at_ms,
            },
        });
    }
    Ok(planned)
}

fn persist_quotas(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    before: &BTreeMap<String, QuotaRuntime>,
    after: &BTreeMap<String, QuotaRuntime>,
) -> Result<(), String> {
    for (key, runtime) in after {
        if before.get(key) == Some(runtime) {
            continue;
        }
        let updated = transaction
            .execute(
                "UPDATE scheduler_quotas
                 SET next_start_at_ms = ?5, in_flight = ?6, revision = revision + 1
                 WHERE tenant_id = ?1 AND enterprise_id = ?2
                   AND manifest_id = ?3 AND quota_key = ?4",
                params![
                    scope.tenant_id,
                    scope.enterprise_id,
                    scope.manifest_id,
                    key,
                    runtime.next_start_at_ms,
                    runtime.in_flight,
                ],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("normalized scheduler quota disappeared during mutation".into());
        }
    }
    Ok(())
}

fn retry_delay(policy: QuotaPolicy, attempts: u16) -> u64 {
    let exponent = u32::from(attempts.saturating_sub(1)).min(20);
    policy
        .base_backoff_ms
        .saturating_mul(1_u64 << exponent)
        .min(policy.max_backoff_ms)
}
