use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use agent_orchestration::{
    EnterpriseBatchId, EnterpriseCheckpointCursor, EnterpriseCheckpointObservation, EnterpriseId,
};
use serde::{Deserialize, Serialize};

use super::{
    apply_membership_delta, authenticated_batches, bundle_at, checkpoint_file_name,
    read_store_manifest, replace_private_json, verify_bundle, write_private_new,
    CheckpointExchangeBundle, StoreMode, StoredCheckpointBundle, MAX_CHECKPOINT_BYTES,
};
use crate::index::{
    ensure_real_directory, read_pinned_chain, read_regular_bounded, sync_directory,
};
use crate::model::ObservedCheckpointStatus;

const MAX_CURSOR_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedCursor {
    coordinator_id: String,
    anchor_manifest_id: String,
    cursor: EnterpriseCheckpointCursor,
}

pub(crate) fn export(
    root: &Path,
    enterprise_id: &EnterpriseId,
    sequence: u64,
) -> Result<CheckpointExchangeBundle, String> {
    if sequence == 0 {
        return Err("enterprise checkpoint export sequence must be positive".into());
    }
    let manifest = read_store_manifest(root, enterprise_id)?;
    if manifest.mode != StoreMode::Coordinator {
        return Err("only a coordinator store can export issued checkpoints".into());
    }
    let (chain, _) = read_pinned_chain(root, enterprise_id)?;
    if manifest.anchor_manifest_id != chain.anchor_manifest_id {
        return Err("enterprise manifest does not match the private trust anchor".into());
    }
    let batches = authenticated_batches(root, enterprise_id, &chain)?;
    let (bundle, _) = replay_coordinator_sequence(root, enterprise_id, &chain, &batches, sequence)?;
    require_coordinator_approval(&manifest.local_signer_id, &bundle)?;
    Ok(CheckpointExchangeBundle {
        coordinator_id: manifest.local_signer_id,
        anchor_manifest_id: chain.anchor_manifest_id,
        bundle,
    })
}

pub(crate) fn observe(
    root: &Path,
    enterprise_id: &EnterpriseId,
    exchange: CheckpointExchangeBundle,
) -> Result<(ObservedCheckpointStatus, bool), String> {
    let coordinator_digest = coordinator_digest(&exchange.coordinator_id)?;
    let manifest = read_store_manifest(root, enterprise_id)?;
    let (chain, _) = read_pinned_chain(root, enterprise_id)?;
    if manifest.anchor_manifest_id != chain.anchor_manifest_id
        || exchange.anchor_manifest_id != chain.anchor_manifest_id
    {
        return Err("observed checkpoint does not match the target-private trust anchor".into());
    }
    require_coordinator_approval(&exchange.coordinator_id, &exchange.bundle)?;
    let batches = authenticated_batches(root, enterprise_id, &chain)?;

    let directory = observed_directory(root, coordinator_digest)?;
    let (mut observed, membership) = recover_cursor(
        &directory,
        enterprise_id,
        &chain,
        &batches,
        &exchange.coordinator_id,
    )?;
    if exchange.bundle.checkpoint.sequence < observed.cursor.highest_sequence() {
        return Err("ledger checkpoint rollback detected".into());
    }
    if exchange.bundle.checkpoint.sequence == observed.cursor.highest_sequence() {
        let stored = observed_bundle_at(&directory, exchange.bundle.checkpoint.sequence)?
            .ok_or_else(|| "observed checkpoint cursor references a missing bundle".to_string())?;
        if stored != exchange.bundle {
            return Err("conflicting ledger checkpoints share one coordinator sequence".into());
        }
    }
    let verified_header = chain.verify_ledger_checkpoint(exchange.bundle.checkpoint.clone())?;
    let mut candidate_cursor = observed.cursor.clone();
    let observation = candidate_cursor.observe(&verified_header)?;
    let mut next_membership = membership;
    if exchange.bundle.checkpoint.sequence > observed.cursor.highest_sequence() {
        apply_membership_delta(&mut next_membership, &exchange.bundle)?;
    }
    verify_bundle(&chain, &batches, &exchange.bundle, &next_membership)?;
    observed.cursor = candidate_cursor;
    let idempotent = observation == EnterpriseCheckpointObservation::Duplicate;
    if idempotent {
        let stored = observed_bundle_at(&directory, exchange.bundle.checkpoint.sequence)?
            .ok_or_else(|| "observed checkpoint cursor references a missing bundle".to_string())?;
        if stored != exchange.bundle {
            return Err("conflicting ledger checkpoints share one coordinator sequence".into());
        }
    } else {
        publish_observed_bundle_create_only(&directory, &exchange.bundle)?;
        publish_observed_cursor(&directory, &observed)?;
    }
    Ok((
        observed_status(
            &exchange.coordinator_id,
            &exchange.anchor_manifest_id,
            &exchange.bundle,
        ),
        idempotent,
    ))
}

fn recover_cursor(
    directory: &Path,
    enterprise_id: &EnterpriseId,
    chain: &agent_orchestration::EnterpriseTrustChain,
    batches: &[agent_orchestration::EnterpriseBatch],
    coordinator_id: &str,
) -> Result<(ObservedCursor, BTreeSet<EnterpriseBatchId>), String> {
    let persisted = read_observed_cursor(directory, enterprise_id, chain, coordinator_id)?;
    let mut observed = ObservedCursor {
        coordinator_id: coordinator_id.to_owned(),
        anchor_manifest_id: chain.anchor_manifest_id.clone(),
        cursor: EnterpriseCheckpointCursor::new(enterprise_id.clone()),
    };
    let mut membership = BTreeSet::new();
    let mut sequence = 1_u64;
    loop {
        let Some(bundle) = observed_bundle_at(directory, sequence)? else {
            if sequence <= persisted.cursor.highest_sequence() {
                return Err("observed checkpoint cursor references a missing bundle".into());
            }
            reject_observed_gap(directory, sequence)?;
            break;
        };
        require_coordinator_approval(coordinator_id, &bundle)?;
        apply_membership_delta(&mut membership, &bundle)?;
        let verified = verify_bundle(chain, batches, &bundle, &membership)?;
        if observed.cursor.observe(&verified)? != EnterpriseCheckpointObservation::Advanced {
            return Err("recovered observed checkpoint did not advance the durable cursor".into());
        }
        if sequence == persisted.cursor.highest_sequence() && observed != persisted {
            return Err(
                "persisted observed checkpoint cursor does not match authenticated replay".into(),
            );
        }
        if sequence > persisted.cursor.highest_sequence() {
            publish_observed_cursor(directory, &observed)?;
        }
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| "observed checkpoint sequence overflow".to_string())?;
    }
    Ok((observed, membership))
}

fn replay_coordinator_sequence(
    root: &Path,
    enterprise_id: &EnterpriseId,
    chain: &agent_orchestration::EnterpriseTrustChain,
    batches: &[agent_orchestration::EnterpriseBatch],
    target_sequence: u64,
) -> Result<(StoredCheckpointBundle, BTreeSet<EnterpriseBatchId>), String> {
    let mut cursor = EnterpriseCheckpointCursor::new(enterprise_id.clone());
    let mut membership = BTreeSet::new();
    let mut target = None;
    for sequence in 1..=target_sequence {
        let bundle = bundle_at(root, sequence)?
            .ok_or_else(|| format!("enterprise checkpoint sequence {sequence} does not exist"))?;
        apply_membership_delta(&mut membership, &bundle)?;
        let verified = verify_bundle(chain, batches, &bundle, &membership)?;
        if cursor.observe(&verified)? != EnterpriseCheckpointObservation::Advanced {
            return Err("exported checkpoint chain did not advance monotonically".into());
        }
        target = Some(bundle);
    }
    target
        .map(|bundle| (bundle, membership))
        .ok_or_else(|| "enterprise checkpoint export sequence must be positive".into())
}

fn read_observed_cursor(
    directory: &Path,
    enterprise_id: &EnterpriseId,
    chain: &agent_orchestration::EnterpriseTrustChain,
    coordinator_id: &str,
) -> Result<ObservedCursor, String> {
    let path = directory.join("cursor.json");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ObservedCursor {
            coordinator_id: coordinator_id.to_owned(),
            anchor_manifest_id: chain.anchor_manifest_id.clone(),
            cursor: EnterpriseCheckpointCursor::new(enterprise_id.clone()),
        }),
        Err(error) => Err(error.to_string()),
        Ok(_) => {
            let bytes =
                read_regular_bounded(&path, MAX_CURSOR_BYTES, "observed checkpoint cursor")?;
            let observed: ObservedCursor = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid observed checkpoint cursor: {error}"))?;
            if observed.coordinator_id != coordinator_id
                || observed.anchor_manifest_id != chain.anchor_manifest_id
            {
                return Err(
                    "observed checkpoint cursor does not match its coordinator or anchor".into(),
                );
            }
            Ok(observed)
        }
    }
}

fn publish_observed_cursor(directory: &Path, cursor: &ObservedCursor) -> Result<(), String> {
    replace_private_json(directory, "cursor.json", cursor)
}

fn observed_directory(root: &Path, coordinator_digest: &str) -> Result<PathBuf, String> {
    let private = root.join("private");
    ensure_real_directory(&private)?;
    let observed = private.join("observed-checkpoints");
    ensure_private_directory(&observed)?;
    let coordinator = observed.join(coordinator_digest);
    ensure_private_directory(&coordinator)?;
    Ok(coordinator)
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err("observed checkpoint path must be a real directory".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(not(unix))]
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path).map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn observed_bundle_at(
    directory: &Path,
    sequence: u64,
) -> Result<Option<StoredCheckpointBundle>, String> {
    if sequence == 0 {
        return Ok(None);
    }
    let path = directory.join(checkpoint_file_name(sequence));
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
        Ok(_) => {
            let bytes =
                read_regular_bounded(&path, MAX_CHECKPOINT_BYTES, "observed checkpoint bundle")?;
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| format!("invalid observed checkpoint bundle: {error}"))
        }
    }
}

fn publish_observed_bundle_create_only(
    directory: &Path,
    bundle: &StoredCheckpointBundle,
) -> Result<(), String> {
    let path = directory.join(checkpoint_file_name(bundle.checkpoint.sequence));
    let bytes = serde_json::to_vec(bundle).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
        return Err("observed checkpoint bundle exceeds the storage limit".into());
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
            let observed = observed_bundle_at(directory, bundle.checkpoint.sequence)?
                .ok_or_else(|| "observed checkpoint publication raced with deletion".to_string())?;
            if observed != *bundle {
                return Err(
                    "checkpoint coordinator sequence already has different content".to_string(),
                );
            }
            return sync_directory(directory);
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error.to_string());
        }
    }
    fs::remove_file(&temporary).map_err(|error| error.to_string())?;
    sync_directory(directory)
}

fn reject_observed_gap(directory: &Path, expected: u64) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let name = entry
            .map_err(|error| error.to_string())?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if let Some(sequence) = super::parse_checkpoint_file_name(&name) {
            if sequence > expected {
                return Err("observed checkpoint sequence gap detected".into());
            }
        }
    }
    Ok(())
}

fn require_coordinator_approval(
    coordinator_id: &str,
    bundle: &StoredCheckpointBundle,
) -> Result<(), String> {
    if !bundle.checkpoint.approvals.contains_key(coordinator_id) {
        return Err("checkpoint was not approved by its declared coordinator".into());
    }
    Ok(())
}

fn coordinator_digest(coordinator_id: &str) -> Result<&str, String> {
    let digest = coordinator_id
        .strip_prefix("signer:")
        .ok_or_else(|| "checkpoint coordinator id has an invalid prefix".to_string())?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("checkpoint coordinator id has an invalid digest".into());
    }
    Ok(digest)
}

fn observed_status(
    coordinator_id: &str,
    anchor_manifest_id: &str,
    bundle: &StoredCheckpointBundle,
) -> ObservedCheckpointStatus {
    let checkpoint = &bundle.checkpoint;
    ObservedCheckpointStatus {
        coordinator_id: coordinator_id.to_owned(),
        anchor_manifest_id: anchor_manifest_id.to_owned(),
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        sequence: checkpoint.sequence,
        manifest_id: checkpoint.manifest_id.clone(),
        issued_at_ms: checkpoint.issued_at_ms,
        batch_root: checkpoint.batch_root.clone(),
        event_root: checkpoint.event_root.clone(),
        batch_count: checkpoint.batch_count,
        event_count: checkpoint.event_count,
        snapshot_commitment: checkpoint.snapshot_commitment.clone(),
        snapshot_commitment_v2: checkpoint.snapshot_commitment_v2.clone(),
    }
}
