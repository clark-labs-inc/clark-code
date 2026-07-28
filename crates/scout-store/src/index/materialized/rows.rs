use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    EnterpriseEdgeId, EnterpriseEntityId, MaterializedEdge, MaterializedEntity,
};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};

use super::super::database::{index_mac, sql_error, verify_index_mac, INDEX_AUTH_KEY_BYTES};

const SQL_PARAMETER_CHUNK: usize = 900;
const ENTITY_SELECT: &str = "SELECT entity_id, kind, provider_namespace, authority_scope, critical,
            classification_rank, labels_folded, materialized_json, mac
     FROM entities";
const EDGE_SELECT: &str =
    "SELECT edge_id, from_id, to_id, kind, classification_rank, materialized_json, mac
     FROM edges";

type EntityRow = (
    String,
    String,
    String,
    String,
    bool,
    i64,
    String,
    String,
    String,
);
type EdgeRow = (String, String, String, String, i64, String, String);

pub(super) fn read_entities_by_ids(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    entity_ids: &BTreeSet<EnterpriseEntityId>,
) -> Result<BTreeMap<EnterpriseEntityId, MaterializedEntity>, String> {
    let mut entities = BTreeMap::new();
    let ids = entity_ids
        .iter()
        .map(EnterpriseEntityId::as_str)
        .collect::<Vec<_>>();
    for chunk in ids.chunks(SQL_PARAMETER_CHUNK) {
        let query = format!(
            "{ENTITY_SELECT} WHERE entity_id IN ({})",
            sql_placeholders(chunk.len())
        );
        let mut statement = connection.prepare(&query).map_err(sql_error)?;
        let rows = statement
            .query_map(params_from_iter(chunk.iter().copied()), read_entity_row)
            .map_err(sql_error)?;
        for row in rows {
            insert_authenticated_entity(auth_key, row.map_err(sql_error)?, &mut entities)?;
        }
    }
    Ok(entities)
}

pub(super) fn read_edges_by_ids(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    edge_ids: &BTreeSet<EnterpriseEdgeId>,
) -> Result<BTreeMap<EnterpriseEdgeId, MaterializedEdge>, String> {
    let mut edges = BTreeMap::new();
    let ids = edge_ids
        .iter()
        .map(EnterpriseEdgeId::as_str)
        .collect::<Vec<_>>();
    for chunk in ids.chunks(SQL_PARAMETER_CHUNK) {
        let query = format!(
            "{EDGE_SELECT} WHERE edge_id IN ({})",
            sql_placeholders(chunk.len())
        );
        let mut statement = connection.prepare(&query).map_err(sql_error)?;
        let rows = statement
            .query_map(params_from_iter(chunk.iter().copied()), read_edge_row)
            .map_err(sql_error)?;
        for row in rows {
            insert_authenticated_edge(auth_key, row.map_err(sql_error)?, &mut edges)?;
        }
    }
    Ok(edges)
}

pub(super) fn read_edges_incident_to_entities(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    entity_ids: &BTreeSet<EnterpriseEntityId>,
) -> Result<BTreeMap<EnterpriseEdgeId, MaterializedEdge>, String> {
    let mut edges = BTreeMap::new();
    let ids = entity_ids
        .iter()
        .map(EnterpriseEntityId::as_str)
        .collect::<Vec<_>>();
    for chunk in ids.chunks(SQL_PARAMETER_CHUNK) {
        let placeholders = sql_placeholders(chunk.len());
        // Repeating numbered placeholders reuses the same bindings, keeping this at N
        // parameters instead of 2N while matching either endpoint.
        let query =
            format!("{EDGE_SELECT} WHERE from_id IN ({placeholders}) OR to_id IN ({placeholders})");
        let mut statement = connection.prepare(&query).map_err(sql_error)?;
        let rows = statement
            .query_map(params_from_iter(chunk.iter().copied()), read_edge_row)
            .map_err(sql_error)?;
        for row in rows {
            insert_authenticated_edge(auth_key, row.map_err(sql_error)?, &mut edges)?;
        }
    }
    Ok(edges)
}

fn read_entity_row(row: &Row<'_>) -> rusqlite::Result<EntityRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn read_edge_row(row: &Row<'_>) -> rusqlite::Result<EdgeRow> {
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

fn insert_authenticated_entity(
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    row: EntityRow,
    entities: &mut BTreeMap<EnterpriseEntityId, MaterializedEntity>,
) -> Result<(), String> {
    verify_index_mac(
        auth_key,
        "entity",
        &(
            row.0.as_str(),
            &row.1,
            &row.2,
            &row.3,
            row.4,
            &row.5,
            &row.6,
            &row.7,
        ),
        &row.8,
    )?;
    let entity: MaterializedEntity =
        serde_json::from_str(&row.7).map_err(|error| error.to_string())?;
    if entity.entity_id.as_str() != row.0 {
        return Err("Scout authenticated entity row identity mismatch".into());
    }
    if entities
        .get(&entity.entity_id)
        .is_some_and(|existing| existing != &entity)
    {
        return Err("Scout authenticated entity rows contain conflicting duplicates".into());
    }
    entities.insert(entity.entity_id.clone(), entity);
    Ok(())
}

fn insert_authenticated_edge(
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    row: EdgeRow,
    edges: &mut BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
) -> Result<(), String> {
    verify_index_mac(
        auth_key,
        "edge",
        &(
            row.0.as_str(),
            row.1.as_str(),
            row.2.as_str(),
            &row.3,
            &row.4,
            &row.5,
        ),
        &row.6,
    )?;
    let edge: MaterializedEdge = serde_json::from_str(&row.5).map_err(|error| error.to_string())?;
    if edge.edge_id.as_str() != row.0 {
        return Err("Scout authenticated edge row identity mismatch".into());
    }
    if edges
        .get(&edge.edge_id)
        .is_some_and(|existing| existing != &edge)
    {
        return Err("Scout authenticated edge rows contain conflicting duplicates".into());
    }
    edges.insert(edge.edge_id.clone(), edge);
    Ok(())
}

fn sql_placeholders(count: usize) -> String {
    (1..=count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn delete_absent_rows(
    connection: &Connection,
    table: &str,
    id_column: &str,
    retained: &BTreeSet<String>,
) -> Result<usize, String> {
    let select = format!("SELECT {id_column} FROM {table}");
    let mut statement = connection.prepare(&select).map_err(sql_error)?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    drop(statement);
    let delete = format!("DELETE FROM {table} WHERE {id_column} = ?1");
    let mut deleted = 0;
    for id in ids {
        if !retained.contains(&id) {
            deleted += connection.execute(&delete, [&id]).map_err(sql_error)?;
        }
    }
    Ok(deleted)
}

pub(super) fn upsert_entity(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    entity: &MaterializedEntity,
) -> Result<bool, String> {
    let kind = enum_name(&entity.kind)?;
    let labels_folded = entity
        .labels
        .iter()
        .map(|label| label.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let materialized_json = serde_json::to_string(entity).map_err(|error| error.to_string())?;
    let classification_rank = i64::from(entity.classification.rank());
    let mac = index_mac(
        auth_key,
        "entity",
        &(
            entity.entity_id.as_str(),
            &kind,
            &entity.authority.provider_namespace,
            &entity.authority.authority_scope,
            entity.critical,
            classification_rank,
            &labels_folded,
            &materialized_json,
        ),
    )?;
    let existing = connection
        .query_row(
            "SELECT kind, provider_namespace, authority_scope, critical,
                    classification_rank, labels_folded, materialized_json, mac
             FROM entities WHERE entity_id = ?1",
            [entity.entity_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    let expected = (
        kind.as_str(),
        entity.authority.provider_namespace.as_str(),
        entity.authority.authority_scope.as_str(),
        entity.critical,
        classification_rank,
        labels_folded.as_str(),
        materialized_json.as_str(),
        mac.as_str(),
    );
    if existing.as_ref().map(|row| {
        (
            row.0.as_str(),
            row.1.as_str(),
            row.2.as_str(),
            row.3,
            row.4,
            row.5.as_str(),
            row.6.as_str(),
            row.7.as_str(),
        )
    }) == Some(expected)
    {
        return Ok(false);
    }
    connection
        .execute(
            "INSERT INTO entities VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(entity_id) DO UPDATE SET
               kind=excluded.kind, provider_namespace=excluded.provider_namespace,
               authority_scope=excluded.authority_scope, critical=excluded.critical,
               classification_rank=excluded.classification_rank,
               labels_folded=excluded.labels_folded,
               materialized_json=excluded.materialized_json, mac=excluded.mac",
            params![
                entity.entity_id.as_str(),
                kind,
                entity.authority.provider_namespace,
                entity.authority.authority_scope,
                entity.critical,
                classification_rank,
                labels_folded,
                materialized_json,
                mac,
            ],
        )
        .map_err(sql_error)?;
    Ok(true)
}

pub(super) fn upsert_edge(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    edge: &MaterializedEdge,
) -> Result<bool, String> {
    let kind = enum_name(&edge.kind)?;
    let materialized_json = serde_json::to_string(edge).map_err(|error| error.to_string())?;
    let classification_rank = i64::from(edge.classification.rank());
    let mac = index_mac(
        auth_key,
        "edge",
        &(
            edge.edge_id.as_str(),
            edge.from.as_str(),
            edge.to.as_str(),
            &kind,
            classification_rank,
            &materialized_json,
        ),
    )?;
    let existing = connection
        .query_row(
            "SELECT from_id, to_id, kind, classification_rank, materialized_json, mac
             FROM edges WHERE edge_id = ?1",
            [edge.edge_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    let expected = (
        edge.from.as_str(),
        edge.to.as_str(),
        kind.as_str(),
        classification_rank,
        materialized_json.as_str(),
        mac.as_str(),
    );
    if existing.as_ref().map(|row| {
        (
            row.0.as_str(),
            row.1.as_str(),
            row.2.as_str(),
            row.3,
            row.4.as_str(),
            row.5.as_str(),
        )
    }) == Some(expected)
    {
        return Ok(false);
    }
    connection
        .execute(
            "INSERT INTO edges VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(edge_id) DO UPDATE SET
               from_id=excluded.from_id, to_id=excluded.to_id, kind=excluded.kind,
               classification_rank=excluded.classification_rank,
               materialized_json=excluded.materialized_json, mac=excluded.mac",
            params![
                edge.edge_id.as_str(),
                edge.from.as_str(),
                edge.to.as_str(),
                kind,
                classification_rank,
                materialized_json,
                mac,
            ],
        )
        .map_err(sql_error)?;
    Ok(true)
}

pub(super) fn upsert_batch(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    batch_id: &str,
    event_count: usize,
) -> Result<bool, String> {
    let mac = index_mac(auth_key, "batch", &(batch_id, event_count))?;
    let existing = connection
        .query_row(
            "SELECT event_count, mac FROM batches WHERE batch_id = ?1",
            [batch_id],
            |row| Ok((row.get::<_, usize>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(sql_error)?;
    if existing
        .as_ref()
        .map(|(count, observed_mac)| (*count, observed_mac.as_str()))
        == Some((event_count, mac.as_str()))
    {
        return Ok(false);
    }
    connection
        .execute(
            "INSERT INTO batches VALUES (?1, ?2, ?3)
             ON CONFLICT(batch_id) DO UPDATE SET
               event_count=excluded.event_count, mac=excluded.mac",
            params![batch_id, event_count, mac],
        )
        .map_err(sql_error)?;
    Ok(true)
}

fn enum_name(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_value(value)
        .map_err(|error| error.to_string())?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "enum did not serialize as a string".to_string())
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
