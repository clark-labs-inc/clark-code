//! Deterministic, I/O-free translation from safe adapter receipts to the
//! authoritative system-cartography observation wire.

use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{
    AdapterId, AdapterPageLimits, AdapterPageOutcome, AdapterPageReceipt, AdapterQuery,
    CursorHandle, NormalizedLink, NormalizedRecord, SafeFieldValue,
};
use scout_ingest_protocol::cartography::{
    ClaimedTask, Classification, EdgeIdentity, EntityIdentity, EvidenceObjectRef, ObservationEvent,
    ObservationFact, ObservationSubject, TaskCompletion, TerminalDisposition,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_TRANSLATED_EVENTS: usize = 10_000;
pub const ADAPTER_PAGE_TASK_KIND: &str = "adapter_page";
pub const ADAPTER_PAGE_TASK_SCOPE_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskBinding {
    pub source_id: Uuid,
    pub task_id: Uuid,
    pub fence: i64,
    pub first_source_sequence: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranslatedPage {
    pub events: Vec<ObservationEvent>,
    pub completion: TaskCompletion,
}

/// Backend-authored scope for one exact target-side adapter page.
///
/// The protocol deliberately omits target and authorization identities here: those
/// are established on the collector and cryptographically bound into the
/// receipt. Every discovery choice that the backend can know in advance is
/// pinned so a model cannot redirect a leased task to another authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterPageTaskScope {
    pub schema_version: u16,
    pub first_source_sequence: i64,
    pub adapter_id: AdapterId,
    pub enterprise_id: String,
    pub charter_id: String,
    pub discovery_epoch: u64,
    pub coverage_sequence: u64,
    pub region_or_project: String,
    pub resource_kind: String,
    pub query: AdapterQuery,
    pub page_ordinal: u32,
    pub cursor_handle: Option<CursorHandle>,
    pub limits: AdapterPageLimits,
}

pub fn task_binding(
    task: &ClaimedTask,
    receipt: &AdapterPageReceipt,
) -> Result<TaskBinding, String> {
    receipt
        .validate_at(receipt.observed_at_ms)
        .map_err(|error| error.to_string())?;
    if task.task_kind != ADAPTER_PAGE_TASK_KIND {
        return Err(format!(
            "claimed task kind must be `{ADAPTER_PAGE_TASK_KIND}` for an adapter receipt"
        ));
    }
    let scope: AdapterPageTaskScope = serde_json::from_value(task.scope.clone())
        .map_err(|_| "claimed adapter task has an invalid scope".to_string())?;
    if scope.schema_version != ADAPTER_PAGE_TASK_SCOPE_VERSION || scope.first_source_sequence <= 0 {
        return Err("claimed adapter task has an unsupported or invalid scope".into());
    }
    let request = &receipt.request;
    let coverage = &request.coverage;
    if scope.adapter_id != request.adapter_id
        || scope.enterprise_id != coverage.enterprise_id
        || scope.charter_id != coverage.charter_id
        || scope.discovery_epoch != coverage.discovery_epoch
        || scope.coverage_sequence != coverage.sequence
        || scope.region_or_project != coverage.region_or_project
        || scope.resource_kind != coverage.resource_kind
        || scope.query != request.query
        || scope.page_ordinal != request.page_ordinal
        || scope.cursor_handle != request.cursor_handle
        || scope.limits != request.limits
    {
        return Err(
            "target-bound adapter receipt does not match the backend-authored task scope".into(),
        );
    }
    Ok(TaskBinding {
        source_id: task.source_id,
        task_id: task.task_id,
        fence: task.fence,
        first_source_sequence: scope.first_source_sequence,
    })
}

pub fn translate_page(
    receipt: &AdapterPageReceipt,
    evidence: &EvidenceObjectRef,
    binding: &TaskBinding,
) -> Result<TranslatedPage, String> {
    receipt
        .validate_at(receipt.observed_at_ms)
        .map_err(|error| error.to_string())?;
    validate_binding(evidence, binding)?;
    let evidence_digests =
        BTreeSet::from([evidence.sha256.clone(), receipt.safe_page_sha256.clone()]);
    let mut facts = BTreeMap::new();
    for record in &receipt.records {
        let entity = entity_identity(record);
        insert_fact(
            &mut facts,
            entity.entity_id()?,
            ObservationFact {
                subject: ObservationSubject::Entity {
                    entity: entity.clone(),
                },
                attributes: record_attributes(record, receipt)?,
                evidence_digests: evidence_digests.clone(),
            },
        )?;
        for link in &record.links {
            let target = link_target(link);
            insert_fact(
                &mut facts,
                target.entity_id()?,
                ObservationFact {
                    subject: ObservationSubject::Entity {
                        entity: target.clone(),
                    },
                    attributes: json!({
                        "provider_type": link.target_provider_type,
                        "discovered_via": "adapter_link",
                    }),
                    evidence_digests: evidence_digests.clone(),
                },
            )?;
            let edge = EdgeIdentity {
                edge_kind: portable_namespace(&link.relationship_type),
                source: entity.clone(),
                target,
                qualifier: link.qualifier.clone(),
            };
            insert_fact(
                &mut facts,
                edge.edge_id()?,
                ObservationFact {
                    subject: ObservationSubject::Edge { edge },
                    attributes: json!({
                        "adapter_id": receipt.request.adapter_id,
                        "adapter_build_sha256": receipt.adapter_build_sha256,
                    }),
                    evidence_digests: evidence_digests.clone(),
                },
            )?;
        }
    }
    let (disposition, complete) = disposition(receipt);
    let coverage_key = format!(
        "coverage:{}",
        receipt
            .request
            .coverage
            .fingerprint_sha256()
            .map_err(|error| error.to_string())?
    );
    insert_fact(
        &mut facts,
        coverage_key.clone(),
        ObservationFact {
            subject: ObservationSubject::Coverage {
                coverage_key,
                disposition,
                complete,
                continuation_handle: receipt.next_cursor_handle.as_ref().map(ToString::to_string),
            },
            attributes: json!({
                "adapter_id": receipt.request.adapter_id,
                "adapter_build_sha256": receipt.adapter_build_sha256,
                "authority_scope": receipt.request.query.authority_scope,
                "provider_resource_type": receipt.request.query.provider_resource_type,
                "page_ordinal": receipt.request.page_ordinal,
                "record_count": receipt.records.len(),
                "redaction_summary": receipt.redaction_summary,
                "outcome": receipt.outcome,
            }),
            evidence_digests,
        },
    )?;
    if facts.len() > MAX_TRANSLATED_EVENTS {
        return Err(format!(
            "adapter page expands beyond {MAX_TRANSLATED_EVENTS} cartography events"
        ));
    }
    let mut events = Vec::with_capacity(facts.len());
    for (offset, fact) in facts.into_values().enumerate() {
        let offset =
            i64::try_from(offset).map_err(|_| "translated event offset overflow".to_string())?;
        let source_sequence = binding
            .first_source_sequence
            .checked_add(offset)
            .ok_or_else(|| "translated source sequence overflow".to_string())?;
        events.push(ObservationEvent::new(
            binding.source_id,
            binding.task_id,
            binding.fence,
            source_sequence,
            receipt.observed_at_ms,
            Classification::Internal,
            evidence.clone(),
            fact,
        )?);
    }
    Ok(TranslatedPage {
        events,
        completion: TaskCompletion {
            task_id: binding.task_id,
            fence: binding.fence,
            disposition,
            evidence_sha256: Some(evidence.sha256.clone()),
            detail: Some(format!(
                "adapter_receipt={} page={} records={}",
                receipt.receipt_id,
                receipt.request.page_ordinal,
                receipt.records.len()
            )),
        },
    })
}

fn validate_binding(evidence: &EvidenceObjectRef, binding: &TaskBinding) -> Result<(), String> {
    if binding.source_id.is_nil()
        || binding.task_id.is_nil()
        || binding.fence <= 0
        || binding.first_source_sequence <= 0
        || evidence.version_id.as_deref().is_none_or(str::is_empty)
    {
        return Err("adapter translation requires a fenced task and versioned evidence".into());
    }
    Ok(())
}

fn insert_fact(
    facts: &mut BTreeMap<String, ObservationFact>,
    object_id: String,
    fact: ObservationFact,
) -> Result<(), String> {
    if let Some(existing) = facts.get(&object_id) {
        if existing != &fact {
            return Err("adapter page contains conflicting observations for one object id".into());
        }
    } else {
        facts.insert(object_id, fact);
    }
    Ok(())
}

fn entity_identity(record: &NormalizedRecord) -> EntityIdentity {
    EntityIdentity {
        entity_kind: record
            .semantic_kind
            .as_deref()
            .map(portable_namespace)
            .unwrap_or_else(|| portable_namespace(&record.provider_type)),
        provider_namespace: portable_namespace(&record.provider_namespace),
        authority_scope: record.identity_authority_scope.clone(),
        provider_native_id: record.native_id.clone(),
    }
}

fn link_target(link: &NormalizedLink) -> EntityIdentity {
    EntityIdentity {
        entity_kind: portable_namespace(&link.target_provider_type),
        provider_namespace: portable_namespace(&link.target_provider_namespace),
        authority_scope: link.target_authority_scope.clone(),
        provider_native_id: link.target_native_id.clone(),
    }
}

fn record_attributes(
    record: &NormalizedRecord,
    receipt: &AdapterPageReceipt,
) -> Result<Value, String> {
    let fields = record
        .fields
        .iter()
        .map(|(name, value)| Ok((name.clone(), safe_value(value)?)))
        .collect::<Result<serde_json::Map<_, _>, String>>()?;
    Ok(json!({
        "provider_type": record.provider_type,
        "semantic_kind": record.semantic_kind,
        "labels": record.labels,
        "fields": fields,
        "record_id": record.record_id,
        "adapter_id": receipt.request.adapter_id,
        "adapter_build_sha256": receipt.adapter_build_sha256,
    }))
}

fn safe_value(value: &SafeFieldValue) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "failed to encode a safe adapter field".into())
}

fn disposition(receipt: &AdapterPageReceipt) -> (TerminalDisposition, bool) {
    match receipt.outcome {
        AdapterPageOutcome::Succeeded { final_page: true } if receipt.records.is_empty() => {
            (TerminalDisposition::Empty, true)
        }
        AdapterPageOutcome::Succeeded { final_page: true } => {
            (TerminalDisposition::Supported, true)
        }
        AdapterPageOutcome::Succeeded { final_page: false }
        | AdapterPageOutcome::Truncated { .. } => (TerminalDisposition::Truncated, false),
        AdapterPageOutcome::Denied { .. } => (TerminalDisposition::Denied, false),
        AdapterPageOutcome::Unreachable { .. } => (TerminalDisposition::Unreachable, false),
        AdapterPageOutcome::Unsupported { .. } => (TerminalDisposition::Unsupported, false),
        AdapterPageOutcome::Unsafe { .. } => (TerminalDisposition::Unsafe, false),
        AdapterPageOutcome::Stale { .. } => (TerminalDisposition::Stale, false),
    }
}

fn portable_namespace(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(128));
    for byte in value.bytes().take(128) {
        let byte = byte.to_ascii_lowercase();
        output.push(
            if byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            {
                char::from(byte)
            } else {
                '_'
            },
        );
    }
    if output.is_empty() {
        "unknown".into()
    } else {
        output
    }
}
