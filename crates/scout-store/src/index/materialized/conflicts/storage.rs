use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{EnterpriseConflict, EnterpriseEdgeId};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};

use super::{conflict_visible, locator, DANGLING_EDGE, SIMULATION_DISAGREEMENT};
use crate::index::database::{index_mac, sql_error, verify_index_mac, INDEX_AUTH_KEY_BYTES};

const SQL_PARAMETER_CHUNK: usize = 899;
const SELECT_ROW: &str = "SELECT conflict_key, kind_rank, locator_a, locator_b, visible_internal,
            materialized_json, mac
     FROM projection_conflicts";

pub(super) type StoredRow = (String, i64, String, String, bool, String, String);

pub(super) fn key_from_locator(
    kind_rank: i64,
    locator_a: &str,
    locator_b: &str,
) -> Result<String, String> {
    serde_json::to_string(&(
        "scout-projection-conflict-v1",
        kind_rank,
        locator_a,
        locator_b,
    ))
    .map_err(|error| error.to_string())
}

pub(super) fn encode(
    conflict: &EnterpriseConflict,
    visible: bool,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<StoredRow, String> {
    let (kind_rank, locator_a, locator_b) = locator(conflict);
    let materialized_json = serde_json::to_string(conflict).map_err(|error| error.to_string())?;
    let mut row = (
        key_from_locator(kind_rank, locator_a, &locator_b)?,
        kind_rank,
        locator_a.into(),
        locator_b,
        visible,
        materialized_json,
        String::new(),
    );
    row.6 = row_mac(auth_key, &row)?;
    Ok(row)
}

pub(super) fn row_mac(
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    row: &StoredRow,
) -> Result<String, String> {
    index_mac(
        auth_key,
        "projection_conflict",
        &(&row.0, row.1, &row.2, &row.3, row.4, &row.5),
    )
}

fn authenticate(
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    row: StoredRow,
) -> Result<(String, EnterpriseConflict), String> {
    verify_index_mac(
        auth_key,
        "projection_conflict",
        &(&row.0, row.1, &row.2, &row.3, row.4, &row.5),
        &row.6,
    )?;
    let conflict: EnterpriseConflict =
        serde_json::from_str(&row.5).map_err(|error| error.to_string())?;
    let (kind_rank, locator_a, locator_b) = locator(&conflict);
    if row.0 != key_from_locator(kind_rank, locator_a, &locator_b)?
        || row.1 != kind_rank
        || row.2 != locator_a
        || row.3 != locator_b
        || (kind_rank != SIMULATION_DISAGREEMENT && row.4 != conflict_visible(&conflict, |_| false))
    {
        return Err("Scout authenticated conflict row identity mismatch".into());
    }
    Ok((row.0, conflict))
}

pub(super) fn read_by_keys(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    keys: &BTreeSet<String>,
    output: &mut BTreeMap<String, EnterpriseConflict>,
) -> Result<(), String> {
    let keys = keys.iter().map(String::as_str).collect::<Vec<_>>();
    for chunk in keys.chunks(SQL_PARAMETER_CHUNK) {
        let placeholders = sql_placeholders(chunk.len(), 1);
        let query = format!("{SELECT_ROW} WHERE conflict_key IN ({placeholders})");
        let mut statement = connection.prepare(&query).map_err(sql_error)?;
        let rows = statement
            .query_map(params_from_iter(chunk.iter().copied()), stored_row)
            .map_err(sql_error)?;
        for row in rows {
            let (key, conflict) = authenticate(auth_key, row.map_err(sql_error)?)?;
            output.insert(key, conflict);
        }
    }
    Ok(())
}

pub(super) fn read_dangling(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    edge_ids: &BTreeSet<EnterpriseEdgeId>,
    output: &mut BTreeMap<String, EnterpriseConflict>,
) -> Result<(), String> {
    let ids = edge_ids
        .iter()
        .map(EnterpriseEdgeId::as_str)
        .collect::<Vec<_>>();
    for chunk in ids.chunks(SQL_PARAMETER_CHUNK) {
        let placeholders = sql_placeholders(chunk.len(), 1);
        let query = format!(
            "{SELECT_ROW} WHERE kind_rank = {DANGLING_EDGE} AND locator_a IN ({placeholders})"
        );
        let mut statement = connection.prepare(&query).map_err(sql_error)?;
        let rows = statement
            .query_map(params_from_iter(chunk.iter().copied()), stored_row)
            .map_err(sql_error)?;
        for row in rows {
            let (key, conflict) = authenticate(auth_key, row.map_err(sql_error)?)?;
            output.insert(key, conflict);
        }
    }
    Ok(())
}

pub(super) fn read_row(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    key: &str,
) -> Result<Option<StoredRow>, String> {
    let row = connection
        .query_row(
            &format!("{SELECT_ROW} WHERE conflict_key = ?1"),
            [key],
            stored_row,
        )
        .optional()
        .map_err(sql_error)?;
    row.map(|row| authenticate(auth_key, row.clone()).map(|_| row))
        .transpose()
}

pub(super) fn read_visible_preview(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    limit: usize,
) -> Result<Vec<EnterpriseConflict>, String> {
    let mut statement = connection
        .prepare(&format!(
            "{SELECT_ROW}
             WHERE visible_internal = 1
             ORDER BY kind_rank, locator_a COLLATE BINARY, locator_b COLLATE BINARY,
                      conflict_key COLLATE BINARY
             LIMIT ?1"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map([limit as i64], stored_row)
        .map_err(sql_error)?;
    rows.map(|row| authenticate(auth_key, row.map_err(sql_error)?).map(|(_, value)| value))
        .collect()
}

fn stored_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

pub(super) fn upsert(connection: &Connection, row: &StoredRow) -> Result<bool, String> {
    connection
        .execute(
            "INSERT INTO projection_conflicts VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(conflict_key) DO UPDATE SET
               kind_rank=excluded.kind_rank, locator_a=excluded.locator_a,
               locator_b=excluded.locator_b, visible_internal=excluded.visible_internal,
               materialized_json=excluded.materialized_json, mac=excluded.mac
             WHERE kind_rank <> excluded.kind_rank OR locator_a <> excluded.locator_a
                OR locator_b <> excluded.locator_b
                OR visible_internal <> excluded.visible_internal
                OR materialized_json <> excluded.materialized_json OR mac <> excluded.mac",
            params![&row.0, row.1, &row.2, &row.3, row.4, &row.5, &row.6],
        )
        .map(|changed| changed != 0)
        .map_err(sql_error)
}

fn sql_placeholders(count: usize, first: usize) -> String {
    (first..first + count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}
