use std::collections::BTreeSet;

use agent_orchestration::{EnterpriseSnapshot, MaterializedEdge, MaterializedEntity};
use rusqlite::{params, Connection, OptionalExtension};

use super::super::database::{index_mac, sql_error, INDEX_AUTH_KEY_BYTES};
use super::rows;

pub(super) fn synchronize(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    snapshot: &EnterpriseSnapshot,
) -> Result<(usize, usize), String> {
    let entity_keys = snapshot
        .entity_history
        .values()
        .flatten()
        .map(entity_version_key)
        .collect::<Result<BTreeSet<_>, _>>()?;
    let edge_keys = snapshot
        .edge_history
        .values()
        .flatten()
        .map(edge_version_key)
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut deleted =
        rows::delete_absent_rows(connection, "entity_versions", "version_key", &entity_keys)?;
    deleted += rows::delete_absent_rows(connection, "edge_versions", "version_key", &edge_keys)?;
    let mut written = 0;
    for entity in snapshot.entity_history.values().flatten() {
        written += usize::from(upsert_entity_version(connection, auth_key, entity)?);
    }
    for edge in snapshot.edge_history.values().flatten() {
        written += usize::from(upsert_edge_version(connection, auth_key, edge)?);
    }
    Ok((written, deleted))
}

fn upsert_entity_version(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    entity: &MaterializedEntity,
) -> Result<bool, String> {
    let version_key = entity_version_key(entity)?;
    let kind = enum_name(&entity.kind)?;
    let labels_folded = entity
        .labels
        .iter()
        .map(|label| label.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let valid_from_ms = required_valid_from(entity.valid_from_ms)?;
    let valid_to_ms = entity
        .valid_to_ms
        .map(i64::try_from)
        .transpose()
        .map_err(|_| {
            "qualified enterprise entity valid_to_ms exceeds SQLite integer range".to_string()
        })?;
    let valid_from_ms = i64::try_from(valid_from_ms).map_err(|_| {
        "qualified enterprise entity valid_from_ms exceeds SQLite integer range".to_string()
    })?;
    let classification_rank = i64::from(entity.classification.rank());
    let materialized_json = serde_json::to_string(entity).map_err(|error| error.to_string())?;
    let authenticated = (
        version_key.as_str(),
        entity.entity_id.as_str(),
        kind.as_str(),
        entity.authority.provider_namespace.as_str(),
        entity.authority.authority_scope.as_str(),
        entity.critical,
        classification_rank,
        labels_folded.as_str(),
        valid_from_ms,
        valid_to_ms,
        materialized_json.as_str(),
    );
    let mac = index_mac(auth_key, "entity_version", &authenticated)?;
    let existing = connection
        .query_row(
            "SELECT kind, provider_namespace, authority_scope, critical,
                    classification_rank, labels_folded, valid_from_ms, valid_to_ms,
                    materialized_json, mac
             FROM entity_versions WHERE version_key = ?1",
            [&version_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    if existing.as_ref().is_some_and(|row| {
        row.0 == kind
            && row.1 == entity.authority.provider_namespace
            && row.2 == entity.authority.authority_scope
            && row.3 == entity.critical
            && row.4 == classification_rank
            && row.5 == labels_folded
            && row.6 == valid_from_ms
            && row.7 == valid_to_ms
            && row.8 == materialized_json
            && row.9 == mac
    }) {
        return Ok(false);
    }
    connection
        .execute(
            "INSERT INTO entity_versions VALUES
               (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(version_key) DO UPDATE SET
               entity_id=excluded.entity_id, kind=excluded.kind,
               provider_namespace=excluded.provider_namespace,
               authority_scope=excluded.authority_scope, critical=excluded.critical,
               classification_rank=excluded.classification_rank,
               labels_folded=excluded.labels_folded, valid_from_ms=excluded.valid_from_ms,
               valid_to_ms=excluded.valid_to_ms,
               materialized_json=excluded.materialized_json, mac=excluded.mac",
            params![
                version_key,
                entity.entity_id.as_str(),
                kind,
                entity.authority.provider_namespace,
                entity.authority.authority_scope,
                entity.critical,
                classification_rank,
                labels_folded,
                valid_from_ms,
                valid_to_ms,
                materialized_json,
                mac,
            ],
        )
        .map_err(sql_error)?;
    Ok(true)
}

fn upsert_edge_version(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    edge: &MaterializedEdge,
) -> Result<bool, String> {
    let version_key = edge_version_key(edge)?;
    let kind = enum_name(&edge.kind)?;
    let valid_from_ms = required_valid_from(edge.valid_from_ms)?;
    let valid_to_ms = edge
        .valid_to_ms
        .map(i64::try_from)
        .transpose()
        .map_err(|_| {
            "qualified enterprise edge valid_to_ms exceeds SQLite integer range".to_string()
        })?;
    let valid_from_ms = i64::try_from(valid_from_ms).map_err(|_| {
        "qualified enterprise edge valid_from_ms exceeds SQLite integer range".to_string()
    })?;
    let classification_rank = i64::from(edge.classification.rank());
    let materialized_json = serde_json::to_string(edge).map_err(|error| error.to_string())?;
    let authenticated = (
        version_key.as_str(),
        edge.edge_id.as_str(),
        edge.from.as_str(),
        edge.to.as_str(),
        kind.as_str(),
        classification_rank,
        valid_from_ms,
        valid_to_ms,
        materialized_json.as_str(),
    );
    let mac = index_mac(auth_key, "edge_version", &authenticated)?;
    let existing = connection
        .query_row(
            "SELECT from_id, to_id, kind, classification_rank, valid_from_ms,
                    valid_to_ms, materialized_json, mac
             FROM edge_versions WHERE version_key = ?1",
            [&version_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    if existing.as_ref().is_some_and(|row| {
        row.0 == edge.from.as_str()
            && row.1 == edge.to.as_str()
            && row.2 == kind
            && row.3 == classification_rank
            && row.4 == valid_from_ms
            && row.5 == valid_to_ms
            && row.6 == materialized_json
            && row.7 == mac
    }) {
        return Ok(false);
    }
    connection
        .execute(
            "INSERT INTO edge_versions VALUES
               (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(version_key) DO UPDATE SET
               edge_id=excluded.edge_id, from_id=excluded.from_id, to_id=excluded.to_id,
               kind=excluded.kind, classification_rank=excluded.classification_rank,
               valid_from_ms=excluded.valid_from_ms, valid_to_ms=excluded.valid_to_ms,
               materialized_json=excluded.materialized_json, mac=excluded.mac",
            params![
                version_key,
                edge.edge_id.as_str(),
                edge.from.as_str(),
                edge.to.as_str(),
                kind,
                classification_rank,
                valid_from_ms,
                valid_to_ms,
                materialized_json,
                mac,
            ],
        )
        .map_err(sql_error)?;
    Ok(true)
}

pub(super) fn entity_version_key(entity: &MaterializedEntity) -> Result<String, String> {
    Ok(format!(
        "{}|{:020}",
        entity.entity_id,
        required_valid_from(entity.valid_from_ms)?
    ))
}

pub(super) fn edge_version_key(edge: &MaterializedEdge) -> Result<String, String> {
    Ok(format!(
        "{}|{:020}",
        edge.edge_id,
        required_valid_from(edge.valid_from_ms)?
    ))
}

fn required_valid_from(value: Option<u64>) -> Result<u64, String> {
    value.ok_or_else(|| "qualified enterprise history record has no valid_from_ms".to_string())
}

fn enum_name(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_value(value)
        .map_err(|error| error.to_string())?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "enum did not serialize as a string".to_string())
}
