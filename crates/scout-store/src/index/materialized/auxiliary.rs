use std::collections::BTreeSet;

use agent_orchestration::{
    CoverageCellId, EnterpriseEntityId, EnterpriseProjectionSlice, EnterpriseSnapshot,
    FrontierTaskId, MaterializedCoverage, MaterializedFrontier, MaterializedSimulationContract,
};
use rusqlite::{params, params_from_iter, Connection};
use serde::{de::DeserializeOwned, Serialize};

use super::super::database::{index_mac, sql_error, verify_index_mac, INDEX_AUTH_KEY_BYTES};

const SQL_PARAMETER_CHUNK: usize = 899;
const COVERAGE_LANE: &str = "coverage";
const FRONTIER_LANE: &str = "frontier";
const SIMULATION_LANE: &str = "simulation";

type AuthenticatedRow = (String, String, String);

pub(super) struct ExistingAuxiliary {
    pub coverage: BTreeSet<CoverageCellId>,
    pub frontier: BTreeSet<FrontierTaskId>,
    pub simulation: BTreeSet<EnterpriseEntityId>,
    pub rows_read: usize,
}

pub(super) fn read_existing(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    coverage_ids: &BTreeSet<CoverageCellId>,
    frontier_ids: &BTreeSet<FrontierTaskId>,
    simulation_ids: &BTreeSet<EnterpriseEntityId>,
) -> Result<ExistingAuxiliary, String> {
    let coverage = read_lane(
        connection,
        auth_key,
        COVERAGE_LANE,
        coverage_ids,
        CoverageCellId::as_str,
        |value: &MaterializedCoverage| &value.cell_id,
    )?;
    let frontier = read_lane(
        connection,
        auth_key,
        FRONTIER_LANE,
        frontier_ids,
        FrontierTaskId::as_str,
        |value: &MaterializedFrontier| &value.task_id,
    )?;
    let simulation = read_lane(
        connection,
        auth_key,
        SIMULATION_LANE,
        simulation_ids,
        EnterpriseEntityId::as_str,
        |value: &MaterializedSimulationContract| &value.runtime_id,
    )?;
    Ok(ExistingAuxiliary {
        rows_read: coverage.len() + frontier.len() + simulation.len(),
        coverage,
        frontier,
        simulation,
    })
}

pub(super) fn synchronize(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    snapshot: &EnterpriseSnapshot,
) -> Result<(usize, usize), String> {
    let mut deleted = delete_absent(
        connection,
        COVERAGE_LANE,
        snapshot.coverage.keys().map(CoverageCellId::as_str),
    )?;
    deleted += delete_absent(
        connection,
        FRONTIER_LANE,
        snapshot.frontier.keys().map(FrontierTaskId::as_str),
    )?;
    deleted += delete_absent(
        connection,
        SIMULATION_LANE,
        snapshot
            .simulation_contracts
            .keys()
            .map(EnterpriseEntityId::as_str),
    )?;
    let mut written = 0;
    for value in snapshot.coverage.values() {
        written += usize::from(upsert(
            connection,
            auth_key,
            COVERAGE_LANE,
            value.cell_id.as_str(),
            value,
        )?);
    }
    for value in snapshot.frontier.values() {
        written += usize::from(upsert(
            connection,
            auth_key,
            FRONTIER_LANE,
            value.task_id.as_str(),
            value,
        )?);
    }
    for value in snapshot.simulation_contracts.values() {
        written += usize::from(upsert(
            connection,
            auth_key,
            SIMULATION_LANE,
            value.runtime_id.as_str(),
            value,
        )?);
    }
    Ok((written, deleted))
}

pub(super) fn upsert_slice(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    update: &EnterpriseProjectionSlice,
) -> Result<usize, String> {
    let mut written = 0;
    for value in update.coverage.values() {
        written += usize::from(upsert(
            connection,
            auth_key,
            COVERAGE_LANE,
            value.cell_id.as_str(),
            value,
        )?);
    }
    for value in update.frontier.values() {
        written += usize::from(upsert(
            connection,
            auth_key,
            FRONTIER_LANE,
            value.task_id.as_str(),
            value,
        )?);
    }
    for value in update.simulation_contracts.values() {
        written += usize::from(upsert(
            connection,
            auth_key,
            SIMULATION_LANE,
            value.runtime_id.as_str(),
            value,
        )?);
    }
    Ok(written)
}

fn read_lane<I, T>(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    lane: &str,
    ids: &BTreeSet<I>,
    id_string: impl Fn(&I) -> &str,
    identity: impl Fn(&T) -> &I,
) -> Result<BTreeSet<I>, String>
where
    I: Clone + Ord,
    T: DeserializeOwned,
{
    let strings = ids.iter().map(&id_string).collect::<Vec<_>>();
    let mut found = BTreeSet::new();
    for chunk in strings.chunks(SQL_PARAMETER_CHUNK) {
        let placeholders = (0..chunk.len())
            .map(|index| format!("?{}", index + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let query = format!(
            "SELECT object_id, materialized_json, mac
             FROM auxiliary_projection
             WHERE lane = ?1 AND object_id IN ({placeholders})"
        );
        let values = std::iter::once(lane).chain(chunk.iter().copied());
        let mut statement = connection.prepare(&query).map_err(sql_error)?;
        let rows = statement
            .query_map(params_from_iter(values), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(sql_error)?;
        for row in rows {
            let row = row.map_err(sql_error)?;
            let value: T = authenticate_row(auth_key, lane, &row)?;
            if id_string(identity(&value)) != row.0 {
                return Err("Scout authenticated auxiliary projection identity mismatch".into());
            }
            found.insert(identity(&value).clone());
        }
    }
    Ok(found)
}

fn authenticate_row<T: DeserializeOwned>(
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    lane: &str,
    row: &AuthenticatedRow,
) -> Result<T, String> {
    verify_index_mac(
        auth_key,
        "auxiliary_projection",
        &(lane, row.0.as_str(), row.1.as_str()),
        &row.2,
    )?;
    serde_json::from_str(&row.1).map_err(|error| error.to_string())
}

fn upsert(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    lane: &str,
    object_id: &str,
    value: &impl Serialize,
) -> Result<bool, String> {
    let materialized_json = serde_json::to_string(value).map_err(|error| error.to_string())?;
    let mac = index_mac(
        auth_key,
        "auxiliary_projection",
        &(lane, object_id, materialized_json.as_str()),
    )?;
    let changed = connection
        .execute(
            "INSERT INTO auxiliary_projection VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(lane, object_id) DO UPDATE SET
               materialized_json=excluded.materialized_json, mac=excluded.mac
             WHERE materialized_json <> excluded.materialized_json OR mac <> excluded.mac",
            params![lane, object_id, materialized_json, mac],
        )
        .map_err(sql_error)?;
    Ok(changed != 0)
}

fn delete_absent<'a>(
    connection: &Connection,
    lane: &str,
    retained: impl IntoIterator<Item = &'a str>,
) -> Result<usize, String> {
    let retained = retained.into_iter().collect::<BTreeSet<_>>();
    let mut statement = connection
        .prepare("SELECT object_id FROM auxiliary_projection WHERE lane = ?1")
        .map_err(sql_error)?;
    let ids = statement
        .query_map([lane], |row| row.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    drop(statement);
    let mut deleted = 0;
    for object_id in ids {
        if !retained.contains(object_id.as_str()) {
            deleted += connection
                .execute(
                    "DELETE FROM auxiliary_projection WHERE lane = ?1 AND object_id = ?2",
                    params![lane, object_id],
                )
                .map_err(sql_error)?;
        }
    }
    Ok(deleted)
}
