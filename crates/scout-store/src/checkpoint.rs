use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use agent_orchestration::{
    EnterpriseBatch, EnterpriseBatchId, EnterpriseCheckpointCursor,
    EnterpriseCheckpointObservation, EnterpriseId, EnterpriseLedgerCheckpoint,
    EnterpriseLedgerCommitment, EnterpriseLedgerSummary, EnterpriseSigningKey,
    EnterpriseSnapshotCommitmentV2, EnterpriseTrustChain,
};
use serde::{Deserialize, Serialize};

use crate::index::{
    ensure_real_directory, read_pinned_chain, read_regular_bounded, sync_directory,
};
use crate::model::AuthenticatedCheckpointStatus;

pub(super) mod exchange;
mod storage;

pub(super) use storage::{
    checkpoint_file_name, parse_checkpoint_file_name, replace_private_json, write_private_new,
};

const MAX_BATCHES: usize = 100_000;
const MAX_CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;
const STORE_SCHEMA_VERSION: u16 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCheckpointBundle {
    pub checkpoint: EnterpriseLedgerCheckpoint,
    pub added_batch_ids: BTreeSet<EnterpriseBatchId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointExchangeBundle {
    pub coordinator_id: String,
    pub anchor_manifest_id: String,
    pub bundle: StoredCheckpointBundle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoreMode {
    Coordinator,
    ReplicaPending,
    Replica,
}

#[derive(Deserialize)]
struct StoreManifest {
    schema_version: u16,
    enterprise_id: EnterpriseId,
    anchor_manifest_id: String,
    local_signer_id: String,
    mode: StoreMode,
}

pub(super) fn issue(
    root: &Path,
    enterprise_id: &EnterpriseId,
    now_ms: u64,
) -> Result<(AuthenticatedCheckpointStatus, bool), String> {
    let manifest = read_store_manifest(root, enterprise_id)?;
    if manifest.mode != StoreMode::Coordinator {
        return Err("only a coordinator store can issue ledger checkpoints".into());
    }
    let (chain, _) = read_pinned_chain(root, enterprise_id)?;
    if manifest.anchor_manifest_id != chain.anchor_manifest_id {
        return Err("enterprise manifest does not match the private trust anchor".into());
    }
    let current_manifest = chain.verify(enterprise_id)?;
    let key = read_local_key(root)?;
    if key.signer_id() != manifest.local_signer_id
        || !current_manifest.coordinators.contains_key(&key.signer_id())
    {
        return Err(
            "local coordinator key does not match the current enterprise trust manifest".into(),
        );
    }
    let batches = authenticated_batches(root, enterprise_id, &chain)?;
    let current_snapshot = current_snapshot_commitment(root, enterprise_id)?;
    let current_ledger = crate::index::ledger_authority::commitment(root, enterprise_id)?;
    let (mut cursor, membership) = recover_cursor(root, enterprise_id, &chain, &batches)?;
    if let Some(bundle) = bundle_at(root, cursor.highest_sequence())? {
        let verified = verify_bundle(&chain, &batches, &bundle, &membership)?;
        if cursor.observe(&verified)? != EnterpriseCheckpointObservation::Duplicate {
            return Err(
                "durable checkpoint cursor did not reproduce its highest checkpoint".into(),
            );
        }
        if bundle_matches_batches(&bundle, &batches, &current_snapshot).is_ok() {
            return Ok((
                checkpoint_status(&bundle.checkpoint, &batches, &current_snapshot)?,
                true,
            ));
        }
    }

    let sequence = cursor
        .highest_sequence()
        .checked_add(1)
        .ok_or_else(|| "enterprise checkpoint sequence overflow".to_string())?;
    let checkpoint = EnterpriseLedgerCheckpoint::issue_v2(
        current_manifest,
        sequence,
        cursor.highest_checkpoint_id().map(str::to_owned),
        now_ms,
        current_ledger,
        Some(current_snapshot.clone()),
        &[&key],
    )?;
    let current_batch_ids = batches
        .iter()
        .map(|batch| batch.batch_id.clone())
        .collect::<BTreeSet<_>>();
    if !membership.is_subset(&current_batch_ids) {
        return Err("authenticated checkpoint membership references a deleted batch".into());
    }
    let bundle = StoredCheckpointBundle {
        checkpoint,
        added_batch_ids: current_batch_ids.difference(&membership).cloned().collect(),
    };
    let mut next_membership = membership;
    apply_membership_delta(&mut next_membership, &bundle)?;
    let verified = verify_bundle(&chain, &batches, &bundle, &next_membership)?;
    publish_bundle_create_only(root, &bundle)?;
    if cursor.observe(&verified)? != EnterpriseCheckpointObservation::Advanced {
        return Err("new checkpoint did not advance the durable cursor".into());
    }
    publish_cursor(root, &cursor)?;
    Ok((
        checkpoint_status(&bundle.checkpoint, &batches, &current_snapshot)?,
        false,
    ))
}

pub(super) fn status(
    root: &Path,
    enterprise_id: &EnterpriseId,
) -> Result<Option<AuthenticatedCheckpointStatus>, String> {
    let (chain, _) = read_pinned_chain(root, enterprise_id)?;
    let batches = authenticated_batches(root, enterprise_id, &chain)?;
    let current_snapshot = current_snapshot_commitment(root, enterprise_id)?;
    let (mut cursor, membership) = recover_cursor(root, enterprise_id, &chain, &batches)?;
    if cursor.highest_sequence() == 0 {
        return Ok(None);
    }
    let bundle = bundle_at(root, cursor.highest_sequence())?
        .ok_or_else(|| "durable checkpoint cursor references a missing bundle".to_string())?;
    let verified = verify_bundle(&chain, &batches, &bundle, &membership)?;
    if cursor.observe(&verified)? != EnterpriseCheckpointObservation::Duplicate {
        return Err("durable checkpoint cursor did not reproduce its highest checkpoint".into());
    }
    Ok(Some(checkpoint_status(
        &bundle.checkpoint,
        &batches,
        &current_snapshot,
    )?))
}

fn recover_cursor(
    root: &Path,
    enterprise_id: &EnterpriseId,
    chain: &EnterpriseTrustChain,
    batches: &[EnterpriseBatch],
) -> Result<(EnterpriseCheckpointCursor, BTreeSet<EnterpriseBatchId>), String> {
    let persisted = read_cursor(root, enterprise_id)?;
    let mut cursor = EnterpriseCheckpointCursor::new(enterprise_id.clone());
    let mut membership = BTreeSet::new();
    let mut sequence = 1_u64;
    loop {
        let Some(bundle) = bundle_at(root, sequence)? else {
            if sequence <= persisted.highest_sequence() {
                return Err("durable checkpoint cursor references a missing bundle".into());
            }
            reject_sequence_gap(root, sequence)?;
            break;
        };
        apply_membership_delta(&mut membership, &bundle)?;
        let verified = verify_bundle(chain, batches, &bundle, &membership)?;
        if cursor.observe(&verified)? != EnterpriseCheckpointObservation::Advanced {
            return Err("recovered checkpoint did not advance the durable cursor".into());
        }
        if sequence == persisted.highest_sequence() && cursor != persisted {
            return Err("persisted checkpoint cursor does not match authenticated replay".into());
        }
        if sequence > persisted.highest_sequence() {
            publish_cursor(root, &cursor)?;
        }
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| "enterprise checkpoint sequence overflow".to_string())?;
    }
    Ok((cursor, membership))
}

fn read_cursor(
    root: &Path,
    enterprise_id: &EnterpriseId,
) -> Result<EnterpriseCheckpointCursor, String> {
    let path = root.join("private/checkpoint-cursor.json");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(EnterpriseCheckpointCursor::new(enterprise_id.clone()))
        }
        Err(error) => Err(error.to_string()),
        Ok(_) => {
            let bytes = read_regular_bounded(&path, 1024 * 1024, "checkpoint cursor")?;
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid durable checkpoint cursor: {error}"))
        }
    }
}

fn publish_cursor(root: &Path, cursor: &EnterpriseCheckpointCursor) -> Result<(), String> {
    let directory = root.join("private");
    ensure_real_directory(&directory)?;
    replace_private_json(&directory, "checkpoint-cursor.json", cursor)
}

fn bundle_at(root: &Path, sequence: u64) -> Result<Option<StoredCheckpointBundle>, String> {
    if sequence == 0 {
        return Ok(None);
    }
    let directory = root.join("checkpoints");
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("enterprise checkpoint path must be a real directory".into())
        }
        Ok(_) => {}
    }
    let path = directory.join(checkpoint_file_name(sequence));
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
        Ok(_) => {
            let bytes = read_regular_bounded(&path, MAX_CHECKPOINT_BYTES, "checkpoint bundle")?;
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| format!("invalid enterprise checkpoint bundle: {error}"))
        }
    }
}

fn publish_bundle_create_only(root: &Path, bundle: &StoredCheckpointBundle) -> Result<(), String> {
    let directory = root.join("checkpoints");
    ensure_real_directory(&directory)?;
    let path = directory.join(checkpoint_file_name(bundle.checkpoint.sequence));
    let bytes = serde_json::to_vec(bundle).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
        return Err("enterprise checkpoint bundle exceeds the storage limit".into());
    }
    let temporary = directory.join(format!(
        ".checkpoint-{}-{}.pending",
        bundle.checkpoint.sequence,
        std::process::id()
    ));
    write_private_new(&temporary, &bytes)?;
    match fs::hard_link(&temporary, &path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary);
            let observed = bundle_at(root, bundle.checkpoint.sequence)?
                .ok_or_else(|| "checkpoint publication raced with deletion".to_string())?;
            return (observed == *bundle)
                .then_some(())
                .ok_or_else(|| "checkpoint sequence already has different content".to_string());
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error.to_string());
        }
    }
    fs::remove_file(&temporary).map_err(|error| error.to_string())?;
    sync_directory(&directory)?;
    Ok(())
}

fn reject_sequence_gap(root: &Path, expected: u64) -> Result<(), String> {
    let directory = root.join("checkpoints");
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let name = entry
            .map_err(|error| error.to_string())?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if let Some(sequence) = parse_checkpoint_file_name(&name) {
            if sequence > expected {
                return Err("enterprise checkpoint sequence gap detected".into());
            }
        }
    }
    Ok(())
}

fn authenticated_batches(
    root: &Path,
    enterprise_id: &EnterpriseId,
    chain: &EnterpriseTrustChain,
) -> Result<Vec<EnterpriseBatch>, String> {
    let ledger = crate::index::ledger_authority::open(root, enterprise_id)?;
    let envelopes = ledger.authority.read_all_envelopes()?.envelopes;
    if envelopes.len() > MAX_BATCHES {
        return Err("enterprise store exceeds the authenticated batch limit".into());
    }
    envelopes
        .into_iter()
        .map(|generation| {
            let verified = chain.verify_signed_batch(generation.envelope)?;
            if &verified.batch().enterprise_id != enterprise_id {
                return Err("stored batch belongs to another enterprise".into());
            }
            Ok(verified.batch().clone())
        })
        .collect()
}

pub(super) fn apply_membership_delta(
    membership: &mut BTreeSet<EnterpriseBatchId>,
    bundle: &StoredCheckpointBundle,
) -> Result<(), String> {
    for batch_id in &bundle.added_batch_ids {
        if !membership.insert(batch_id.clone()) {
            return Err("checkpoint membership delta repeats an existing batch".into());
        }
    }
    if bundle.checkpoint.batch_count != membership.len() as u64 {
        return Err("checkpoint membership delta count is inconsistent".into());
    }
    Ok(())
}

fn verify_bundle(
    chain: &EnterpriseTrustChain,
    authenticated_batches: &[EnterpriseBatch],
    bundle: &StoredCheckpointBundle,
    membership: &BTreeSet<EnterpriseBatchId>,
) -> Result<agent_orchestration::VerifiedEnterpriseCheckpoint, String> {
    let verified = chain.verify_ledger_checkpoint(bundle.checkpoint.clone())?;
    if bundle.checkpoint.batch_count != membership.len() as u64 {
        return Err("checkpoint bundle membership count is inconsistent".into());
    }
    let batches = authenticated_batches
        .iter()
        .map(|batch| (batch.batch_id.clone(), batch))
        .collect::<BTreeMap<_, _>>();
    let included = membership
        .iter()
        .map(|batch_id| {
            batches
                .get(batch_id)
                .copied()
                .ok_or_else(|| "authenticated checkpoint references a deleted batch".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    verified.verify_batches(included)?;
    Ok(verified)
}

fn bundle_matches_batches(
    bundle: &StoredCheckpointBundle,
    batches: &[EnterpriseBatch],
    current_snapshot: &EnterpriseSnapshotCommitmentV2,
) -> Result<(), String> {
    if !checkpoint_covers_batches(&bundle.checkpoint, batches)? {
        return Err("authenticated checkpoint does not match the current ledger".into());
    }
    if bundle.checkpoint.snapshot_commitment_v2.as_ref() != Some(current_snapshot) {
        return Err("authenticated checkpoint does not match the current projection".into());
    }
    Ok(())
}

fn checkpoint_status(
    checkpoint: &EnterpriseLedgerCheckpoint,
    batches: &[EnterpriseBatch],
    current_snapshot: &EnterpriseSnapshotCommitmentV2,
) -> Result<AuthenticatedCheckpointStatus, String> {
    let current = EnterpriseLedgerSummary::from_batches(checkpoint.enterprise_id.clone(), batches)?;
    let covers = checkpoint_covers_batches(checkpoint, batches)?;
    Ok(AuthenticatedCheckpointStatus {
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        manifest_id: checkpoint.manifest_id.clone(),
        sequence: checkpoint.sequence,
        issued_at_ms: checkpoint.issued_at_ms,
        batch_root: checkpoint.batch_root.clone(),
        event_root: checkpoint.event_root.clone(),
        batch_count: checkpoint.batch_count,
        event_count: checkpoint.event_count,
        snapshot_commitment: checkpoint.snapshot_commitment.clone(),
        snapshot_commitment_v2: checkpoint.snapshot_commitment_v2.clone(),
        checkpoint_covers_current_ledger: covers,
        checkpoint_covers_current_projection: checkpoint.snapshot_commitment_v2.as_ref()
            == Some(current_snapshot),
        uncheckpointed_batch_count: current.batch_count.saturating_sub(checkpoint.batch_count),
        uncheckpointed_event_count: current.event_count.saturating_sub(checkpoint.event_count),
    })
}

fn checkpoint_covers_batches(
    checkpoint: &EnterpriseLedgerCheckpoint,
    batches: &[EnterpriseBatch],
) -> Result<bool, String> {
    if let Some(expected) = &checkpoint.ledger_commitment {
        let observed = EnterpriseLedgerCommitment::from_batches(
            &checkpoint.enterprise_id,
            expected.generation,
            batches,
        )?;
        return Ok(&observed == expected);
    }
    let current = EnterpriseLedgerSummary::from_batches(checkpoint.enterprise_id.clone(), batches)?;
    Ok(current.batch_root == checkpoint.batch_root
        && current.event_root == checkpoint.event_root
        && current.batch_count == checkpoint.batch_count
        && current.event_count == checkpoint.event_count)
}

fn current_snapshot_commitment(
    root: &Path,
    enterprise_id: &EnterpriseId,
) -> Result<EnterpriseSnapshotCommitmentV2, String> {
    let receipt = crate::index::ensure_materialized_locked(root, enterprise_id)?;
    EnterpriseSnapshotCommitmentV2::new(
        enterprise_id,
        receipt.graph_digest,
        receipt
            .event_set_root_v1
            .ok_or_else(|| "Scout index omitted its event-set commitment".to_string())?,
        receipt
            .projection_map_root_v2
            .ok_or_else(|| "Scout index omitted its projection-map commitment".to_string())?,
    )
}

fn read_store_manifest(root: &Path, enterprise_id: &EnterpriseId) -> Result<StoreManifest, String> {
    let bytes = read_regular_bounded(
        &root.join("manifest.json"),
        1024 * 1024,
        "enterprise manifest",
    )?;
    let manifest: StoreManifest =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if manifest.schema_version != STORE_SCHEMA_VERSION || manifest.enterprise_id != *enterprise_id {
        return Err("enterprise manifest does not match this store".into());
    }
    Ok(manifest)
}

fn read_local_key(root: &Path) -> Result<EnterpriseSigningKey, String> {
    let bytes = read_regular_bounded(
        &root.join("private/local-signing-bootstrap"),
        40,
        "local signing bootstrap",
    )?;
    if bytes.len() != 40 {
        return Err("local enterprise signing bootstrap has an invalid length".into());
    }
    let seed: [u8; 32] = bytes[..32]
        .try_into()
        .map_err(|_| "local enterprise signing seed is invalid".to_string())?;
    Ok(EnterpriseSigningKey::from_seed(seed))
}
