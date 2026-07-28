use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    CoverageCellId, EnterpriseClassification, EnterpriseConflict, EnterpriseEdgeId,
    EnterpriseEntityId, EnterpriseSnapshot, FrontierTaskId,
};
use rusqlite::Connection;

use super::super::database::{sql_error, INDEX_AUTH_KEY_BYTES};

const MAX_STATUS_PREVIEW: usize = 64;
const SOURCE_EQUIVOCATION: i64 = 0;
const ORPHAN_RETRACTION: i64 = 1;
const RETRACTION_OF_RETRACTION: i64 = 2;
const DANGLING_EDGE: i64 = 3;
const COVERAGE_DISAGREEMENT: i64 = 4;
const FRONTIER_DISAGREEMENT: i64 = 5;
const SIMULATION_DISAGREEMENT: i64 = 6;
const CHARTER_DISAGREEMENT: i64 = 7;
const DISCOVERY_PASS_INVALID: i64 = 8;
const DISCOVERY_PASS_FORK: i64 = 9;
const DISCOVERY_PASS_NON_MONOTONIC: i64 = 10;

mod storage;

use storage::{
    encode, key_from_locator, read_by_keys, read_dangling, read_row, read_visible_preview, row_mac,
    upsert,
};

#[derive(Default)]
pub(super) struct ConflictScope {
    pub coverage: BTreeSet<CoverageCellId>,
    pub frontier: BTreeSet<FrontierTaskId>,
    pub simulation: BTreeSet<EnterpriseEntityId>,
    pub dangling_edges: BTreeSet<EnterpriseEdgeId>,
}

#[derive(Debug)]
pub(super) struct ConflictRead {
    pub conflicts: BTreeSet<EnterpriseConflict>,
    pub rows_read: usize,
}

#[derive(Debug)]
pub(super) struct ConflictPreview {
    pub conflicts: Vec<EnterpriseConflict>,
    pub rows_read: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ConflictMutation {
    pub inserted: usize,
    pub updated: usize,
    pub deleted: usize,
    pub visible_delta: i64,
    /// Conflict rows returned by SQLite, excluding point misses.
    pub rows_read: usize,
    puts: BTreeMap<String, EnterpriseConflict>,
    removals: BTreeSet<String>,
}

impl ConflictMutation {
    fn extend(&mut self, other: Self) {
        self.inserted += other.inserted;
        self.updated += other.updated;
        self.deleted += other.deleted;
        self.visible_delta += other.visible_delta;
        self.rows_read += other.rows_read;
        self.puts.extend(other.puts);
        self.removals.extend(other.removals);
    }

    pub(super) fn commitment_puts(&self) -> impl Iterator<Item = (&str, &EnterpriseConflict)> {
        self.puts
            .iter()
            .map(|(identity, conflict)| (identity.as_str(), conflict))
    }

    pub(super) fn commitment_removals(&self) -> impl Iterator<Item = &str> {
        self.removals.iter().map(String::as_str)
    }
}

pub(super) fn stable_key(conflict: &EnterpriseConflict) -> Result<String, String> {
    let (kind_rank, locator_a, locator_b) = locator(conflict);
    serde_json::to_string(&(
        "scout-projection-conflict-v1",
        kind_rank,
        locator_a,
        locator_b,
    ))
    .map_err(|error| error.to_string())
}

pub(super) fn snapshot_visible_count(snapshot: &EnterpriseSnapshot) -> usize {
    snapshot
        .conflicts
        .iter()
        .filter(|conflict| {
            conflict_visible(conflict, |runtime_id| {
                snapshot.entities.get(runtime_id).is_some_and(|entity| {
                    EnterpriseClassification::Internal.permits(entity.classification)
                })
            })
        })
        .count()
}

pub(super) fn synchronize(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    snapshot: &EnterpriseSnapshot,
) -> Result<ConflictMutation, String> {
    let mut desired = BTreeMap::new();
    for conflict in &snapshot.conflicts {
        let visible = conflict_visible(conflict, |runtime_id| {
            snapshot.entities.get(runtime_id).is_some_and(|entity| {
                EnterpriseClassification::Internal.permits(entity.classification)
            })
        });
        let row = encode(conflict, visible, auth_key)?;
        if desired.insert(row.0.clone(), row).is_some() {
            return Err("Scout conflicts contain a duplicate stable logical key".into());
        }
    }
    let existing = connection
        .prepare("SELECT conflict_key, visible_internal FROM projection_conflicts")
        .map_err(sql_error)?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })
        .map_err(sql_error)?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(sql_error)?;
    let mut mutation = ConflictMutation {
        rows_read: existing.len(),
        ..ConflictMutation::default()
    };
    for (key, visible) in &existing {
        if !desired.contains_key(key) {
            mutation.deleted += connection
                .execute(
                    "DELETE FROM projection_conflicts WHERE conflict_key = ?1",
                    [key],
                )
                .map_err(sql_error)?;
            mutation.visible_delta -= i64::from(*visible);
            mutation.removals.insert(key.clone());
        }
    }
    for (key, row) in desired {
        let changed = upsert(connection, &row)?;
        if !changed {
            continue;
        }
        match existing.get(&key) {
            Some(was_visible) => {
                mutation.updated += 1;
                mutation.visible_delta += i64::from(row.4) - i64::from(*was_visible);
            }
            None => {
                mutation.inserted += 1;
                mutation.visible_delta += i64::from(row.4);
            }
        }
        let conflict = serde_json::from_str(&row.5).map_err(|error| error.to_string())?;
        mutation.puts.insert(key, conflict);
    }
    Ok(mutation)
}

pub(super) fn read_affected(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    scope: &ConflictScope,
) -> Result<ConflictRead, String> {
    let mut rows = BTreeMap::new();
    let keys = scope
        .coverage
        .iter()
        .map(|id| key_from_locator(COVERAGE_DISAGREEMENT, id.as_str(), ""))
        .chain(
            scope
                .frontier
                .iter()
                .map(|id| key_from_locator(FRONTIER_DISAGREEMENT, id.as_str(), "")),
        )
        .chain(
            scope
                .simulation
                .iter()
                .map(|id| key_from_locator(SIMULATION_DISAGREEMENT, id.as_str(), "")),
        )
        .collect::<Result<BTreeSet<_>, _>>()?;
    read_by_keys(connection, auth_key, &keys, &mut rows)?;
    read_dangling(connection, auth_key, &scope.dangling_edges, &mut rows)?;
    Ok(ConflictRead {
        rows_read: rows.len(),
        conflicts: rows.into_values().collect(),
    })
}

pub(super) fn apply_affected(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    old: &BTreeSet<EnterpriseConflict>,
    new: &BTreeSet<EnterpriseConflict>,
    simulation_visibility: &BTreeMap<EnterpriseEntityId, bool>,
) -> Result<ConflictMutation, String> {
    let old_by_key = conflicts_by_key(old)?;
    let mut new_rows = BTreeMap::new();
    for conflict in new {
        let visible = match conflict {
            EnterpriseConflict::SimulationContractDisagreement { runtime_id, .. } => {
                *simulation_visibility.get(runtime_id).ok_or_else(|| {
                    format!(
                        "Scout simulation conflict visibility missing for {}",
                        runtime_id.as_str()
                    )
                })?
            }
            _ => conflict_visible(conflict, |_| false),
        };
        let row = encode(conflict, visible, auth_key)?;
        if new_rows.insert(row.0.clone(), row).is_some() {
            return Err("Scout conflicts contain a duplicate stable logical key".into());
        }
    }
    let relevant_keys = old_by_key
        .keys()
        .chain(new_rows.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut persisted = BTreeMap::new();
    read_by_keys(connection, auth_key, &relevant_keys, &mut persisted)?;
    for (key, conflict) in &old_by_key {
        if persisted.get(key) != Some(conflict) {
            return Err(
                "Scout affected conflict slice does not match authenticated storage".into(),
            );
        }
    }
    for key in new_rows.keys() {
        if persisted.contains_key(key) && !old_by_key.contains_key(key) {
            return Err("Scout affected conflict slice omitted an existing logical row".into());
        }
    }
    let mut mutation = ConflictMutation {
        rows_read: persisted.len(),
        ..ConflictMutation::default()
    };
    for key in old_by_key.keys().filter(|key| !new_rows.contains_key(*key)) {
        let visible = read_row(connection, auth_key, key)?
            .ok_or_else(|| "Scout affected conflict disappeared during update".to_string())?
            .4;
        mutation.rows_read += 1;
        mutation.deleted += connection
            .execute(
                "DELETE FROM projection_conflicts WHERE conflict_key = ?1",
                [key],
            )
            .map_err(sql_error)?;
        mutation.visible_delta -= i64::from(visible);
        mutation.removals.insert(key.clone());
    }
    for (key, row) in &new_rows {
        let previous = read_row(connection, auth_key, key)?;
        mutation.rows_read += usize::from(previous.is_some());
        if upsert(connection, row)? {
            match previous {
                Some(previous) => {
                    mutation.updated += 1;
                    mutation.visible_delta += i64::from(row.4) - i64::from(previous.4);
                }
                None => {
                    mutation.inserted += 1;
                    mutation.visible_delta += i64::from(row.4);
                }
            }
            let conflict = serde_json::from_str(&row.5).map_err(|error| error.to_string())?;
            mutation.puts.insert(key.clone(), conflict);
        }
    }
    let inserted_simulations = new_rows
        .values()
        .filter(|row| row.1 == SIMULATION_DISAGREEMENT)
        .map(|row| row.2.as_str())
        .collect::<BTreeSet<_>>();
    let remaining_visibility = simulation_visibility
        .iter()
        .filter(|(id, _)| !inserted_simulations.contains(id.as_str()))
        .map(|(id, visible)| (id.clone(), *visible))
        .collect();
    mutation.extend(update_simulation_visibility(
        connection,
        auth_key,
        &remaining_visibility,
    )?);
    Ok(mutation)
}

pub(super) fn update_simulation_visibility(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    visibility: &BTreeMap<EnterpriseEntityId, bool>,
) -> Result<ConflictMutation, String> {
    let mut mutation = ConflictMutation::default();
    for (runtime_id, visible) in visibility {
        let key = key_from_locator(SIMULATION_DISAGREEMENT, runtime_id.as_str(), "")?;
        let Some(mut row) = read_row(connection, auth_key, &key)? else {
            continue;
        };
        mutation.rows_read += 1;
        if row.4 == *visible {
            continue;
        }
        let previous = row.4;
        row.4 = *visible;
        row.6 = row_mac(auth_key, &row)?;
        if upsert(connection, &row)? {
            mutation.updated += 1;
            mutation.visible_delta += i64::from(*visible) - i64::from(previous);
        }
    }
    Ok(mutation)
}

pub(super) fn visible_preview(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    limit: usize,
) -> Result<ConflictPreview, String> {
    if limit > MAX_STATUS_PREVIEW {
        return Err("Scout conflict status preview exceeds 64 rows".into());
    }
    let conflicts = read_visible_preview(connection, auth_key, limit)?;
    // Derived EnterpriseConflict::Ord compares the variant discriminant, then
    // fields in declaration order. Each logical row is unique before any
    // mutable event-id set, and SQLite BINARY ordering matches the ASCII
    // identifier/string ordering used by those fields. Keep a runtime check so
    // adding a variant or changing field order fails closed.
    let mut rust_order = conflicts.clone();
    rust_order.sort();
    if conflicts != rust_order {
        return Err("Scout conflict preview ordering diverged from EnterpriseConflict::Ord".into());
    }
    Ok(ConflictPreview {
        rows_read: conflicts.len(),
        conflicts,
    })
}

fn conflicts_by_key(
    conflicts: &BTreeSet<EnterpriseConflict>,
) -> Result<BTreeMap<String, EnterpriseConflict>, String> {
    let mut keyed = BTreeMap::new();
    for conflict in conflicts {
        let key = stable_key(conflict)?;
        if keyed.insert(key, conflict.clone()).is_some() {
            return Err("Scout conflicts contain a duplicate stable logical key".into());
        }
    }
    Ok(keyed)
}

fn conflict_visible(
    conflict: &EnterpriseConflict,
    simulation_visible: impl FnOnce(&EnterpriseEntityId) -> bool,
) -> bool {
    match conflict {
        EnterpriseConflict::SourceEquivocation { .. }
        | EnterpriseConflict::OrphanRetraction { .. }
        | EnterpriseConflict::RetractionOfRetraction { .. }
        | EnterpriseConflict::DanglingEdge { .. } => false,
        EnterpriseConflict::SimulationContractDisagreement { runtime_id, .. } => {
            simulation_visible(runtime_id)
        }
        EnterpriseConflict::CoverageDisagreement { .. }
        | EnterpriseConflict::FrontierDisagreement { .. }
        | EnterpriseConflict::CharterDisagreement { .. }
        | EnterpriseConflict::DiscoveryPassInvalid { .. }
        | EnterpriseConflict::DiscoveryPassFork { .. }
        | EnterpriseConflict::DiscoveryPassNonMonotonic { .. } => true,
    }
}

fn locator(conflict: &EnterpriseConflict) -> (i64, &str, String) {
    match conflict {
        EnterpriseConflict::SourceEquivocation {
            source_position, ..
        } => (SOURCE_EQUIVOCATION, source_position, String::new()),
        EnterpriseConflict::OrphanRetraction {
            retraction_event_id,
            target_event_id,
        } => (
            ORPHAN_RETRACTION,
            retraction_event_id.as_str(),
            target_event_id.as_str().into(),
        ),
        EnterpriseConflict::RetractionOfRetraction {
            retraction_event_id,
            target_event_id,
        } => (
            RETRACTION_OF_RETRACTION,
            retraction_event_id.as_str(),
            target_event_id.as_str().into(),
        ),
        EnterpriseConflict::DanglingEdge {
            edge_id,
            missing_entity_id,
        } => (
            DANGLING_EDGE,
            edge_id.as_str(),
            missing_entity_id.as_str().into(),
        ),
        EnterpriseConflict::CoverageDisagreement { cell_id, .. } => {
            (COVERAGE_DISAGREEMENT, cell_id.as_str(), String::new())
        }
        EnterpriseConflict::FrontierDisagreement { task_id, .. } => {
            (FRONTIER_DISAGREEMENT, task_id.as_str(), String::new())
        }
        EnterpriseConflict::SimulationContractDisagreement { runtime_id, .. } => {
            (SIMULATION_DISAGREEMENT, runtime_id.as_str(), String::new())
        }
        EnterpriseConflict::CharterDisagreement { .. } => (CHARTER_DISAGREEMENT, "", String::new()),
        EnterpriseConflict::DiscoveryPassInvalid { pass_id } => {
            (DISCOVERY_PASS_INVALID, pass_id, String::new())
        }
        EnterpriseConflict::DiscoveryPassFork {
            discovery_epoch_sequence,
            ..
        } => (
            DISCOVERY_PASS_FORK,
            "",
            format!("{discovery_epoch_sequence:020}"),
        ),
        EnterpriseConflict::DiscoveryPassNonMonotonic {
            first_pass_id,
            confirming_pass_id,
        } => (
            DISCOVERY_PASS_NON_MONOTONIC,
            first_pass_id,
            confirming_pass_id.clone(),
        ),
    }
}

#[cfg(test)]
mod tests;
