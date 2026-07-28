use agent_orchestration::{EnterpriseFact, EnterpriseId};
use rusqlite::{params, OptionalExtension, Transaction};
use scout_adapter_protocol::{AdapterPageOutcome, AdapterPageReceipt};
use scout_ingest_protocol::{IngestReceipt, IngestRequest, ScoutTenantId};
use scout_scheduler::{
    CompletionDisposition, PageCompletion, TaskOrigin, TaskSpec, TerminalDisposition,
};

use super::model::TaskRecordImage;
use super::storage::SchedulerScope;

pub(super) fn validate_atomic_page_inputs(
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    adapter_receipt: &AdapterPageReceipt,
    ingest_request: &IngestRequest,
    completion: &PageCompletion,
    observed_at_ms: u64,
) -> Result<(), String> {
    adapter_receipt
        .validate_at(adapter_receipt.observed_at_ms)
        .map_err(|error| error.to_string())?;
    if observed_at_ms < adapter_receipt.observed_at_ms
        || observed_at_ms < completion.completed_at_ms
    {
        return Err("atomic page commit time precedes its adapter evidence".into());
    }
    if &ingest_request.tenant_id != tenant_id
        || &ingest_request.bundle.signed_batch.batch.enterprise_id != enterprise_id
        || adapter_receipt.request.coverage.enterprise_id != enterprise_id.as_str()
    {
        return Err("atomic page commit crosses a tenant or enterprise boundary".into());
    }
    if completion.receipt_id.as_deref() != Some(adapter_receipt.receipt_id.as_str())
        || completion.evidence_sha256.as_deref() != Some(adapter_receipt.safe_page_sha256.as_str())
    {
        return Err("scheduler completion is not bound to the exact adapter receipt".into());
    }
    validate_outcome_binding(adapter_receipt, completion)?;

    let target_id = adapter_receipt.target.target_id.to_string();
    let auth_context_id = adapter_receipt.auth_context.context_id.to_string();
    let events = &ingest_request.bundle.signed_batch.batch.events;
    if events.iter().any(|event| {
        event.provenance.source_fingerprint != adapter_receipt.safe_page_sha256
            || event.provenance.machine_id != target_id
            || event.provenance.auth_context_id != auth_context_id
            || !fact_cites_page(&event.fact, &adapter_receipt.safe_page_sha256)
    }) {
        return Err("signed batch provenance is not bound to the adapter page".into());
    }
    Ok(())
}

fn fact_cites_page(fact: &EnterpriseFact, safe_page_sha256: &str) -> bool {
    match fact {
        EnterpriseFact::EntityObserved(value) => value.evidence_digests.contains(safe_page_sha256),
        EnterpriseFact::EdgeObserved(value) => value.evidence_digests.contains(safe_page_sha256),
        EnterpriseFact::CoverageObserved(value) => {
            value.evidence_digests.contains(safe_page_sha256)
        }
        EnterpriseFact::FrontierObserved(value) => {
            value.evidence_digests.contains(safe_page_sha256)
        }
        EnterpriseFact::SimulationContractObserved(value) => {
            value.evidence_digests.contains(safe_page_sha256)
        }
        EnterpriseFact::DiscoveryCharterObserved(_)
        | EnterpriseFact::DiscoveryPassSealed(_)
        | EnterpriseFact::ObservationRetracted { .. } => false,
    }
}

pub(super) fn validate_task_binding(
    task: &TaskRecordImage,
    adapter_receipt: &AdapterPageReceipt,
    completion: &PageCompletion,
) -> Result<(), String> {
    let spec = &task.spec;
    let request = &adapter_receipt.request;
    if completion.task_id != spec.task_id
        || request.target_id != spec.target_id
        || request.adapter_id != spec.adapter_id
        || request.auth_context_id != spec.auth_context_id
        || request.auth_context_handle != spec.auth_context_handle
        || request.coverage != spec.coverage
        || request.query != spec.query
        || request.page_ordinal != spec.page_ordinal
        || request.cursor_handle != spec.cursor_handle
    {
        return Err("adapter page does not match the leased scheduler task".into());
    }
    if let Some(continuation) = &completion.continuation {
        validate_continuation(spec, continuation, adapter_receipt)?;
    } else if adapter_receipt.next_cursor_handle.is_some()
        && matches!(
            completion.disposition,
            CompletionDisposition::Success { final_page: false }
        )
    {
        return Err("nonterminal adapter page is missing its scheduler continuation".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn persist_page_commit(
    transaction: &Transaction<'_>,
    scope: SchedulerScope<'_>,
    operation_id: &str,
    adapter_receipt: &AdapterPageReceipt,
    ingest_request: &IngestRequest,
    ingest_receipt: &IngestReceipt,
    completion: &PageCompletion,
) -> Result<(), String> {
    let batch_id = ingest_request.bundle.signed_batch.batch.batch_id.as_str();
    let adapter_receipt_json =
        serde_json::to_vec(adapter_receipt).map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO scheduler_page_commits (
                 tenant_id, enterprise_id, manifest_id, task_id, fence,
                 operation_id, adapter_receipt_id, safe_page_sha256,
                 adapter_receipt_json, batch_id, ingest_receipt_id
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
             )",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                completion.task_id.as_str(),
                completion.fence,
                operation_id,
                adapter_receipt.receipt_id.as_str(),
                adapter_receipt.safe_page_sha256,
                adapter_receipt_json,
                batch_id,
                ingest_receipt.receipt_id,
            ],
        )
        .map_err(|error| error.to_string())?;
    let stored = transaction
        .query_row(
            "SELECT operation_id, adapter_receipt_id, safe_page_sha256,
                    adapter_receipt_json, batch_id, ingest_receipt_id
             FROM scheduler_page_commits
             WHERE tenant_id = ?1 AND enterprise_id = ?2
               AND manifest_id = ?3 AND task_id = ?4 AND fence = ?5",
            params![
                scope.tenant_id,
                scope.enterprise_id,
                scope.manifest_id,
                completion.task_id.as_str(),
                completion.fence,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let expected = (
        operation_id.to_owned(),
        adapter_receipt.receipt_id.to_string(),
        adapter_receipt.safe_page_sha256.clone(),
        serde_json::to_vec(adapter_receipt).map_err(|error| error.to_string())?,
        batch_id.to_owned(),
        ingest_receipt.receipt_id.clone(),
    );
    if stored.as_ref() != Some(&expected) {
        return Err("atomic page commit collides with different durable evidence".into());
    }
    Ok(())
}

fn validate_outcome_binding(
    adapter_receipt: &AdapterPageReceipt,
    completion: &PageCompletion,
) -> Result<(), String> {
    let valid = match (&adapter_receipt.outcome, &completion.disposition) {
        (
            AdapterPageOutcome::Succeeded { final_page },
            CompletionDisposition::Success {
                final_page: completed_final,
            },
        ) => final_page == completed_final,
        (AdapterPageOutcome::Succeeded { final_page: true }, CompletionDisposition::Empty) => {
            adapter_receipt.records.is_empty()
        }
        (
            AdapterPageOutcome::Denied { .. },
            CompletionDisposition::Gap {
                terminal: TerminalDisposition::Denied,
            },
        )
        | (
            AdapterPageOutcome::Unreachable { .. },
            CompletionDisposition::Gap {
                terminal: TerminalDisposition::Unreachable,
            },
        )
        | (
            AdapterPageOutcome::Unsupported { .. },
            CompletionDisposition::Gap {
                terminal: TerminalDisposition::Unsupported,
            },
        )
        | (
            AdapterPageOutcome::Unsafe { .. },
            CompletionDisposition::Gap {
                terminal: TerminalDisposition::Unsafe,
            },
        )
        | (
            AdapterPageOutcome::Stale { .. },
            CompletionDisposition::Gap {
                terminal: TerminalDisposition::Stale,
            },
        ) => true,
        (
            AdapterPageOutcome::Truncated {
                continuation_available: true,
                ..
            },
            CompletionDisposition::Success { final_page: false },
        ) => true,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("scheduler completion disposition disagrees with adapter outcome".into())
    }
}

fn validate_continuation(
    parent: &TaskSpec,
    continuation: &TaskSpec,
    adapter_receipt: &AdapterPageReceipt,
) -> Result<(), String> {
    let expected_page = parent
        .page_ordinal
        .checked_add(1)
        .ok_or_else(|| "scheduler page ordinal overflow".to_string())?;
    if continuation.enterprise_id != parent.enterprise_id
        || continuation.charter_id != parent.charter_id
        || continuation.discovery_epoch != parent.discovery_epoch
        || continuation.target_id != parent.target_id
        || continuation.adapter_id != parent.adapter_id
        || continuation.auth_context_id != parent.auth_context_id
        || continuation.auth_context_handle != parent.auth_context_handle
        || continuation.coverage != parent.coverage
        || continuation.query != parent.query
        || continuation.page_ordinal != expected_page
        || continuation.cursor_handle != adapter_receipt.next_cursor_handle
        || !matches!(
            &continuation.origin,
            TaskOrigin::Continuation { parent_task_id }
                if parent_task_id == &parent.task_id
        )
    {
        return Err("scheduler continuation is not the exact next adapter page".into());
    }
    Ok(())
}
