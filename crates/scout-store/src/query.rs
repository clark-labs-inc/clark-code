use std::collections::{BTreeMap, BTreeSet, VecDeque};

use agent_orchestration::{
    EnterpriseClassification, EnterpriseEntityId, MaterializedEdge, MaterializedEntity,
};
use base64::Engine;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use super::model::{
    AuthenticatedPageCursor, BatchPage, EdgePage, EdgeQuery, EntityPage, EntityQuery, IndexReceipt,
    IndexedBatch, NeighborhoodPage, PageCursor,
};
use agent_orchestration::EnterpriseId;

use super::index::materialized::PROJECTION_VERSION;
const MAX_RESULTS: usize = 1_000;

mod temporal;

pub(super) use temporal::{qualified_edges, qualified_entities, qualified_neighborhood};

pub(super) fn entities(
    connection: &mut Connection,
    enterprise_id: &EnterpriseId,
    receipt: &IndexReceipt,
    auth_key: &[u8; 32],
    query: EntityQuery,
) -> Result<EntityPage, String> {
    validate_limit(query.limit)?;
    let filter_digest = filter_digest(&(
        &query.kind,
        &query.provider_namespace,
        &query.authority_scope,
        &query.label_contains,
        &query.critical,
    ))?;
    let last_id = decode_cursor(
        auth_key,
        query.cursor.as_deref(),
        enterprise_id,
        receipt,
        &filter_digest,
    )?;
    let kind = enum_name_opt(&query.kind)?;
    let label = query.label_contains.as_ref().map(|value| {
        format!(
            "%{}%",
            value
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    });
    let transaction = connection.transaction().map_err(sql_error)?;
    let mut statement = transaction
        .prepare(
            "SELECT entity_id, kind, provider_namespace, authority_scope, critical,
                    classification_rank, labels_folded, materialized_json, mac FROM entities
             WHERE entity_id > ?1
               AND (?2 IS NULL OR kind = ?2)
               AND (?3 IS NULL OR provider_namespace = ?3)
               AND (?4 IS NULL OR authority_scope = ?4)
               AND (?5 IS NULL OR critical = ?5)
               AND (?6 IS NULL OR labels_folded LIKE ?6 ESCAPE '\\')
               AND classification_rank <= ?7
             ORDER BY entity_id LIMIT ?8",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(
            params![
                last_id,
                kind,
                query.provider_namespace,
                query.authority_scope,
                query.critical,
                label,
                i64::from(EnterpriseClassification::Internal.rank()),
                query.limit + 1
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .map_err(sql_error)?;
    let mut values = rows
        .map(|row| {
            let (id, kind, adapter, scope, critical, rank, labels, json, mac) =
                row.map_err(sql_error)?;
            super::index::verify_index_mac(
                auth_key,
                "entity",
                &(&id, &kind, &adapter, &scope, critical, rank, &labels, &json),
                &mac,
            )?;
            serde_json::from_str(&json).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<MaterializedEntity>, String>>()?;
    drop(statement);
    transaction.commit().map_err(sql_error)?;
    let has_more = values.len() > query.limit;
    values.truncate(query.limit);
    let next_cursor = has_more
        .then(|| values.last())
        .flatten()
        .map(|entity| {
            encode_cursor(
                auth_key,
                PageCursor {
                    enterprise_id: enterprise_id.clone(),
                    event_root: receipt.event_root.clone(),
                    graph_digest: receipt.graph_digest.clone(),
                    projection_version: PROJECTION_VERSION,
                    filter_digest,
                    last_id: entity.entity_id.to_string(),
                },
            )
        })
        .transpose()?;
    Ok(EntityPage {
        entities: values,
        next_cursor,
    })
}

pub(super) fn batches(
    connection: &mut Connection,
    enterprise_id: &EnterpriseId,
    receipt: &IndexReceipt,
    auth_key: &[u8; 32],
    cursor: Option<String>,
    limit: usize,
) -> Result<BatchPage, String> {
    validate_limit(limit)?;
    let filter_digest = filter_digest(&"batches")?;
    let last_id = decode_cursor(
        auth_key,
        cursor.as_deref(),
        enterprise_id,
        receipt,
        &filter_digest,
    )?;
    let transaction = connection.transaction().map_err(sql_error)?;
    let mut statement = transaction
        .prepare(
            "SELECT batch_id, event_count, mac FROM batches
             WHERE batch_id > ?1 ORDER BY batch_id LIMIT ?2",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![last_id, limit + 1], |row| {
            Ok((
                IndexedBatch {
                    batch_id: row.get(0)?,
                    event_count: row.get(1)?,
                },
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(sql_error)?;
    let mut values = rows
        .map(|row| {
            let (batch, mac) = row.map_err(sql_error)?;
            super::index::verify_index_mac(
                auth_key,
                "batch",
                &(&batch.batch_id, batch.event_count),
                &mac,
            )?;
            Ok::<IndexedBatch, String>(batch)
        })
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    transaction.commit().map_err(sql_error)?;
    let has_more = values.len() > limit;
    values.truncate(limit);
    let next_cursor = has_more
        .then(|| values.last())
        .flatten()
        .map(|batch| {
            encode_cursor(
                auth_key,
                PageCursor {
                    enterprise_id: enterprise_id.clone(),
                    event_root: receipt.event_root.clone(),
                    graph_digest: receipt.graph_digest.clone(),
                    projection_version: PROJECTION_VERSION,
                    filter_digest,
                    last_id: batch.batch_id.clone(),
                },
            )
        })
        .transpose()?;
    Ok(BatchPage {
        batches: values,
        next_cursor,
    })
}

pub(super) fn edges(
    connection: &mut Connection,
    enterprise_id: &EnterpriseId,
    receipt: &IndexReceipt,
    auth_key: &[u8; 32],
    query: EdgeQuery,
) -> Result<EdgePage, String> {
    validate_limit(query.limit)?;
    let filter_digest = filter_digest(&(&query.kind, &query.from, &query.to))?;
    let last_id = decode_cursor(
        auth_key,
        query.cursor.as_deref(),
        enterprise_id,
        receipt,
        &filter_digest,
    )?;
    let kind = enum_name_opt(&query.kind)?;
    let transaction = connection.transaction().map_err(sql_error)?;
    let mut statement = transaction
        .prepare(
            "SELECT edge_id, from_id, to_id, kind, classification_rank,
                    materialized_json, mac FROM edges
             WHERE edge_id > ?1
               AND (?2 IS NULL OR kind = ?2)
               AND (?3 IS NULL OR from_id = ?3)
               AND (?4 IS NULL OR to_id = ?4)
               AND classification_rank <= ?5
             ORDER BY edge_id LIMIT ?6",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(
            params![
                last_id,
                kind,
                query.from.as_ref().map(|id| id.as_str()),
                query.to.as_ref().map(|id| id.as_str()),
                i64::from(EnterpriseClassification::Internal.rank()),
                query.limit + 1
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(sql_error)?;
    let mut values = rows
        .map(|row| {
            let (id, from, to, kind, rank, json, mac) = row.map_err(sql_error)?;
            super::index::verify_index_mac(
                auth_key,
                "edge",
                &(&id, &from, &to, &kind, rank, &json),
                &mac,
            )?;
            serde_json::from_str(&json).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<MaterializedEdge>, String>>()?;
    drop(statement);
    transaction.commit().map_err(sql_error)?;
    let has_more = values.len() > query.limit;
    values.truncate(query.limit);
    let next_cursor = has_more
        .then(|| values.last())
        .flatten()
        .map(|edge| {
            encode_cursor(
                auth_key,
                PageCursor {
                    enterprise_id: enterprise_id.clone(),
                    event_root: receipt.event_root.clone(),
                    graph_digest: receipt.graph_digest.clone(),
                    projection_version: PROJECTION_VERSION,
                    filter_digest,
                    last_id: edge.edge_id.to_string(),
                },
            )
        })
        .transpose()?;
    Ok(EdgePage {
        edges: values,
        next_cursor,
    })
}

pub(super) fn neighborhood(
    connection: &Connection,
    auth_key: &[u8; 32],
    seed: EnterpriseEntityId,
    depth: u8,
    limit: usize,
) -> Result<NeighborhoodPage, String> {
    if depth > 8 {
        return Err("enterprise neighborhood depth cannot exceed 8".into());
    }
    validate_limit(limit)?;
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([(seed, 0_u8)]);
    let mut entities = BTreeMap::new();
    let mut truncated = false;
    while let Some((entity_id, level)) = queue.pop_front() {
        if !seen.insert(entity_id.clone()) {
            continue;
        }
        if entities.len() >= limit {
            truncated = true;
            break;
        }
        let Some(entity) = fetch_entity(connection, auth_key, &entity_id)? else {
            continue;
        };
        entities.insert(entity_id.clone(), entity);
        if level >= depth {
            continue;
        }
        let mut statement = connection
            .prepare(
                "SELECT edge_id, from_id, to_id, kind, classification_rank,
                        materialized_json, mac FROM edges
                 WHERE (from_id = ?1 OR to_id = ?1)
                   AND classification_rank <= ?2
                 ORDER BY edge_id",
            )
            .map_err(sql_error)?;
        let neighbors = statement
            .query_map(
                params![
                    entity_id.as_str(),
                    i64::from(EnterpriseClassification::Internal.rank())
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .map_err(sql_error)?;
        for neighbor in neighbors {
            let (id, from, to, kind, rank, json, mac) = neighbor.map_err(sql_error)?;
            super::index::verify_index_mac(
                auth_key,
                "edge",
                &(&id, &from, &to, &kind, rank, &json),
                &mac,
            )?;
            let next = if from == entity_id.as_str() { to } else { from };
            queue.push_back((EnterpriseEntityId::new(next)?, level + 1));
        }
    }
    Ok(NeighborhoodPage {
        entities: entities.into_values().collect(),
        truncated,
    })
}

fn fetch_entity(
    connection: &Connection,
    auth_key: &[u8; 32],
    entity_id: &EnterpriseEntityId,
) -> Result<Option<MaterializedEntity>, String> {
    let mut statement = connection
        .prepare(
            "SELECT entity_id, kind, provider_namespace, authority_scope, critical,
                    classification_rank, labels_folded, materialized_json, mac
             FROM entities WHERE entity_id = ?1 AND classification_rank <= ?2",
        )
        .map_err(sql_error)?;
    let mut rows = statement
        .query(params![
            entity_id.as_str(),
            i64::from(EnterpriseClassification::Internal.rank())
        ])
        .map_err(sql_error)?;
    let Some(row) = rows.next().map_err(sql_error)? else {
        return Ok(None);
    };
    let id: String = row.get(0).map_err(sql_error)?;
    let kind: String = row.get(1).map_err(sql_error)?;
    let adapter: String = row.get(2).map_err(sql_error)?;
    let scope: String = row.get(3).map_err(sql_error)?;
    let critical: bool = row.get(4).map_err(sql_error)?;
    let rank: i64 = row.get(5).map_err(sql_error)?;
    let labels: String = row.get(6).map_err(sql_error)?;
    let json: String = row.get(7).map_err(sql_error)?;
    let mac: String = row.get(8).map_err(sql_error)?;
    super::index::verify_index_mac(
        auth_key,
        "entity",
        &(&id, &kind, &adapter, &scope, critical, rank, &labels, &json),
        &mac,
    )?;
    serde_json::from_str(&json)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub(super) fn decode_cursor(
    auth_key: &[u8; 32],
    cursor: Option<&str>,
    enterprise_id: &EnterpriseId,
    receipt: &IndexReceipt,
    filter_digest: &str,
) -> Result<String, String> {
    let Some(cursor) = cursor else {
        return Ok(String::new());
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| "invalid Scout index cursor encoding".to_string())?;
    let envelope: AuthenticatedPageCursor = serde_json::from_slice(&bytes)
        .map_err(|_| "invalid Scout index cursor payload".to_string())?;
    super::index::verify_index_mac(auth_key, "page_cursor", &envelope.payload, &envelope.mac)?;
    let cursor: PageCursor = serde_json::from_str(&envelope.payload)
        .map_err(|_| "invalid Scout index cursor payload".to_string())?;
    if cursor.enterprise_id != *enterprise_id
        || cursor.event_root != receipt.event_root
        || cursor.graph_digest != receipt.graph_digest
        || cursor.projection_version != PROJECTION_VERSION
        || cursor.filter_digest != filter_digest
    {
        return Err("stale or mismatched Scout index cursor; restart pagination".into());
    }
    Ok(cursor.last_id)
}

pub(super) fn encode_cursor(auth_key: &[u8; 32], cursor: PageCursor) -> Result<String, String> {
    let payload = serde_json::to_string(&cursor).map_err(|error| error.to_string())?;
    let envelope = AuthenticatedPageCursor {
        mac: super::index::index_mac(auth_key, "page_cursor", &payload)?,
        payload,
    };
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&envelope).map_err(|error| error.to_string())?))
}

pub(super) fn filter_digest(value: &impl serde::Serialize) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|error| error.to_string())?)
    ))
}

pub(super) fn enum_name_opt<T: serde::Serialize>(
    value: &Option<T>,
) -> Result<Option<String>, String> {
    value
        .as_ref()
        .map(|value| {
            serde_json::to_value(value)
                .map_err(|error| error.to_string())?
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "enum did not serialize as a string".to_string())
        })
        .transpose()
}

pub(super) fn validate_limit(limit: usize) -> Result<(), String> {
    if limit == 0 || limit > MAX_RESULTS {
        return Err(format!(
            "Scout index query limit must be in 1..={MAX_RESULTS}"
        ));
    }
    Ok(())
}

pub(super) fn sql_error(error: rusqlite::Error) -> String {
    format!("Scout index SQLite query: {error}")
}
