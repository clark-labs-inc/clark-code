use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use agent_orchestration::{
    EnterpriseEventId, EnterpriseGraph, EnterpriseId, EnterpriseSignedBatch,
};
use rusqlite::Connection;

use super::database::{
    load_or_create_index_auth_key, open_database, read_meta, read_meta_json, sql_error, write_meta,
    write_meta_json, INDEX_AUTH_KEY_BYTES,
};
use super::ledger_authority::{OpenLedger, ProjectionLedgerCursor};
use super::read_pinned_chain;
use crate::model::{IndexReceipt, IndexedStatus};

mod auxiliary;
mod commitments;
mod conflicts;
mod events;
mod history;
mod incremental;
mod ledger;
mod rebuild_support;
mod rows;
mod state;
mod storage_seal;

pub(crate) const PROJECTION_VERSION: u16 = 19;

pub(super) fn ensure_locked(
    root: &Path,
    enterprise_id: &EnterpriseId,
) -> Result<(Connection, IndexReceipt, [u8; INDEX_AUTH_KEY_BYTES]), String> {
    let ledger = super::ledger_authority::open(root, enterprise_id)?;
    let cursor = ProjectionLedgerCursor::from_head(&ledger.head);
    let auth_key = load_or_create_index_auth_key(root)?;
    let storage_is_sealed = storage_seal::validate(root, &auth_key)?;
    let mut connection = open_database(root)?;
    let current_head = read_meta(&connection, "ledger_head_id", &auth_key).ok();
    let current_enterprise = read_meta(&connection, "enterprise_id", &auth_key).ok();
    let current_version = read_meta(&connection, "projection_version", &auth_key).ok();
    let stale = current_head.as_deref() != Some(cursor.head_id.as_str())
        || current_enterprise.as_deref() != Some(enterprise_id.as_str())
        || current_version.as_deref() != Some(&PROJECTION_VERSION.to_string());
    let mut receipt = if stale {
        catch_up_or_rebuild(
            root,
            enterprise_id,
            &ledger,
            &cursor,
            &auth_key,
            &mut connection,
        )?
    } else {
        match warm_receipt(&connection, enterprise_id, &auth_key) {
            Ok(receipt) => receipt,
            Err(_) => rebuild(
                root,
                enterprise_id,
                &ledger,
                &cursor,
                &auth_key,
                &mut connection,
            )?,
        }
    };
    attach_ledger_work(&mut receipt, &ledger);
    if !storage_is_sealed && !receipt.rebuilt {
        receipt.event_set_root_v1 = None;
        receipt.projection_map_root_v2 = None;
        receipt.enterprise_snapshot_root_v2 = None;
    }
    Ok((connection, receipt, auth_key))
}

pub(super) fn ensure_committed_locked(
    root: &Path,
    enterprise_id: &EnterpriseId,
) -> Result<IndexReceipt, String> {
    let (mut connection, _receipt, auth_key) = ensure_locked(root, enterprise_id)?;
    let commitment_state = commitments::read(&connection, enterprise_id, &auth_key)?;
    if commitments::validate_storage(&connection, enterprise_id, &commitment_state, &auth_key)
        .is_ok()
    {
        storage_seal::write(root, &auth_key)?;
        return warm_receipt(&connection, enterprise_id, &auth_key);
    }
    let ledger = super::ledger_authority::open(root, enterprise_id)?;
    let cursor = ProjectionLedgerCursor::from_head(&ledger.head);
    let mut rebuilt = rebuild(
        root,
        enterprise_id,
        &ledger,
        &cursor,
        &auth_key,
        &mut connection,
    )?;
    attach_ledger_work(&mut rebuilt, &ledger);
    let commitment_state = commitments::read(&connection, enterprise_id, &auth_key)?;
    commitments::validate_storage(&connection, enterprise_id, &commitment_state, &auth_key)?;
    Ok(rebuilt)
}

fn warm_receipt(
    connection: &Connection,
    enterprise_id: &EnterpriseId,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<IndexReceipt, String> {
    let mut receipt: IndexReceipt = read_meta_json(connection, "receipt", auth_key)?;
    let status: IndexedStatus = read_meta_json(connection, "status", auth_key)?;
    let commitment_state = commitments::read(connection, enterprise_id, auth_key)?;
    if status.enterprise_id != *enterprise_id
        || receipt.event_root != status.event_root
        || receipt.graph_digest != status.graph_digest
        || receipt.event_set_root_v1.as_deref() != Some(commitment_state.event_root_id().as_str())
        || receipt.projection_map_root_v2.as_deref()
            != Some(commitment_state.projection_root_id().as_str())
        || receipt.enterprise_snapshot_root_v2.as_deref()
            != Some(
                commitment_state
                    .snapshot_root_id(enterprise_id, &status.graph_digest)?
                    .as_str(),
            )
    {
        return Err("Scout warm supplemental commitment metadata is inconsistent".into());
    }
    receipt.rebuilt = false;
    receipt.derived_batches_read = 0;
    receipt.events_replayed = 0;
    receipt.event_ids_scanned = 0;
    receipt.entity_rows_read = 0;
    receipt.edge_rows_read = 0;
    receipt.history_rows_read = 0;
    receipt.auxiliary_rows_read = 0;
    receipt.conflict_rows_read = 0;
    receipt.conflict_rows_written = 0;
    receipt.conflict_rows_deleted = 0;
    receipt.incident_edges_reclassified = 0;
    receipt.affected_projection_rows = 0;
    receipt.full_projection_fallback = false;
    receipt.projection_rows_written = 0;
    receipt.projection_rows_deleted = 0;
    receipt.supplemental_rows_written = 0;
    receipt.supplemental_rows_deleted = 0;
    receipt.ledger_authority_work = Default::default();
    Ok(receipt)
}

fn catch_up_or_rebuild(
    root: &Path,
    enterprise_id: &EnterpriseId,
    ledger: &OpenLedger,
    cursor: &ProjectionLedgerCursor,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    connection: &mut Connection,
) -> Result<IndexReceipt, String> {
    let previous = read_meta_json::<ProjectionLedgerCursor>(connection, "ledger_cursor", auth_key);
    if let Ok(previous) = previous {
        if cursor.is_direct_successor_of(&previous) {
            let mut range = ledger
                .authority
                .read_envelope_range(cursor.generation, cursor.generation)?;
            if range.envelopes.len() != 1 {
                return Err("Scout exact ledger successor payload is unavailable".into());
            }
            let envelope = range
                .envelopes
                .pop()
                .ok_or_else(|| "Scout exact ledger successor payload is unavailable".to_string())?
                .envelope;
            let (chain, _) = read_pinned_chain(root, enterprise_id)?;
            let envelope = chain.verify_signed_batch(envelope)?.into_envelope();
            if let Ok(mut receipt) =
                incremental::append(connection, enterprise_id, &envelope, cursor, auth_key)
            {
                receipt.ledger_authority_work.merge(range.work);
                storage_seal::write(root, auth_key)?;
                return Ok(receipt);
            }
        }
    }
    rebuild(root, enterprise_id, ledger, cursor, auth_key, connection)
}

fn attach_ledger_work(receipt: &mut IndexReceipt, ledger: &OpenLedger) {
    receipt.ledger_authority_work.merge(ledger.work);
}

fn usize_from_u64(value: u64, label: &str) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| format!("Scout {label} does not fit this platform"))
}

fn rebuild(
    root: &Path,
    enterprise_id: &EnterpriseId,
    ledger: &OpenLedger,
    cursor: &ProjectionLedgerCursor,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    connection: &mut Connection,
) -> Result<IndexReceipt, String> {
    let (chain, _) = read_pinned_chain(root, enterprise_id)?;
    let authority_rows = ledger.authority.read_all_envelopes()?;
    let mut graph = EnterpriseGraph::new(enterprise_id.clone());
    let mut indexed_batches = BTreeMap::<String, usize>::new();
    let mut event_batches = BTreeMap::<EnterpriseEventId, String>::new();
    for item in authority_rows.envelopes {
        let batch = chain
            .verify_signed_batch(item.envelope)?
            .into_envelope()
            .batch;
        let batch_id = batch.batch_id.to_string();
        indexed_batches.insert(batch_id.clone(), batch.events.len());
        for event in &batch.events {
            event_batches.insert(event.event_id.clone(), batch_id.clone());
        }
        graph.apply_batch(batch)?;
    }
    let snapshot = graph.snapshot()?;
    if snapshot.event_count != usize_from_u64(cursor.event_count, "ledger event count")?
        || indexed_batches.len() != usize_from_u64(cursor.batch_count, "ledger batch count")?
    {
        return Err("Scout authenticated ledger counts do not match its verified payloads".into());
    }
    let projection_state = state::ProjectionState::from_snapshot(&snapshot);
    let mut status = rebuild_support::status_from_snapshot(
        enterprise_id,
        usize_from_u64(cursor.batch_count, "ledger batch count")?,
        &snapshot,
    );
    let mut receipt = IndexReceipt {
        event_root: snapshot.event_root.clone(),
        graph_digest: snapshot.graph_digest.clone(),
        event_set_root_v1: None,
        projection_map_root_v2: None,
        enterprise_snapshot_root_v2: None,
        batch_set_root: cursor.batch_set_root_v1.clone(),
        ledger_authority_work: authority_rows.work,
        rebuilt: true,
        derived_batches_read: indexed_batches.len(),
        events_replayed: snapshot.event_count,
        event_ids_scanned: snapshot.event_count,
        entity_rows_read: 0,
        edge_rows_read: 0,
        history_rows_read: 0,
        auxiliary_rows_read: 0,
        conflict_rows_read: 0,
        conflict_rows_written: 0,
        conflict_rows_deleted: 0,
        incident_edges_reclassified: 0,
        affected_projection_rows: snapshot.entities.len()
            + snapshot.edges.len()
            + snapshot.coverage.len()
            + snapshot.frontier.len()
            + snapshot.simulation_contracts.len()
            + snapshot.conflicts.len(),
        full_projection_fallback: true,
        projection_rows_written: 0,
        projection_rows_deleted: 0,
        supplemental_rows_written: 0,
        supplemental_rows_deleted: 0,
    };
    let transaction = connection.transaction().map_err(sql_error)?;
    transaction
        .execute_batch("DELETE FROM meta;")
        .map_err(sql_error)?;
    let entity_ids = snapshot
        .entities
        .keys()
        .map(|id| id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    receipt.projection_rows_deleted +=
        rows::delete_absent_rows(&transaction, "entities", "entity_id", &entity_ids)?;
    for entity in snapshot.entities.values() {
        receipt.projection_rows_written +=
            usize::from(rows::upsert_entity(&transaction, auth_key, entity)?);
    }
    let edge_ids = snapshot
        .edges
        .keys()
        .map(|id| id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    receipt.projection_rows_deleted +=
        rows::delete_absent_rows(&transaction, "edges", "edge_id", &edge_ids)?;
    for edge in snapshot.edges.values() {
        receipt.projection_rows_written +=
            usize::from(rows::upsert_edge(&transaction, auth_key, edge)?);
    }
    let (history_written, history_deleted) =
        history::synchronize(&transaction, auth_key, &snapshot)?;
    receipt.projection_rows_written += history_written;
    receipt.projection_rows_deleted += history_deleted;
    let (auxiliary_written, auxiliary_deleted) =
        auxiliary::synchronize(&transaction, auth_key, &snapshot)?;
    receipt.projection_rows_written += auxiliary_written;
    receipt.projection_rows_deleted += auxiliary_deleted;
    let conflict_mutation = conflicts::synchronize(&transaction, auth_key, &snapshot)?;
    receipt.conflict_rows_read = conflict_mutation.rows_read;
    receipt.conflict_rows_written = conflict_mutation.inserted + conflict_mutation.updated;
    receipt.conflict_rows_deleted = conflict_mutation.deleted;
    receipt.projection_rows_written += receipt.conflict_rows_written;
    receipt.projection_rows_deleted += receipt.conflict_rows_deleted;
    let batch_ids = indexed_batches.keys().cloned().collect::<BTreeSet<_>>();
    receipt.projection_rows_deleted +=
        rows::delete_absent_rows(&transaction, "batches", "batch_id", &batch_ids)?;
    transaction
        .execute_batch("DROP TABLE IF EXISTS cached_batches;")
        .map_err(sql_error)?;
    let event_ids = graph
        .events()
        .map(|event| event.event_id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    events::delete_absent_events(&transaction, &event_ids)?;
    let retracted = rebuild_support::retracted_event_ids(graph.events());
    let active_event_ids = event_ids
        .iter()
        .filter(|event_id| !retracted.contains(event_id.as_str()))
        .map(|event_id| EnterpriseEventId::new(event_id.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    for (batch_id, event_count) in &indexed_batches {
        receipt.projection_rows_written += usize::from(rows::upsert_batch(
            &transaction,
            auth_key,
            batch_id,
            *event_count,
        )?);
    }
    for event in graph.events() {
        let batch_id = event_batches
            .get(&event.event_id)
            .ok_or_else(|| "Scout event lost its verified batch membership".to_string())?;
        events::upsert_event(
            &transaction,
            auth_key,
            batch_id,
            event,
            active_event_ids.contains(&event.event_id),
            true,
        )?;
    }
    drop(graph);
    drop(event_batches);
    drop(active_event_ids);
    let (commitment_state, commitment_work) = commitments::rebuild(
        &transaction,
        enterprise_id,
        event_ids
            .into_iter()
            .map(EnterpriseEventId::new)
            .collect::<Result<Vec<_>, _>>()?,
        &snapshot,
        auth_key,
    )?;
    if commitment_state.event_root_id() != cursor.event_set_root_v1 {
        return Err("Scout rebuilt event root differs from the authoritative ledger".into());
    }
    status.event_root = commitment_state.materialized_event_digest(enterprise_id)?;
    status.graph_digest = commitment_state.materialized_graph_digest(enterprise_id)?;
    receipt.event_root = status.event_root.clone();
    receipt.graph_digest = status.graph_digest.clone();
    status.event_set_root_v1 = Some(commitment_state.event_root_id());
    status.projection_map_root_v2 = Some(commitment_state.projection_root_id());
    receipt.event_set_root_v1 = status.event_set_root_v1.clone();
    receipt.projection_map_root_v2 = status.projection_map_root_v2.clone();
    status.enterprise_snapshot_root_v2 =
        Some(commitment_state.snapshot_root_id(enterprise_id, &status.graph_digest)?);
    receipt.enterprise_snapshot_root_v2 = status.enterprise_snapshot_root_v2.clone();
    receipt.supplemental_rows_written = commitment_work.rows_written;
    receipt.supplemental_rows_deleted = commitment_work.rows_deleted;
    write_projection_meta(
        &transaction,
        auth_key,
        ProjectionMeta {
            enterprise_id,
            cursor,
            status: &status,
            projection_state: &projection_state,
            commitment_state: &commitment_state,
            receipt: &receipt,
        },
    )?;
    transaction.commit().map_err(sql_error)?;
    storage_seal::write(root, auth_key)?;
    Ok(receipt)
}

struct ProjectionMeta<'a> {
    enterprise_id: &'a EnterpriseId,
    cursor: &'a ProjectionLedgerCursor,
    status: &'a IndexedStatus,
    projection_state: &'a state::ProjectionState,
    commitment_state: &'a commitments::CommitmentState,
    receipt: &'a IndexReceipt,
}

fn write_projection_meta(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    meta: ProjectionMeta<'_>,
) -> Result<(), String> {
    write_meta(
        connection,
        "enterprise_id",
        meta.enterprise_id.as_str(),
        auth_key,
    )?;
    write_meta(
        connection,
        "projection_version",
        &PROJECTION_VERSION.to_string(),
        auth_key,
    )?;
    write_meta(
        connection,
        "batch_set_root",
        &meta.cursor.batch_set_root_v1,
        auth_key,
    )?;
    write_meta(connection, "ledger_head_id", &meta.cursor.head_id, auth_key)?;
    write_meta_json(connection, "ledger_cursor", meta.cursor, auth_key)?;
    write_meta_json(connection, "status", meta.status, auth_key)?;
    write_meta_json(
        connection,
        "projection_state",
        meta.projection_state,
        auth_key,
    )?;
    commitments::write(connection, meta.commitment_state, auth_key)?;
    write_meta_json(connection, "receipt", meta.receipt, auth_key)
}

pub(super) fn ensure_after_insert_locked(
    root: &Path,
    enterprise_id: &EnterpriseId,
    envelope: &EnterpriseSignedBatch,
) -> Result<(Connection, IndexReceipt, [u8; INDEX_AUTH_KEY_BYTES]), String> {
    let ledger = super::ledger_authority::open(root, enterprise_id)?;
    let cursor = ProjectionLedgerCursor::from_head(&ledger.head);
    let auth_key = load_or_create_index_auth_key(root)?;
    let storage_is_sealed = storage_seal::validate(root, &auth_key)?;
    let mut connection = open_database(root)?;
    let mut receipt = if storage_is_sealed {
        match incremental::append(&mut connection, enterprise_id, envelope, &cursor, &auth_key) {
            Ok(receipt) => {
                storage_seal::write(root, &auth_key)?;
                receipt
            }
            Err(_) => rebuild(
                root,
                enterprise_id,
                &ledger,
                &cursor,
                &auth_key,
                &mut connection,
            )?,
        }
    } else {
        rebuild(
            root,
            enterprise_id,
            &ledger,
            &cursor,
            &auth_key,
            &mut connection,
        )?
    };
    attach_ledger_work(&mut receipt, &ledger);
    Ok((connection, receipt, auth_key))
}
