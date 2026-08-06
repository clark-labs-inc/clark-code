use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use agent_orchestration::{EnterpriseSignedBatch, EnterpriseSigningKey, EnterpriseTrustChain};
use scout_store::{
    dispatch as scout_store_dispatch, EntityQuery as IndexedEntityQuery, ScoutStoreRequest,
    ScoutStoreResponse, StoredCheckpointBundle, SERVICE_NAME as SCOUT_STORE_SERVICE,
};
use serde_json::json;

pub(super) struct IndexMetrics {
    pub(super) event_root: String,
    pub(super) graph_digest: String,
    pub(super) event_set_root_v1: String,
    pub(super) projection_map_root_v2: String,
    pub(super) enterprise_snapshot_root_v2: String,
    pub(super) warm_envelope_rows_read: usize,
    pub(super) page_size: usize,
    pub(super) rebuild_ms: u128,
    pub(super) warm_status_ms: u128,
    pub(super) index_bytes: u64,
    pub(super) index_page_count: u64,
    pub(super) index_freelist_pages: u64,
    pub(super) index_table_bytes: BTreeMap<String, u64>,
    pub(super) projection_rows_written: usize,
    pub(super) projection_rows_deleted: usize,
    pub(super) supplemental_rows_written: usize,
    pub(super) supplemental_rows_deleted: usize,
    pub(super) projection_total_rows: usize,
    pub(super) checkpoint_id: String,
    pub(super) checkpoint_sequence: u64,
    pub(super) checkpoint_covers_current_ledger: bool,
    pub(super) checkpoint_delta_batch_count: usize,
    pub(super) checkpoint_chain_membership_entries: usize,
    pub(super) checkpoint_chain_bytes: u64,
}

pub(super) fn index_fixture(
    chain: &EnterpriseTrustChain,
    envelopes: &[EnterpriseSignedBatch],
) -> Result<IndexMetrics, String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    std::fs::create_dir_all(temp.path().join("trust")).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(temp.path().join("private")).map_err(|error| error.to_string())?;
    std::fs::write(
        temp.path().join("trust/chain.json"),
        serde_json::to_vec(chain).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    std::fs::write(
        temp.path().join("private/anchor-manifest-id"),
        chain.anchor_manifest_id.as_bytes(),
    )
    .map_err(|error| error.to_string())?;
    let coordinator = EnterpriseSigningKey::from_seed([0x42; 32]);
    let mut bootstrap = vec![0x42; 32];
    bootstrap.extend_from_slice(&1_u64.to_le_bytes());
    std::fs::write(
        temp.path().join("private/local-signing-bootstrap"),
        bootstrap,
    )
    .map_err(|error| error.to_string())?;
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_vec(&json!({
            "schema_version": 3,
            "enterprise_id": chain.manifests[0].enterprise_id,
            "anchor_manifest_id": chain.anchor_manifest_id,
            "local_signer_id": coordinator.signer_id(),
            "mode": "coordinator"
        }))
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let split = envelopes.len().div_ceil(2);
    write_envelopes(temp.path(), &envelopes[..split])?;
    force_cold_rebuild(temp.path())?;
    let enterprise_id = chain.manifests[0].enterprise_id.clone();
    let initial = index_call(
        temp.path(),
        ScoutStoreRequest::Rebuild {
            enterprise_id: enterprise_id.clone(),
        },
    )?;
    let ScoutStoreResponse::Rebuilt(initial_receipt) = initial else {
        return Err("Scout benchmark initial index returned the wrong response".into());
    };
    if !initial_receipt.rebuilt || initial_receipt.derived_batches_read != split {
        return Err("Scout benchmark initial index did not cover the first batch half".into());
    }
    let first_checkpoint = index_call(
        temp.path(),
        ScoutStoreRequest::IssueCheckpoint {
            enterprise_id: enterprise_id.clone(),
            now_ms: 19_000,
        },
    )?;
    let ScoutStoreResponse::CheckpointIssued {
        status: first_checkpoint_status,
        idempotent: false,
    } = first_checkpoint
    else {
        return Err("Scout benchmark did not issue its first checkpoint".into());
    };
    write_envelopes(temp.path(), &envelopes[split..])?;
    force_cold_rebuild(temp.path())?;
    let started = Instant::now();
    let rebuilt = index_call(
        temp.path(),
        ScoutStoreRequest::Rebuild {
            enterprise_id: enterprise_id.clone(),
        },
    )?;
    let rebuild_ms = started.elapsed().as_millis();
    let ScoutStoreResponse::Rebuilt(rebuild_receipt) = rebuilt else {
        return Err("Scout benchmark index returned the wrong rebuild response".into());
    };
    if !rebuild_receipt.rebuilt || rebuild_receipt.derived_batches_read != envelopes.len() {
        return Err("Scout benchmark index did not authenticate every batch on rebuild".into());
    }

    let started = Instant::now();
    let status = index_call(
        temp.path(),
        ScoutStoreRequest::Status {
            enterprise_id: enterprise_id.clone(),
        },
    )?;
    let warm_status_ms = started.elapsed().as_millis();
    let ScoutStoreResponse::Status {
        status: indexed_status,
        receipt: warm_receipt,
    } = status
    else {
        return Err("Scout benchmark index returned the wrong status response".into());
    };
    if warm_receipt.rebuilt || warm_receipt.ledger_authority_work.envelope_rows_read != 0 {
        return Err("warm Scout index status reread signed batch envelopes".into());
    }
    if indexed_status.event_set_root_v1.is_some()
        || indexed_status.projection_map_root_v2.is_some()
        || indexed_status.enterprise_snapshot_root_v2.is_some()
        || warm_receipt.event_set_root_v1.is_some()
        || warm_receipt.projection_map_root_v2.is_some()
        || warm_receipt.enterprise_snapshot_root_v2.is_some()
    {
        return Err("Scout query status exposed global supplemental commitments".into());
    }
    let page = index_call(
        temp.path(),
        ScoutStoreRequest::Entities {
            enterprise_id,
            query: IndexedEntityQuery {
                limit: 100,
                ..IndexedEntityQuery::default()
            },
        },
    )?;
    let ScoutStoreResponse::Entities { page, receipt } = page else {
        return Err("Scout benchmark index returned the wrong entity response".into());
    };
    if page.entities.len() != 100
        || page.next_cursor.is_none()
        || receipt.rebuilt
        || receipt.ledger_authority_work.envelope_rows_read != 0
    {
        return Err("Scout benchmark indexed pagination is not bounded and warm".into());
    }
    let checkpoint = index_call(
        temp.path(),
        ScoutStoreRequest::IssueCheckpoint {
            enterprise_id: chain.manifests[0].enterprise_id.clone(),
            now_ms: 20_000,
        },
    )?;
    let ScoutStoreResponse::CheckpointIssued {
        status: checkpoint_status,
        idempotent: false,
    } = checkpoint
    else {
        return Err("Scout benchmark did not advance its checkpoint".into());
    };
    if !checkpoint_status.checkpoint_covers_current_ledger
        || !checkpoint_status.checkpoint_covers_current_projection
        || checkpoint_status.uncheckpointed_batch_count != 0
        || checkpoint_status.uncheckpointed_event_count != 0
    {
        return Err(
            "Scout benchmark checkpoint did not cover the current ledger and projection".into(),
        );
    }
    let commitment = checkpoint_status
        .snapshot_commitment_v2
        .as_ref()
        .ok_or_else(|| "Scout benchmark checkpoint omitted its snapshot commitment".to_string())?;
    if Some(&commitment.event_set_root_v1) != rebuild_receipt.event_set_root_v1.as_ref()
        || Some(&commitment.projection_map_root_v2)
            != rebuild_receipt.projection_map_root_v2.as_ref()
        || Some(&commitment.enterprise_snapshot_root_v2)
            != rebuild_receipt.enterprise_snapshot_root_v2.as_ref()
        || commitment.graph_digest != rebuild_receipt.graph_digest
    {
        return Err("Scout benchmark checkpoint signed different projection roots".into());
    }
    let first_path = checkpoint_path(temp.path(), first_checkpoint_status.sequence);
    let second_path = checkpoint_path(temp.path(), checkpoint_status.sequence);
    let first_bytes = std::fs::read(&first_path).map_err(|error| error.to_string())?;
    let second_bytes = std::fs::read(&second_path).map_err(|error| error.to_string())?;
    let first_bundle: StoredCheckpointBundle =
        serde_json::from_slice(&first_bytes).map_err(|error| error.to_string())?;
    let second_bundle: StoredCheckpointBundle =
        serde_json::from_slice(&second_bytes).map_err(|error| error.to_string())?;
    if !first_bundle
        .added_batch_ids
        .is_disjoint(&second_bundle.added_batch_ids)
        || first_bundle.added_batch_ids.len() + second_bundle.added_batch_ids.len()
            != envelopes.len()
    {
        return Err("Scout checkpoint deltas did not partition immutable membership".into());
    }
    let checkpoint_chain_bytes = u64::try_from(first_bytes.len() + second_bytes.len())
        .map_err(|_| "checkpoint byte count overflow".to_string())?;
    let index_path = temp.path().join("index-v4.sqlite3");
    let index_bytes = std::fs::metadata(&index_path)
        .map_err(|error| error.to_string())?
        .len();
    let (index_page_count, index_freelist_pages, index_table_bytes) = database_space(&index_path)?;
    Ok(IndexMetrics {
        event_root: indexed_status.event_root,
        graph_digest: indexed_status.graph_digest,
        event_set_root_v1: rebuild_receipt
            .event_set_root_v1
            .ok_or("Scout benchmark index omitted its event-set commitment")?,
        projection_map_root_v2: rebuild_receipt
            .projection_map_root_v2
            .ok_or("Scout benchmark index omitted its projection-map commitment")?,
        enterprise_snapshot_root_v2: rebuild_receipt
            .enterprise_snapshot_root_v2
            .ok_or("Scout benchmark index omitted its combined snapshot commitment")?,
        warm_envelope_rows_read: warm_receipt.ledger_authority_work.envelope_rows_read,
        page_size: page.entities.len(),
        rebuild_ms,
        warm_status_ms,
        index_bytes,
        index_page_count,
        index_freelist_pages,
        index_table_bytes,
        projection_rows_written: rebuild_receipt.projection_rows_written,
        projection_rows_deleted: rebuild_receipt.projection_rows_deleted,
        supplemental_rows_written: rebuild_receipt.supplemental_rows_written,
        supplemental_rows_deleted: rebuild_receipt.supplemental_rows_deleted,
        projection_total_rows: indexed_status.entities
            + indexed_status.edges
            + indexed_status.batches,
        checkpoint_id: checkpoint_status.checkpoint_id,
        checkpoint_sequence: checkpoint_status.sequence,
        checkpoint_covers_current_ledger: checkpoint_status.checkpoint_covers_current_ledger,
        checkpoint_delta_batch_count: second_bundle.added_batch_ids.len(),
        checkpoint_chain_membership_entries: first_bundle.added_batch_ids.len()
            + second_bundle.added_batch_ids.len(),
        checkpoint_chain_bytes,
    })
}

fn database_space(path: &Path) -> Result<(u64, u64, BTreeMap<String, u64>), String> {
    let connection = rusqlite::Connection::open(path).map_err(|error| error.to_string())?;
    let page_count = pragma_u64(&connection, "page_count")?;
    let freelist_pages = pragma_u64(&connection, "freelist_count")?;
    let mut statement = connection
        .prepare("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name ORDER BY name")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut table_bytes = BTreeMap::new();
    for row in rows {
        let (name, bytes) = row.map_err(|error| error.to_string())?;
        table_bytes.insert(
            name,
            u64::try_from(bytes).map_err(|_| "SQLite dbstat byte count is negative")?,
        );
    }
    Ok((page_count, freelist_pages, table_bytes))
}

fn pragma_u64(connection: &rusqlite::Connection, name: &str) -> Result<u64, String> {
    let value = connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?;
    u64::try_from(value).map_err(|_| format!("SQLite {name} is negative"))
}

fn write_envelopes(
    root: &std::path::Path,
    envelopes: &[EnterpriseSignedBatch],
) -> Result<(), String> {
    for envelope in envelopes {
        let response = index_call(
            root,
            ScoutStoreRequest::Ingest {
                enterprise_id: envelope.batch.enterprise_id.clone(),
                envelope: Box::new(envelope.clone()),
            },
        )?;
        if !matches!(response, ScoutStoreResponse::Ingested { .. }) {
            return Err("Scout benchmark seed ingest returned the wrong response".into());
        }
    }
    Ok(())
}

fn force_cold_rebuild(root: &Path) -> Result<(), String> {
    let connection = rusqlite::Connection::open(root.join("index-v4.sqlite3"))
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE meta SET value = 'enterprise-benchmark-force-cold' \
             WHERE key = 'projection_version'",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn checkpoint_path(root: &std::path::Path, sequence: u64) -> std::path::PathBuf {
    root.join("checkpoints")
        .join(format!("{sequence:020}.json"))
}

fn index_call(
    root: &std::path::Path,
    request: ScoutStoreRequest,
) -> Result<ScoutStoreResponse, String> {
    let request = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    let response = scout_store_dispatch(SCOUT_STORE_SERVICE, root, &request)?;
    serde_json::from_slice(&response).map_err(|error| error.to_string())
}
