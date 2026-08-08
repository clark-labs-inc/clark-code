use std::fs;
use std::path::Path;

use agent_orchestration::{EnterpriseId, EnterpriseSignedBatch, EnterpriseTrustChain};
use fs2::FileExt;

use super::model::{IndexedStatus, IngestOutcome, ScoutStoreRequest, ScoutStoreResponse};

mod database;
pub(crate) mod ledger_authority;
pub(super) mod materialized;

pub(crate) use database::{index_mac, verify_index_mac};

const MAX_BATCH_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BATCHES: usize = 100_000;
const MAX_TRUST_CHAIN_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn ensure_materialized_locked(
    root: &Path,
    enterprise_id: &EnterpriseId,
) -> Result<super::model::IndexReceipt, String> {
    materialized::ensure_committed_locked(root, enterprise_id)
}

pub(super) fn handle(
    root: &Path,
    request: ScoutStoreRequest,
) -> Result<ScoutStoreResponse, String> {
    ensure_real_directory(root)?;
    if let ScoutStoreRequest::Ingest {
        enterprise_id,
        envelope,
    } = request
    {
        return ingest(root, &enterprise_id, *envelope);
    }
    let enterprise_id = request.enterprise_id().clone();
    fs::create_dir_all(root).map_err(io_error)?;
    let lock = open_lock(root)?;
    lock.lock_exclusive().map_err(io_error)?;
    let result = (|| match request {
        ScoutStoreRequest::Ingest { .. } => {
            Err("Scout ingest reached the query dispatch unexpectedly".into())
        }
        ScoutStoreRequest::IssueCheckpoint { now_ms, .. } => {
            let (status, idempotent) = super::checkpoint::issue(root, &enterprise_id, now_ms)?;
            Ok(ScoutStoreResponse::CheckpointIssued { status, idempotent })
        }
        ScoutStoreRequest::CheckpointStatus { .. } => {
            let status = super::checkpoint::status(root, &enterprise_id)?;
            Ok(ScoutStoreResponse::CheckpointStatus { status })
        }
        ScoutStoreRequest::ExportCheckpoint { sequence, .. } => {
            let exchange = super::checkpoint::exchange::export(root, &enterprise_id, sequence)?;
            Ok(ScoutStoreResponse::CheckpointExported {
                exchange: Box::new(exchange),
            })
        }
        ScoutStoreRequest::ObserveCheckpoint { exchange, .. } => {
            let (status, idempotent) =
                super::checkpoint::exchange::observe(root, &enterprise_id, *exchange)?;
            Ok(ScoutStoreResponse::CheckpointObserved { status, idempotent })
        }
        ScoutStoreRequest::EnqueueOutbox { batch_id, .. } => {
            let (entry, idempotent) = super::outbox::enqueue(root, &enterprise_id, &batch_id)?;
            Ok(ScoutStoreResponse::OutboxUpdated { entry, idempotent })
        }
        ScoutStoreRequest::BeginOutboxDelivery {
            batch_id,
            attempt_id,
            previous_attempt_id,
            ..
        } => {
            let (entry, idempotent) = super::outbox::begin_delivery(
                root,
                &enterprise_id,
                &batch_id,
                &attempt_id,
                previous_attempt_id.as_deref(),
            )?;
            Ok(ScoutStoreResponse::OutboxUpdated { entry, idempotent })
        }
        ScoutStoreRequest::ResolveOutboxDelivery {
            batch_id,
            attempt_id,
            resolution,
            resolution_id,
            ..
        } => {
            let (entry, idempotent) = super::outbox::resolve_delivery(
                root,
                &enterprise_id,
                &batch_id,
                &attempt_id,
                resolution,
                &resolution_id,
            )?;
            Ok(ScoutStoreResponse::OutboxUpdated { entry, idempotent })
        }
        ScoutStoreRequest::OutboxStatus { batch_id, .. } => {
            let entry = super::outbox::status(root, &enterprise_id, &batch_id)?;
            Ok(ScoutStoreResponse::OutboxStatus { entry })
        }
        ScoutStoreRequest::ListOutbox {
            filter,
            cursor,
            limit,
            ..
        } => {
            let page = super::outbox::list(root, &enterprise_id, filter, cursor.as_deref(), limit)?;
            Ok(ScoutStoreResponse::OutboxListed { page })
        }
        indexed_request => {
            let (mut connection, receipt, auth_key) =
                materialized::ensure_locked(root, &enterprise_id)?;
            match indexed_request {
                ScoutStoreRequest::Rebuild { .. } => Ok(ScoutStoreResponse::Rebuilt(receipt)),
                ScoutStoreRequest::Status { .. } => {
                    let mut status: IndexedStatus =
                        database::read_meta_json(&connection, "status", &auth_key)?;
                    redact_supplemental_status(&mut status);
                    Ok(ScoutStoreResponse::Status {
                        status: Box::new(status),
                        receipt: redact_supplemental_receipt(receipt),
                    })
                }
                ScoutStoreRequest::Entities { query, .. } => {
                    let page = super::query::entities(
                        &mut connection,
                        &enterprise_id,
                        &receipt,
                        &auth_key,
                        query,
                    )?;
                    Ok(ScoutStoreResponse::Entities {
                        page,
                        receipt: redact_supplemental_receipt(receipt),
                    })
                }
                ScoutStoreRequest::QualifiedEntities { query, .. } => {
                    let page = super::query::qualified_entities(
                        &mut connection,
                        &enterprise_id,
                        &receipt,
                        &auth_key,
                        query,
                    )?;
                    Ok(ScoutStoreResponse::Entities {
                        page,
                        receipt: redact_supplemental_receipt(receipt),
                    })
                }
                ScoutStoreRequest::Edges { query, .. } => {
                    let page = super::query::edges(
                        &mut connection,
                        &enterprise_id,
                        &receipt,
                        &auth_key,
                        query,
                    )?;
                    Ok(ScoutStoreResponse::Edges {
                        page,
                        receipt: redact_supplemental_receipt(receipt),
                    })
                }
                ScoutStoreRequest::QualifiedEdges { query, .. } => {
                    let page = super::query::qualified_edges(
                        &mut connection,
                        &enterprise_id,
                        &receipt,
                        &auth_key,
                        query,
                    )?;
                    Ok(ScoutStoreResponse::Edges {
                        page,
                        receipt: redact_supplemental_receipt(receipt),
                    })
                }
                ScoutStoreRequest::Neighborhood {
                    seed, depth, limit, ..
                } => {
                    let page =
                        super::query::neighborhood(&connection, &auth_key, seed, depth, limit)?;
                    Ok(ScoutStoreResponse::Neighborhood {
                        page,
                        receipt: redact_supplemental_receipt(receipt),
                    })
                }
                ScoutStoreRequest::QualifiedNeighborhood { query, .. } => {
                    let page = super::query::qualified_neighborhood(&connection, &auth_key, query)?;
                    Ok(ScoutStoreResponse::Neighborhood {
                        page,
                        receipt: redact_supplemental_receipt(receipt),
                    })
                }
                ScoutStoreRequest::Batches { cursor, limit, .. } => {
                    let page = super::query::batches(
                        &mut connection,
                        &enterprise_id,
                        &receipt,
                        &auth_key,
                        cursor,
                        limit,
                    )?;
                    Ok(ScoutStoreResponse::Batches {
                        page,
                        receipt: redact_supplemental_receipt(receipt),
                    })
                }
                _ => Err("Scout control request reached indexed dispatch unexpectedly".into()),
            }
        }
    })();
    FileExt::unlock(&lock).map_err(io_error)?;
    result
}

fn redact_supplemental_receipt(
    mut receipt: super::model::IndexReceipt,
) -> super::model::IndexReceipt {
    receipt.event_set_root_v1 = None;
    receipt.projection_map_root_v2 = None;
    receipt.enterprise_snapshot_root_v2 = None;
    receipt
}

fn redact_supplemental_status(status: &mut IndexedStatus) {
    status.event_set_root_v1 = None;
    status.projection_map_root_v2 = None;
    status.enterprise_snapshot_root_v2 = None;
}

impl ScoutStoreRequest {
    fn enterprise_id(&self) -> &EnterpriseId {
        match self {
            Self::Ingest { enterprise_id, .. }
            | Self::IssueCheckpoint { enterprise_id, .. }
            | Self::CheckpointStatus { enterprise_id }
            | Self::ExportCheckpoint { enterprise_id, .. }
            | Self::ObserveCheckpoint { enterprise_id, .. }
            | Self::EnqueueOutbox { enterprise_id, .. }
            | Self::BeginOutboxDelivery { enterprise_id, .. }
            | Self::ResolveOutboxDelivery { enterprise_id, .. }
            | Self::OutboxStatus { enterprise_id, .. }
            | Self::ListOutbox { enterprise_id, .. }
            | Self::Rebuild { enterprise_id }
            | Self::Status { enterprise_id }
            | Self::Entities { enterprise_id, .. }
            | Self::QualifiedEntities { enterprise_id, .. }
            | Self::Edges { enterprise_id, .. }
            | Self::QualifiedEdges { enterprise_id, .. }
            | Self::Neighborhood { enterprise_id, .. }
            | Self::QualifiedNeighborhood { enterprise_id, .. }
            | Self::Batches { enterprise_id, .. } => enterprise_id,
        }
    }
}

fn open_lock(root: &Path) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("index.lock"))
        .map_err(io_error)
}

fn ingest(
    root: &Path,
    enterprise_id: &EnterpriseId,
    envelope: EnterpriseSignedBatch,
) -> Result<ScoutStoreResponse, String> {
    fs::create_dir_all(root).map_err(io_error)?;
    let lock = open_lock(root)?;
    lock.lock_exclusive().map_err(io_error)?;

    let (chain, _) = read_pinned_chain(root, enterprise_id)?;
    let verified = chain.verify_signed_batch(envelope)?;
    let envelope = verified.envelope().clone();
    if envelope.batch.enterprise_id != *enterprise_id {
        return Err("Scout ingest envelope belongs to another enterprise".into());
    }
    let encoded = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    if encoded.len() as u64 > MAX_BATCH_BYTES {
        return Err("Scout ingest envelope exceeds the batch byte limit".into());
    }
    let ledger = ledger_authority::open(root, enterprise_id)?;
    if ledger.head.batch_count >= MAX_BATCHES as u64
        && ledger
            .authority
            .read_envelope(&envelope.batch.batch_id)?
            .envelope
            .is_none()
    {
        return Err(format!(
            "Scout ingest refuses more than {MAX_BATCHES} immutable batches"
        ));
    }
    let append = ledger.authority.append_verified(&verified)?;
    let outcome = match append.outcome {
        crate::ledger_authority::LedgerAppendOutcome::Inserted => IngestOutcome::Inserted,
        crate::ledger_authority::LedgerAppendOutcome::AlreadyPresent => {
            IngestOutcome::AlreadyPresent
        }
    };
    let (_, mut receipt, _) = match outcome {
        IngestOutcome::Inserted => {
            materialized::ensure_after_insert_locked(root, enterprise_id, &envelope)?
        }
        IngestOutcome::AlreadyPresent => {
            let receipt = materialized::ensure_committed_locked(root, enterprise_id)?;
            let auth_key = database::load_or_create_index_auth_key(root)?;
            let connection = database::open_database(root)?;
            (connection, receipt, auth_key)
        }
    };
    receipt.ledger_authority_work.merge(ledger.work);
    receipt.ledger_authority_work.merge(append.work);
    FileExt::unlock(&lock).map_err(io_error)?;
    Ok(ScoutStoreResponse::Ingested { outcome, receipt })
}

pub(super) fn ensure_real_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(format!(
            "Scout target path is not a real directory: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(io_error)
        }
        Err(error) => Err(io_error(error)),
    }
}

pub(super) fn read_pinned_chain(
    root: &Path,
    enterprise_id: &EnterpriseId,
) -> Result<(EnterpriseTrustChain, Vec<u8>), String> {
    let trust_dir = root.join("trust");
    let private_dir = root.join("private");
    ensure_real_directory(&trust_dir)?;
    ensure_real_directory(&private_dir)?;
    let pin_bytes = read_regular_bounded(
        &private_dir.join("anchor-manifest-id"),
        256,
        "private trust anchor",
    )?;
    let pinned_anchor = std::str::from_utf8(&pin_bytes)
        .map_err(|_| "Scout private trust anchor is not UTF-8".to_string())?;
    validate_anchor_manifest_id(pinned_anchor)?;
    let chain_bytes = read_regular_bounded(
        &trust_dir.join("chain.json"),
        MAX_TRUST_CHAIN_BYTES,
        "trust chain",
    )?;
    let chain: EnterpriseTrustChain =
        serde_json::from_slice(&chain_bytes).map_err(|error| error.to_string())?;
    chain.verify(enterprise_id)?;
    if chain.anchor_manifest_id != pinned_anchor {
        return Err("Scout trust chain does not match the target-private anchor pin".into());
    }
    Ok((chain, chain_bytes))
}

pub(super) fn read_regular_bounded(
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > max_bytes
    {
        return Err(format!("Scout {label} path is unsafe or oversized"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(format!("Scout {label} must not be hard-linked"));
        }
    }
    fs::read(path).map_err(io_error)
}

fn validate_anchor_manifest_id(value: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix("trust-manifest:") else {
        return Err("Scout private trust anchor has an invalid prefix".into());
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Scout private trust anchor has an invalid digest".into());
    }
    Ok(())
}

pub(super) fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(io_error)
    }
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
}

fn io_error(error: std::io::Error) -> String {
    format!("Scout index filesystem: {error}")
}
