use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use scout_adapter_protocol::{AdapterPageReceipt, ReceiptId};
use scout_cartography_adapter::{task_binding, translate_page};
use scout_ingest_protocol::cartography::{BatchAcceptance, ClaimedTask, EvidenceObjectRef};
use scout_platform_client::ScoutRunAdvanceReceipt;
use uuid::Uuid;

use super::CartographyBackendState;

const MAX_PENDING_CLAIMS: usize = 256;
const MAX_PENDING_RECEIPTS: usize = 256;
const RECEIPT_CONTENT_TYPE: &str = "application/json";

#[derive(Clone, Debug, PartialEq)]
struct StoredClaim {
    run_id: Uuid,
    task: ClaimedTask,
}

#[derive(Default)]
struct PendingState {
    claims: BTreeMap<Uuid, StoredClaim>,
    receipts: BTreeMap<String, AdapterPageReceipt>,
    in_flight_tasks: BTreeSet<Uuid>,
    in_flight_receipts: BTreeSet<String>,
}

#[derive(Default)]
pub(super) struct PendingSubmissions {
    state: Mutex<PendingState>,
}

#[derive(Clone, Debug)]
struct ReservedSubmission {
    claim: StoredClaim,
    receipt_id: String,
    receipt: AdapterPageReceipt,
}

pub(super) struct SubmittedAdapterReceipt {
    pub task_id: Uuid,
    pub adapter_receipt_id: String,
    pub evidence: EvidenceObjectRef,
    pub acceptance: BatchAcceptance,
    pub advancement: ScoutRunAdvanceReceipt,
}

impl PendingSubmissions {
    pub(super) fn record_claim(&self, run_id: Uuid, task: ClaimedTask) -> Result<(), String> {
        if run_id.is_nil()
            || task.task_id.is_nil()
            || task.source_id.is_nil()
            || task.fence <= 0
            || !task.scope.is_object()
        {
            return Err("Clark Code returned an invalid claimed Scout task".into());
        }
        let stored = StoredClaim { run_id, task };
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Scout pending-submission state is unavailable".to_string())?;
        if let Some(existing) = state.claims.get(&stored.task.task_id) {
            return if existing == &stored {
                Ok(())
            } else {
                Err("Clark Code returned conflicting leases for one Scout task".into())
            };
        }
        if state.claims.len() >= MAX_PENDING_CLAIMS {
            return Err("Scout pending claim capacity is exhausted; allow leases to expire".into());
        }
        state.claims.insert(stored.task.task_id, stored);
        Ok(())
    }

    pub(super) fn record_receipt(&self, receipt: AdapterPageReceipt) -> Result<(), String> {
        receipt
            .validate_at(receipt.observed_at_ms)
            .map_err(|error| format!("target returned an invalid adapter receipt: {error}"))?;
        let receipt_id = receipt.receipt_id.to_string();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Scout pending-submission state is unavailable".to_string())?;
        let mut matching_claims = state
            .claims
            .values()
            .filter(|claim| task_binding(&claim.task, &receipt).is_ok());
        let claim = matching_claims.next().ok_or_else(|| {
            "target receipt does not match a retained backend task claim".to_string()
        })?;
        if matching_claims.next().is_some() {
            return Err(
                "target receipt ambiguously matches multiple retained backend task claims".into(),
            );
        }
        let task_id = claim.task.task_id;
        if let Some(existing) = state.receipts.get(&receipt_id) {
            return if existing == &receipt {
                Ok(())
            } else {
                Err("target returned conflicting content for one adapter receipt id".into())
            };
        }
        if state.receipts.values().any(|existing| {
            state
                .claims
                .get(&task_id)
                .is_some_and(|claim| task_binding(&claim.task, existing).is_ok())
        }) {
            return Err(
                "the retained backend task already has a pending adapter receipt; submit it before collecting again"
                    .into(),
            );
        }
        if state.receipts.len() >= MAX_PENDING_RECEIPTS {
            return Err(
                "Scout pending receipt capacity is exhausted; submit retained receipts first"
                    .into(),
            );
        }
        state.receipts.insert(receipt_id, receipt);
        Ok(())
    }

    pub(super) fn claimed_task_for_adapter(&self, adapter_id: &str) -> Result<ClaimedTask, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "Scout pending-submission state is unavailable".to_string())?;
        let mut matches = state.claims.values().filter_map(|claim| {
            let scope: scout_cartography_adapter::AdapterPageTaskScope =
                serde_json::from_value(claim.task.scope.clone()).ok()?;
            (scope.adapter_id.as_str() == adapter_id).then(|| claim.task.clone())
        });
        let task = matches.next().ok_or_else(|| {
            format!("claim a backend `{adapter_id}` task before collecting its evidence")
        })?;
        if matches.next().is_some() {
            return Err(format!(
                "multiple pending `{adapter_id}` tasks are retained; submit or expire the earlier lease"
            ));
        }
        Ok(task)
    }

    fn reserve(&self, task_id: Uuid, receipt_id: &str) -> Result<ReservedSubmission, String> {
        let receipt_id = ReceiptId::new(receipt_id)
            .map_err(|_| "submit_adapter_receipt requires a canonical receipt id".to_string())?
            .to_string();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Scout pending-submission state is unavailable".to_string())?;
        if state.in_flight_tasks.contains(&task_id)
            || state.in_flight_receipts.contains(&receipt_id)
        {
            return Err("the selected Scout task or receipt is already being submitted".into());
        }
        let claim = state.claims.get(&task_id).cloned().ok_or_else(|| {
            "task_id was not retained from this host's claim_task result".to_string()
        })?;
        let receipt = state.receipts.get(&receipt_id).cloned().ok_or_else(|| {
            "receipt_id was not retained from this host's scout_adapter fetch_page result"
                .to_string()
        })?;
        task_binding(&claim.task, &receipt)?;
        state.in_flight_tasks.insert(task_id);
        state.in_flight_receipts.insert(receipt_id.clone());
        Ok(ReservedSubmission {
            claim,
            receipt_id,
            receipt,
        })
    }

    fn finish(&self, reserved: &ReservedSubmission, succeeded: bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.in_flight_tasks.remove(&reserved.claim.task.task_id);
        state.in_flight_receipts.remove(&reserved.receipt_id);
        if succeeded {
            state.claims.remove(&reserved.claim.task.task_id);
            state.receipts.remove(&reserved.receipt_id);
        }
    }
}

pub(super) async fn submit_adapter_receipt(
    state: &CartographyBackendState,
    task_id: Uuid,
    receipt_id: &str,
) -> Result<SubmittedAdapterReceipt, String> {
    let reserved = state.pending.reserve(task_id, receipt_id)?;
    let result = submit_reserved(state, &reserved).await;
    state.pending.finish(&reserved, result.is_ok());
    result
}

async fn submit_reserved(
    state: &CartographyBackendState,
    reserved: &ReservedSubmission,
) -> Result<SubmittedAdapterReceipt, String> {
    let session = state.ready()?;
    let binding = task_binding(&reserved.claim.task, &reserved.receipt)?;
    let bytes = serde_json::to_vec(&reserved.receipt)
        .map_err(|_| "failed to encode the retained adapter receipt".to_string())?;
    let evidence = session
        .upload_evidence_idempotent(
            reserved.claim.run_id,
            binding.source_id,
            binding.task_id,
            binding.fence,
            &reserved.receipt_id,
            reserved.receipt.observed_at_ms,
            RECEIPT_CONTENT_TYPE,
            &bytes,
        )
        .await?;
    let translated = translate_page(&reserved.receipt, &evidence, &binding)?;
    let acceptance = session
        .ingest(
            reserved.claim.run_id,
            translated.events,
            vec![translated.completion],
        )
        .await?;
    let advancement = session
        .advance_run(
            reserved.claim.run_id,
            binding.task_id,
            acceptance.receipt.receipt_id.clone(),
        )
        .await?;
    Ok(SubmittedAdapterReceipt {
        task_id: binding.task_id,
        adapter_receipt_id: reserved.receipt_id.clone(),
        evidence,
        acceptance,
        advancement,
    })
}

#[cfg(test)]
mod tests {
    use scout_adapter_protocol::{
        AdapterId, AdapterPageLimits, AdapterPageOutcome, AdapterPageRequest, AdapterQuery,
        AuthContextDescriptor, AuthContextHandle, AuthSourceKind, CoverageBinding,
        RedactionSummary, RequestId, TargetIdentity,
    };
    use scout_cartography_adapter::{
        AdapterPageTaskScope, ADAPTER_PAGE_TASK_KIND, ADAPTER_PAGE_TASK_SCOPE_VERSION,
    };

    use super::*;

    #[test]
    fn pending_state_rejects_unretained_model_selected_ids() {
        let pending = PendingSubmissions::default();
        assert!(pending
            .reserve(Uuid::new_v4(), &format!("receipt:{}", "a".repeat(64)))
            .unwrap_err()
            .contains("not retained"));
    }

    #[test]
    fn receipt_must_bind_one_claim_and_each_claim_retains_only_one_receipt() {
        let first = receipt(1_100, '2');
        let second = receipt(1_200, '3');
        let pending = PendingSubmissions::default();

        assert!(pending
            .record_receipt(first.clone())
            .unwrap_err()
            .contains("does not match a retained backend task claim"));

        pending
            .record_claim(Uuid::new_v4(), claimed_task(&first))
            .unwrap();
        pending.record_receipt(first).unwrap();
        assert!(pending
            .record_receipt(second)
            .unwrap_err()
            .contains("already has a pending adapter receipt"));
    }

    fn receipt(observed_at_ms: u64, request_marker: char) -> AdapterPageReceipt {
        let adapter_id = AdapterId::new("clark/test-system@1").unwrap();
        let target = TargetIdentity::new(
            digest('1'),
            digest('2'),
            digest('3'),
            digest('4'),
            "linux".into(),
            "x86_64".into(),
        )
        .unwrap();
        let auth = AuthContextDescriptor::new(
            AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000001").unwrap(),
            target.target_id.clone(),
            adapter_id.clone(),
            "test".into(),
            "test".into(),
            "principal:1".into(),
            AuthSourceKind::CliProfile,
            digest('5'),
            900,
            Some(10_000),
        )
        .unwrap();
        let request = AdapterPageRequest {
            protocol_version: scout_adapter_protocol::ADAPTER_PROTOCOL_VERSION,
            request_id: RequestId::new(format!(
                "request:00000000-0000-4000-8000-00000000000{request_marker}"
            ))
            .unwrap(),
            target_id: target.target_id.clone(),
            target_identity_sha256: target.fingerprint_sha256().unwrap(),
            adapter_id,
            auth_context_handle: auth.handle.clone(),
            auth_context_id: auth.context_id.clone(),
            coverage: CoverageBinding {
                enterprise_id: "enterprise:test".into(),
                charter_id: "charter:test".into(),
                discovery_epoch: 1,
                sequence: 1,
                adapter_id: auth.adapter_id.clone(),
                auth_context_id: auth.context_id.clone(),
                tenant: "test".into(),
                region_or_project: "global".into(),
                resource_kind: "system".into(),
            },
            query: AdapterQuery {
                operation: "list_systems".into(),
                authority_scope: "test".into(),
                provider_resource_type: "test.system".into(),
                filters: BTreeMap::new(),
                projection: BTreeSet::from(["name".into()]),
                page_size: 10,
            },
            page_ordinal: 0,
            cursor_handle: None,
            limits: AdapterPageLimits {
                max_records: 10,
                max_response_bytes: 10_000,
                max_duration_ms: 1_000,
            },
            requested_at_ms: 1_000,
        };
        AdapterPageReceipt::new(
            request,
            target,
            auth,
            digest('6'),
            observed_at_ms,
            AdapterPageOutcome::Succeeded { final_page: true },
            Vec::new(),
            None,
            RedactionSummary {
                source_records_seen: 0,
                records_emitted: 0,
                fields_omitted: 0,
                values_rejected: 0,
            },
        )
        .unwrap()
    }

    fn claimed_task(receipt: &AdapterPageReceipt) -> ClaimedTask {
        let coverage = &receipt.request.coverage;
        let scope = AdapterPageTaskScope {
            schema_version: ADAPTER_PAGE_TASK_SCOPE_VERSION,
            first_source_sequence: 1,
            adapter_id: receipt.request.adapter_id.clone(),
            enterprise_id: coverage.enterprise_id.clone(),
            charter_id: coverage.charter_id.clone(),
            discovery_epoch: coverage.discovery_epoch,
            coverage_sequence: coverage.sequence,
            region_or_project: coverage.region_or_project.clone(),
            resource_kind: coverage.resource_kind.clone(),
            query: receipt.request.query.clone(),
            page_ordinal: receipt.request.page_ordinal,
            cursor_handle: receipt.request.cursor_handle.clone(),
            limits: receipt.request.limits,
        };
        ClaimedTask {
            task_id: Uuid::new_v4(),
            source_id: Uuid::new_v4(),
            task_kind: ADAPTER_PAGE_TASK_KIND.into(),
            scope: serde_json::to_value(scope).unwrap(),
            fence: 1,
            lease_expires_at: "2026-08-12T23:59:59Z".into(),
        }
    }

    fn digest(marker: char) -> String {
        std::iter::repeat_n(marker, 64).collect()
    }
}
