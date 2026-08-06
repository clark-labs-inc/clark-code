use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    project_event_slice, EnterpriseClassification, EnterpriseId, EnterpriseSignedBatch,
};
use rusqlite::{Connection, TransactionBehavior};

use super::super::database::{read_meta_json, sql_error, INDEX_AUTH_KEY_BYTES};
use super::events;
use super::state::ProjectionState;
use super::{auxiliary, conflicts, rows, write_projection_meta, ProjectionLedgerCursor};
use crate::model::{IndexReceipt, IndexedStatus};

mod affected;
mod status;
mod validation;

pub(super) fn append(
    connection: &mut Connection,
    enterprise_id: &EnterpriseId,
    envelope: &EnterpriseSignedBatch,
    current_cursor: &ProjectionLedgerCursor,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<IndexReceipt, String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql_error)?;
    validation::projection_identity(&transaction, enterprise_id, auth_key)?;
    let previous_cursor: ProjectionLedgerCursor =
        read_meta_json(&transaction, "ledger_cursor", auth_key)?;
    validation::single_append(&previous_cursor, current_cursor, envelope)?;
    let previous_receipt: IndexReceipt = read_meta_json(&transaction, "receipt", auth_key)?;
    if previous_receipt.batch_set_root != previous_cursor.batch_set_root_v1 {
        return Err("Scout cached receipt does not match cached ledger cursor".into());
    }
    let previous_status: IndexedStatus = read_meta_json(&transaction, "status", auth_key)?;
    validation::cached_metadata(
        enterprise_id,
        &previous_cursor,
        &previous_status,
        &previous_receipt,
    )?;
    let mut projection_state: ProjectionState =
        read_meta_json(&transaction, "projection_state", auth_key)?;
    let previous_commitment_state =
        super::commitments::read(&transaction, enterprise_id, auth_key)?;
    let previous_event_set_root = previous_commitment_state.event_root_id();
    let previous_projection_map_root = previous_commitment_state.projection_root_id();
    if previous_status.event_root
        != previous_commitment_state.materialized_event_digest(enterprise_id)?
        || previous_status.graph_digest
            != previous_commitment_state.materialized_graph_digest(enterprise_id)?
        || previous_status.event_set_root_v1.as_deref() != Some(previous_event_set_root.as_str())
        || previous_status.projection_map_root_v2.as_deref()
            != Some(previous_projection_map_root.as_str())
        || previous_status.enterprise_snapshot_root_v2.as_deref()
            != Some(
                previous_commitment_state
                    .snapshot_root_id(enterprise_id, &previous_status.graph_digest)?
                    .as_str(),
            )
    {
        return Err("Scout cached supplemental commitments are inconsistent".into());
    }

    let inserted_event_ids = events::validate_new_events(&transaction, auth_key, &envelope.batch)?;
    let authoritative_event_delta = current_cursor
        .event_count
        .checked_sub(previous_cursor.event_count)
        .ok_or_else(|| "Scout authoritative event count moved backwards".to_string())?;
    if authoritative_event_delta != inserted_event_ids.len() as u64 {
        return Err("Scout authoritative event delta differs from the materialized append".into());
    }
    let include_topology = projection_state.current_pass_id.is_none();
    validation::control_barrier(
        envelope,
        &inserted_event_ids,
        projection_state.sealed_epoch_sequence(),
    )?;
    let locators = validation::inserted_locators(envelope, &inserted_event_ids, include_topology)?;
    let mut projection_events = events::read_projection_events(&transaction, auth_key, &locators)?;
    let events_replayed = projection_events.len();
    projection_events.extend(
        envelope
            .batch
            .events
            .iter()
            .filter(|event| inserted_event_ids.contains(&event.event_id))
            .filter(|event| validation::locator_is_selected(event, &locators))
            .cloned(),
    );
    let mut update = project_event_slice(&projection_events, include_topology);

    let changed_entity_ids = update.entities.keys().cloned().collect::<BTreeSet<_>>();
    let incident_edges =
        rows::read_edges_incident_to_entities(&transaction, auth_key, &changed_entity_ids)?;
    let incident_edges_reclassified = incident_edges.len();
    let direct_edge_ids = update.edges.keys().cloned().collect::<BTreeSet<_>>();
    let mut old_edges = incident_edges;
    old_edges.extend(rows::read_edges_by_ids(
        &transaction,
        auth_key,
        &direct_edge_ids,
    )?);
    let mut candidate_edges = old_edges.clone();
    candidate_edges.extend(update.edges.clone());

    let coverage_ids = update.coverage.keys().cloned().collect::<BTreeSet<_>>();
    let frontier_ids = update.frontier.keys().cloned().collect::<BTreeSet<_>>();
    let simulation_ids = changed_entity_ids
        .iter()
        .chain(update.simulation_contracts.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let existing_conflicts = conflicts::read_affected(
        &transaction,
        auth_key,
        &conflicts::ConflictScope {
            coverage: coverage_ids.clone(),
            frontier: frontier_ids.clone(),
            simulation: update.simulation_contracts.keys().cloned().collect(),
            dangling_edges: candidate_edges.keys().cloned().collect(),
        },
    )?;
    let existing_auxiliary = auxiliary::read_existing(
        &transaction,
        auth_key,
        &coverage_ids,
        &frontier_ids,
        &simulation_ids,
    )?;
    let mut required_entity_ids = changed_entity_ids.clone();
    required_entity_ids.extend(update.simulation_contracts.keys().cloned());
    for edge in candidate_edges.values() {
        required_entity_ids.insert(edge.from.clone());
        required_entity_ids.insert(edge.to.clone());
    }
    let old_entities = rows::read_entities_by_ids(&transaction, auth_key, &required_entity_ids)?;
    affected::classify_candidate_edges(&mut candidate_edges, &update.entities, &old_entities);
    update.edges = affected::changed_edges(&old_edges, &candidate_edges);
    let mut next_conflicts = update.conflicts.clone();
    next_conflicts.extend(affected::dangling_edge_conflicts(
        &candidate_edges,
        &update.entities,
        &old_entities,
    ));
    let simulation_visibility = simulation_ids
        .iter()
        .map(|runtime_id| {
            let visible = update
                .entities
                .get(runtime_id)
                .or_else(|| old_entities.get(runtime_id))
                .is_some_and(|entity| {
                    EnterpriseClassification::Internal.permits(entity.classification)
                });
            (runtime_id.clone(), visible)
        })
        .collect::<BTreeMap<_, _>>();
    let conflict_mutation = conflicts::apply_affected(
        &transaction,
        auth_key,
        &existing_conflicts.conflicts,
        &next_conflicts,
        &simulation_visibility,
    )?;
    projection_state.conflict_count = adjusted_count(
        projection_state.conflict_count,
        conflict_mutation.deleted,
        conflict_mutation.inserted,
        "conflict",
    )?;
    projection_state.visible_conflict_count = adjusted_signed_count(
        projection_state.visible_conflict_count,
        conflict_mutation.visible_delta,
        "visible conflict",
    )?;

    let old_changed_entities = old_entities
        .iter()
        .filter(|(entity_id, _)| changed_entity_ids.contains(*entity_id))
        .map(|(entity_id, entity)| (entity_id.clone(), entity.clone()))
        .collect::<BTreeMap<_, _>>();
    let visible_entities = status::adjust_visible_count(
        previous_status.entities,
        &old_changed_entities,
        &update.entities,
    )?;
    let visible_edges =
        status::adjust_visible_count(previous_status.edges, &old_edges, &candidate_edges)?;
    let visible_simulation_contracts = affected::updated_simulation_count(
        previous_status.simulation_contracts,
        &update,
        &old_entities,
        &existing_auxiliary.simulation,
    )?;

    projection_state.coverage_cell_count = increased_count(
        projection_state.coverage_cell_count,
        update.coverage.keys(),
        &existing_auxiliary.coverage,
        "coverage cell",
    )?;
    projection_state.frontier_task_count = increased_count(
        projection_state.frontier_task_count,
        update.frontier.keys(),
        &existing_auxiliary.frontier,
        "frontier task",
    )?;
    projection_state.simulation_contract_count = increased_count(
        projection_state.simulation_contract_count,
        update.simulation_contracts.keys(),
        &existing_auxiliary.simulation,
        "simulation contract",
    )?;
    projection_state.entity_row_count = projection_state
        .entity_row_count
        .checked_add(
            update
                .entities
                .keys()
                .filter(|entity_id| !old_changed_entities.contains_key(*entity_id))
                .count(),
        )
        .ok_or_else(|| "Scout entity row count overflowed".to_string())?;
    projection_state.edge_row_count = projection_state
        .edge_row_count
        .checked_add(
            update
                .edges
                .keys()
                .filter(|edge_id| !old_edges.contains_key(*edge_id))
                .count(),
        )
        .ok_or_else(|| "Scout edge row count overflowed".to_string())?;
    let affected_projection_rows = update.entities.len()
        + update.edges.len()
        + update.coverage.len()
        + update.frontier.len()
        + update.simulation_contracts.len()
        + conflict_mutation.inserted
        + conflict_mutation.updated
        + conflict_mutation.deleted;

    let conflict_preview = conflicts::visible_preview(&transaction, auth_key, 64)?;
    if conflict_preview.conflicts.len() > projection_state.visible_conflict_count {
        return Err("Scout visible conflict preview exceeds its authenticated count".into());
    }
    let mut status = status::build(
        enterprise_id,
        &projection_state,
        status::StatusCounts {
            batches: usize::try_from(current_cursor.batch_count)
                .map_err(|_| "Scout ledger batch count does not fit this platform".to_string())?,
            events: usize::try_from(current_cursor.event_count)
                .map_err(|_| "Scout ledger event count does not fit this platform".to_string())?,
            entities: visible_entities,
            edges: visible_edges,
            simulation_contracts: visible_simulation_contracts,
        },
        conflict_preview.conflicts,
    );
    let mut receipt = IndexReceipt {
        event_root: String::new(),
        graph_digest: String::new(),
        event_set_root_v1: None,
        projection_map_root_v2: None,
        enterprise_snapshot_root_v2: None,
        batch_set_root: current_cursor.batch_set_root_v1.clone(),
        ledger_authority_work: Default::default(),
        rebuilt: false,
        derived_batches_read: 0,
        events_replayed,
        event_ids_scanned: 0,
        entity_rows_read: old_entities.len(),
        edge_rows_read: old_edges.len(),
        history_rows_read: 0,
        auxiliary_rows_read: existing_auxiliary.rows_read,
        conflict_rows_read: existing_conflicts.rows_read
            + conflict_mutation.rows_read
            + conflict_preview.rows_read,
        conflict_rows_written: conflict_mutation.inserted + conflict_mutation.updated,
        conflict_rows_deleted: conflict_mutation.deleted,
        incident_edges_reclassified,
        affected_projection_rows,
        full_projection_fallback: false,
        projection_rows_written: conflict_mutation.inserted + conflict_mutation.updated,
        projection_rows_deleted: conflict_mutation.deleted,
        supplemental_rows_written: 0,
        supplemental_rows_deleted: 0,
    };

    for entity in update.entities.values() {
        receipt.projection_rows_written +=
            usize::from(rows::upsert_entity(&transaction, auth_key, entity)?);
    }
    for edge in update.edges.values() {
        receipt.projection_rows_written +=
            usize::from(rows::upsert_edge(&transaction, auth_key, edge)?);
    }
    receipt.projection_rows_written += auxiliary::upsert_slice(&transaction, auth_key, &update)?;
    receipt.projection_rows_written += usize::from(rows::upsert_batch(
        &transaction,
        auth_key,
        envelope.batch.batch_id.as_str(),
        envelope.batch.events.len(),
    )?);
    let active_batch_events = envelope
        .batch
        .events
        .iter()
        .map(|event| event.event_id.clone())
        .collect();
    events::upsert_batch_events(
        &transaction,
        auth_key,
        &envelope.batch,
        &active_batch_events,
        false,
    )?;
    let (commitment_state, commitment_work) = super::commitments::append(
        &transaction,
        enterprise_id,
        previous_commitment_state,
        super::commitments::ProjectionDelta {
            inserted_event_ids: &inserted_event_ids,
            update: &update,
            projection_state: &projection_state,
            conflict_mutation: &conflict_mutation,
        },
        auth_key,
    )?;
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
    if commitment_state.event_root_id() != current_cursor.event_set_root_v1 {
        return Err("Scout materialized event root differs from the authoritative ledger".into());
    }
    transaction
        .execute_batch("DELETE FROM meta;")
        .map_err(sql_error)?;
    write_projection_meta(
        &transaction,
        auth_key,
        super::ProjectionMeta {
            enterprise_id,
            cursor: current_cursor,
            status: &status,
            projection_state: &projection_state,
            commitment_state: &commitment_state,
            receipt: &receipt,
        },
    )?;
    transaction.commit().map_err(sql_error)?;
    Ok(receipt)
}

fn increased_count<'a, I: 'a + Ord>(
    previous: usize,
    changed: impl IntoIterator<Item = &'a I>,
    existing: &BTreeSet<I>,
    label: &str,
) -> Result<usize, String> {
    previous
        .checked_add(
            changed
                .into_iter()
                .filter(|identity| !existing.contains(*identity))
                .count(),
        )
        .ok_or_else(|| format!("Scout {label} count overflowed"))
}

fn adjusted_count(
    previous: usize,
    removed: usize,
    added: usize,
    label: &str,
) -> Result<usize, String> {
    previous
        .checked_sub(removed)
        .and_then(|count| count.checked_add(added))
        .ok_or_else(|| format!("Scout {label} count overflowed its prior state"))
}

fn adjusted_signed_count(previous: usize, delta: i64, label: &str) -> Result<usize, String> {
    if delta >= 0 {
        previous
            .checked_add(delta as usize)
            .ok_or_else(|| format!("Scout {label} count overflowed"))
    } else {
        previous
            .checked_sub(delta.unsigned_abs() as usize)
            .ok_or_else(|| format!("Scout {label} count underflowed"))
    }
}
