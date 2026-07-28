use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use scout_accumulator::{verify_proof, ProofStatus};
use scout_coordinator::CoordinatorStore;
use scout_ingest_protocol::{CoordinatorSigningKey, IngestReceipt, IngestRequest, ScoutTenantId};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::enterprise_eval;

pub fn central_ingestion(
    service_count: usize,
    machine_count: usize,
) -> Result<(String, Value), String> {
    let (chain, envelopes) = enterprise_eval::signed_scale_fixture(service_count, machine_count)?;
    let first = envelopes
        .first()
        .ok_or_else(|| "central-ingestion benchmark has no signed batches".to_string())?;
    let enterprise_id = first.batch.enterprise_id.clone();
    let tenant_id = ScoutTenantId::new("organization:benchmark")?;
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let signer_seed = [0x73; 32];
    let coordinator =
        CoordinatorStore::open(root.path(), CoordinatorSigningKey::from_seed(signer_seed))?;
    coordinator.pin_enterprise(
        &tenant_id,
        &enterprise_id,
        &chain.anchor_manifest_id,
        &chain,
    )?;
    let requests = envelopes
        .into_iter()
        .enumerate()
        .map(|(index, signed_batch)| {
            let digest = Sha256::digest(signed_batch.batch.batch_id.as_str().as_bytes());
            IngestRequest::new(
                tenant_id.clone(),
                format!("outbox-attempt:{digest:x}"),
                agent_orchestration::EnterpriseBatchBundle {
                    trust_chain: chain.clone(),
                    signed_batch,
                },
            )
            .map(|request| (index, request))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let request_count = requests.len();
    let duplicate_request = requests
        .first()
        .map(|(_, request)| request.clone())
        .ok_or_else(|| "central-ingestion benchmark has no upload requests".to_string())?;
    let worker_count = machine_count.max(1).min(request_count);
    let queue = Arc::new(Mutex::new(VecDeque::from(requests)));
    let started = Instant::now();
    let receipts = thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..worker_count {
            let coordinator = coordinator.clone();
            let queue = Arc::clone(&queue);
            workers.push(scope.spawn(move || {
                let mut receipts = Vec::new();
                loop {
                    let next = queue
                        .lock()
                        .map_err(|_| "central-ingestion work queue was poisoned".to_string())?
                        .pop_front();
                    let Some((index, request)) = next else {
                        break;
                    };
                    receipts.push((
                        index,
                        coordinator.ingest(&request.tenant_id, &request, 100_000 + index as u64)?,
                    ));
                }
                Ok::<_, String>(receipts)
            }));
        }
        let mut receipts = Vec::with_capacity(request_count);
        for worker in workers {
            receipts.extend(
                worker
                    .join()
                    .map_err(|_| "central-ingestion benchmark worker panicked".to_string())??,
            );
        }
        Ok::<_, String>(receipts)
    })?;
    let concurrent_ingest_ms = started.elapsed().as_millis();
    verify_receipt_chain(
        &receipts,
        request_count,
        &coordinator.coordinator_public_key(),
    )?;

    let original = coordinator.ingest(&tenant_id, &duplicate_request, 200_000)?;
    drop(coordinator);
    let reopened =
        CoordinatorStore::open(root.path(), CoordinatorSigningKey::from_seed(signer_seed))?;
    let replayed = reopened.ingest(&tenant_id, &duplicate_request, 300_000)?;
    if original != replayed {
        return Err("coordinator restart changed an idempotent receipt".into());
    }
    let status = reopened
        .status(&tenant_id, &enterprise_id)?
        .ok_or_else(|| "coordinator lost its enterprise pin".to_string())?;
    if status.accepted_batches != request_count as u64
        || status.next_sequence != request_count as u64 + 1
    {
        return Err("coordinator durable status disagrees with accepted receipts".into());
    }
    let proof = reopened.batch_proof(
        &tenant_id,
        &enterprise_id,
        duplicate_request
            .bundle
            .signed_batch
            .batch
            .batch_id
            .as_str(),
    )?;
    if verify_proof(&proof.root, &proof.proof).map_err(|error| error.to_string())?
        != ProofStatus::Member
        || proof.root.digest.to_string() != status.batch_accumulator_root
    {
        return Err("coordinator batch accumulator proof did not verify".into());
    }
    let membership_proof_bytes = serde_json::to_vec(&proof)
        .map_err(|error| error.to_string())?
        .len();
    let coordinator_state_bytes = directory_bytes(root.path())?;
    let semantic_payload = json!({
        "enterprise_id": enterprise_id,
        "tenant_id": tenant_id,
        "anchor_manifest_id": chain.anchor_manifest_id,
        "coordinator_public_key": reopened.coordinator_public_key(),
        "accepted_batches": status.accepted_batches,
        "next_sequence": status.next_sequence,
        "batch_accumulator_root": status.batch_accumulator_root,
        "batch_accumulator_count": proof.root.count,
        "membership_proof_verified": true,
        "receipt_chain_complete": true,
        "restart_idempotent": true,
        "worker_count": worker_count,
    });
    let semantic_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&semantic_payload).map_err(|error| error.to_string())?)
    );
    Ok((
        format!(
            "{} concurrent signed uploads formed one durable receipt chain",
            status.accepted_batches
        ),
        json!({
            "enterprise_id": enterprise_id,
            "tenant_id": tenant_id,
            "anchor_manifest_id": chain.anchor_manifest_id,
            "coordinator_public_key": reopened.coordinator_public_key(),
            "accepted_batches": status.accepted_batches,
            "next_sequence": status.next_sequence,
            "batch_accumulator_root": status.batch_accumulator_root,
            "batch_accumulator_count": proof.root.count,
            "membership_proof_verified": true,
            "membership_proof_bytes": membership_proof_bytes,
            "receipt_chain_complete": true,
            "restart_idempotent": true,
            "worker_count": worker_count,
            "concurrent_ingest_ms": concurrent_ingest_ms,
            "coordinator_state_bytes": coordinator_state_bytes,
            "semantic_sha256": semantic_sha256,
        }),
    ))
}

fn verify_receipt_chain(
    receipts: &[(usize, IngestReceipt)],
    expected: usize,
    coordinator_public_key: &str,
) -> Result<(), String> {
    if receipts.len() != expected {
        return Err("central-ingestion benchmark lost a receipt".into());
    }
    let by_sequence = receipts
        .iter()
        .map(|(_, receipt)| {
            receipt.verify(coordinator_public_key)?;
            Ok((receipt.sequence, receipt))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    for sequence in 1..=expected as u64 {
        let receipt = by_sequence
            .get(&sequence)
            .ok_or_else(|| "central-ingestion receipt sequence has a gap".to_string())?;
        let expected_previous = sequence
            .checked_sub(1)
            .filter(|value| *value > 0)
            .and_then(|previous| by_sequence.get(&previous))
            .map(|previous| &previous.receipt_id);
        if receipt.previous_receipt_id.as_ref() != expected_previous {
            return Err("central-ingestion receipt chain names the wrong predecessor".into());
        }
    }
    Ok(())
}

fn directory_bytes(root: &std::path::Path) -> Result<u64, String> {
    let mut total = 0_u64;
    for entry in std::fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}
