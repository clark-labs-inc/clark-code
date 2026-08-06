use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use scout_adapter_protocol::{AdapterPageReceipt, ReceiptId};
use scout_cartography_adapter::{task_binding, translate_page};
use scout_ingest_protocol::cartography::{BatchAcceptance, ClaimedTask, EvidenceObjectRef};
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
}

impl PendingSubmissions {
    pub(super) fn record_claim(&self, run_id: Uuid, task: ClaimedTask) -> Result<(), String> {
        if run_id.is_nil()
            || task.task_id.is_nil()
            || task.source_id.is_nil()
            || task.fence <= 0
            || !task.scope.is_object()
        {
            return Err("Clark returned an invalid claimed Scout task".into());
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
                Err("Clark returned conflicting leases for one Scout task".into())
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
        if let Some(existing) = state.receipts.get(&receipt_id) {
            return if existing == &receipt {
                Ok(())
            } else {
                Err("target returned conflicting content for one adapter receipt id".into())
            };
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
    Ok(SubmittedAdapterReceipt {
        task_id: binding.task_id,
        adapter_receipt_id: reserved.receipt_id.clone(),
        evidence,
        acceptance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_state_rejects_unretained_model_selected_ids() {
        let pending = PendingSubmissions::default();
        assert!(pending
            .reserve(Uuid::new_v4(), &format!("receipt:{}", "a".repeat(64)))
            .unwrap_err()
            .contains("not retained"));
    }
}
