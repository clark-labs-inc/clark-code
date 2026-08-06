use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection, OptionalExtension};
use scout_scheduler::{QuotaKey, QuotaPolicy, ScheduleManifest, TaskSpec, TaskStatus};

use super::SchedulerScope;
use crate::scheduler::model::{BindingImage, QuotaRuntimeImage, SchedulerImage, TaskRecordImage};

pub(in crate::scheduler) fn load_scheduler(
    connection: &Connection,
    scope: SchedulerScope<'_>,
) -> Result<Option<SchedulerImage>, String> {
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
    let Some((manifest_json, generation, state_sha256)) = metadata else {
        return Ok(None);
    };
    let manifest: ScheduleManifest =
        serde_json::from_slice(&manifest_json).map_err(|error| error.to_string())?;
    if manifest.manifest_id != scope.manifest_id || manifest.enterprise_id != scope.enterprise_id {
        return Err("normalized scheduler manifest scope disagrees with its row key".into());
    }

    let bindings = read_bindings(connection, scope)?;
    let mut tasks = BTreeMap::new();
    let mut in_flight = BTreeMap::<QuotaKey, BTreeSet<_>>::new();
    let mut statement = connection
        .prepare(
            "SELECT task_id, binding_id, spec_json, status_json, attempts, fence,
                    priority, ready_at_ms, state_kind, target_id, quota_key,
                    lease_machine_id, lease_expires_at_ms
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
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, u16>(4)?,
                    row.get::<_, u64>(5)?,
                    row.get::<_, u16>(6)?,
                    row.get::<_, u64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<u64>>(12)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (
            task_id,
            binding_id,
            spec_json,
            status_json,
            attempts,
            fence,
            priority,
            ready_at_ms,
            state_kind,
            target_id,
            quota_key,
            lease_machine_id,
            lease_expires_at_ms,
        ) = row.map_err(|error| error.to_string())?;
        let spec: TaskSpec =
            serde_json::from_slice(&spec_json).map_err(|error| error.to_string())?;
        let status: TaskStatus =
            serde_json::from_slice(&status_json).map_err(|error| error.to_string())?;
        if spec.task_id.as_str() != task_id {
            return Err("normalized scheduler task id disagrees with its specification".into());
        }
        let binding = BindingImage::from_spec(&spec);
        if binding.id()? != binding_id
            || bindings.get(&binding_id)
                != Some(&serde_json::to_vec(&binding).map_err(|error| error.to_string())?)
        {
            return Err("normalized scheduler task binding is missing or inconsistent".into());
        }
        let record = TaskRecordImage {
            spec,
            status,
            attempts,
            fence,
        };
        let columns = record.columns();
        let derived_quota = record.spec.quota_key();
        if priority != record.spec.priority
            || state_kind != columns.state_kind
            || ready_at_ms != columns.ready_at_ms
            || target_id != record.spec.target_id.as_str()
            || quota_key != derived_quota.as_str()
            || lease_machine_id.as_deref() != columns.lease_machine_id
            || lease_expires_at_ms != columns.lease_expires_at_ms
        {
            return Err("normalized scheduler task index columns are inconsistent".into());
        }
        if matches!(record.status, TaskStatus::Leased { .. }) {
            in_flight
                .entry(derived_quota)
                .or_default()
                .insert(record.spec.task_id.clone());
        }
        let key = record.spec.task_id.clone();
        if tasks.insert(key, record).is_some() {
            return Err("normalized scheduler contains a duplicate task".into());
        }
    }
    drop(statement);

    let quotas = read_quotas(connection, scope, &manifest, in_flight)?;
    let image = SchedulerImage {
        manifest,
        tasks,
        quotas,
        generation,
    };
    let receipt = image.receipt()?;
    if receipt.generation != generation || receipt.state_sha256 != state_sha256 {
        return Err("normalized scheduler receipt does not match persisted metadata".into());
    }
    Ok(Some(image))
}

fn read_bindings(
    connection: &Connection,
    scope: SchedulerScope<'_>,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut statement = connection
        .prepare(
            "SELECT binding_id, binding_json
             FROM scheduler_bindings
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND manifest_id = ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![scope.tenant_id, scope.enterprise_id, scope.manifest_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|error| error.to_string())
}

fn read_quotas(
    connection: &Connection,
    scope: SchedulerScope<'_>,
    manifest: &ScheduleManifest,
    mut in_flight: BTreeMap<QuotaKey, BTreeSet<scout_scheduler::SchedulerTaskId>>,
) -> Result<BTreeMap<QuotaKey, QuotaRuntimeImage>, String> {
    let mut quotas = BTreeMap::new();
    let mut statement = connection
        .prepare(
            "SELECT quota_key, policy_json, next_start_at_ms, in_flight
             FROM scheduler_quotas
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND manifest_id = ?3",
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
        let (key_text, policy_json, next_start_at_ms, stored_in_flight) =
            row.map_err(|error| error.to_string())?;
        let (key, policy) = manifest
            .quota_policies
            .iter()
            .find(|(key, _)| key.as_str() == key_text)
            .ok_or_else(|| "normalized scheduler has an unknown quota".to_string())?;
        let stored_policy: QuotaPolicy =
            serde_json::from_slice(&policy_json).map_err(|error| error.to_string())?;
        if &stored_policy != policy {
            return Err("normalized scheduler quota policy is inconsistent".into());
        }
        let members = in_flight.remove(key).unwrap_or_default();
        if stored_in_flight != members.len() {
            return Err("normalized scheduler quota in-flight count is inconsistent".into());
        }
        quotas.insert(
            key.clone(),
            QuotaRuntimeImage {
                in_flight: members,
                next_start_at_ms,
            },
        );
    }
    if quotas.len() != manifest.quota_policies.len() || !in_flight.is_empty() {
        return Err("normalized scheduler quota coverage is incomplete".into());
    }
    Ok(quotas)
}
