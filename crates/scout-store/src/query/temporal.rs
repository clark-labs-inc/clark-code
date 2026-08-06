use std::collections::{BTreeMap, BTreeSet, VecDeque};

use agent_orchestration::{EnterpriseEntityId, EnterpriseId, MaterializedEdge, MaterializedEntity};
use rusqlite::{params, Connection};

use super::{
    decode_cursor, encode_cursor, enum_name_opt, filter_digest, sql_error, validate_limit,
};
use crate::index::materialized::PROJECTION_VERSION;
use crate::model::{
    EdgePage, EntityPage, IndexReceipt, NeighborhoodPage, NeighborhoodQuery, PageCursor,
    QualifiedEdgeQuery, QualifiedEntityQuery,
};

pub(crate) fn qualified_entities(
    connection: &mut Connection,
    enterprise_id: &EnterpriseId,
    receipt: &IndexReceipt,
    auth_key: &[u8; 32],
    query: QualifiedEntityQuery,
) -> Result<EntityPage, String> {
    validate_limit(query.limit)?;
    let filter_digest = filter_digest(&(
        "qualified_entities",
        &query.kind,
        &query.provider_namespace,
        &query.authority_scope,
        &query.label_contains,
        &query.critical,
        query.as_of_ms,
        query.include_retired,
        query.max_classification,
    ))?;
    let last_key = decode_cursor(
        auth_key,
        query.cursor.as_deref(),
        enterprise_id,
        receipt,
        &filter_digest,
    )?;
    let kind = enum_name_opt(&query.kind)?;
    let label = escaped_label(query.label_contains.as_deref());
    let as_of = sqlite_time(query.as_of_ms)?;
    let transaction = connection.transaction().map_err(sql_error)?;
    let mut statement = transaction
        .prepare(
            "SELECT version_key, entity_id, kind, provider_namespace, authority_scope,
                    critical, classification_rank, labels_folded, valid_from_ms,
                    valid_to_ms, materialized_json, mac
             FROM entity_versions
             WHERE version_key > ?1
               AND (?2 IS NULL OR kind = ?2)
               AND (?3 IS NULL OR provider_namespace = ?3)
               AND (?4 IS NULL OR authority_scope = ?4)
               AND (?5 IS NULL OR critical = ?5)
               AND (?6 IS NULL OR labels_folded LIKE ?6 ESCAPE '\\')
               AND classification_rank <= ?7
               AND (
                 (?8 IS NOT NULL AND valid_from_ms <= ?8
                    AND (valid_to_ms IS NULL OR ?8 < valid_to_ms))
                 OR (?8 IS NULL AND (?9 = 1 OR valid_to_ms IS NULL))
               )
             ORDER BY version_key LIMIT ?10",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(
            params![
                last_key,
                kind,
                query.provider_namespace,
                query.authority_scope,
                query.critical,
                label,
                i64::from(query.max_classification.rank()),
                as_of,
                query.include_retired,
                query.limit + 1,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                ))
            },
        )
        .map_err(sql_error)?;
    let mut values = rows
        .map(|row| {
            let row = row.map_err(sql_error)?;
            crate::index::verify_index_mac(
                auth_key,
                "entity_version",
                &(
                    &row.0, &row.1, &row.2, &row.3, &row.4, row.5, row.6, &row.7, row.8, row.9,
                    &row.10,
                ),
                &row.11,
            )?;
            let entity = serde_json::from_str(&row.10).map_err(|error| error.to_string())?;
            Ok::<_, String>((row.0, entity))
        })
        .collect::<Result<Vec<(String, MaterializedEntity)>, _>>()?;
    drop(statement);
    transaction.commit().map_err(sql_error)?;
    let has_more = values.len() > query.limit;
    values.truncate(query.limit);
    let next_cursor = has_more
        .then(|| values.last())
        .flatten()
        .map(|(key, _)| {
            encode_cursor(
                auth_key,
                PageCursor {
                    enterprise_id: enterprise_id.clone(),
                    event_root: receipt.event_root.clone(),
                    graph_digest: receipt.graph_digest.clone(),
                    projection_version: PROJECTION_VERSION,
                    filter_digest,
                    last_id: key.clone(),
                },
            )
        })
        .transpose()?;
    Ok(EntityPage {
        entities: values.into_iter().map(|(_, entity)| entity).collect(),
        next_cursor,
    })
}

pub(crate) fn qualified_edges(
    connection: &mut Connection,
    enterprise_id: &EnterpriseId,
    receipt: &IndexReceipt,
    auth_key: &[u8; 32],
    query: QualifiedEdgeQuery,
) -> Result<EdgePage, String> {
    validate_limit(query.limit)?;
    let filter_digest = filter_digest(&(
        "qualified_edges",
        &query.kind,
        &query.from,
        &query.to,
        query.as_of_ms,
        query.include_retired,
        query.max_classification,
    ))?;
    let last_key = decode_cursor(
        auth_key,
        query.cursor.as_deref(),
        enterprise_id,
        receipt,
        &filter_digest,
    )?;
    let kind = enum_name_opt(&query.kind)?;
    let as_of = sqlite_time(query.as_of_ms)?;
    let transaction = connection.transaction().map_err(sql_error)?;
    let mut statement = transaction
        .prepare(
            "SELECT version_key, edge_id, from_id, to_id, kind, classification_rank,
                    valid_from_ms, valid_to_ms, materialized_json, mac
             FROM edge_versions
             WHERE version_key > ?1
               AND (?2 IS NULL OR kind = ?2)
               AND (?3 IS NULL OR from_id = ?3)
               AND (?4 IS NULL OR to_id = ?4)
               AND classification_rank <= ?5
               AND (
                 (?6 IS NOT NULL AND valid_from_ms <= ?6
                    AND (valid_to_ms IS NULL OR ?6 < valid_to_ms))
                 OR (?6 IS NULL AND (?7 = 1 OR valid_to_ms IS NULL))
               )
             ORDER BY version_key LIMIT ?8",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(
            params![
                last_key,
                kind,
                query.from.as_ref().map(|id| id.as_str()),
                query.to.as_ref().map(|id| id.as_str()),
                i64::from(query.max_classification.rank()),
                as_of,
                query.include_retired,
                query.limit + 1,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .map_err(sql_error)?;
    let mut values = rows
        .map(|row| {
            let row = row.map_err(sql_error)?;
            crate::index::verify_index_mac(
                auth_key,
                "edge_version",
                &(
                    &row.0, &row.1, &row.2, &row.3, &row.4, row.5, row.6, row.7, &row.8,
                ),
                &row.9,
            )?;
            let edge = serde_json::from_str(&row.8).map_err(|error| error.to_string())?;
            Ok::<_, String>((row.0, edge))
        })
        .collect::<Result<Vec<(String, MaterializedEdge)>, _>>()?;
    drop(statement);
    transaction.commit().map_err(sql_error)?;
    let has_more = values.len() > query.limit;
    values.truncate(query.limit);
    let next_cursor = has_more
        .then(|| values.last())
        .flatten()
        .map(|(key, _)| {
            encode_cursor(
                auth_key,
                PageCursor {
                    enterprise_id: enterprise_id.clone(),
                    event_root: receipt.event_root.clone(),
                    graph_digest: receipt.graph_digest.clone(),
                    projection_version: PROJECTION_VERSION,
                    filter_digest,
                    last_id: key.clone(),
                },
            )
        })
        .transpose()?;
    Ok(EdgePage {
        edges: values.into_iter().map(|(_, edge)| edge).collect(),
        next_cursor,
    })
}

pub(crate) fn qualified_neighborhood(
    connection: &Connection,
    auth_key: &[u8; 32],
    query: NeighborhoodQuery,
) -> Result<NeighborhoodPage, String> {
    if query.depth > 8 {
        return Err("enterprise neighborhood depth cannot exceed 8".into());
    }
    validate_limit(query.limit)?;
    let as_of = sqlite_time(query.as_of_ms)?;
    let clearance = i64::from(query.max_classification.rank());
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([(query.seed, 0_u8)]);
    let mut entities = BTreeMap::new();
    let mut truncated = false;
    while let Some((entity_id, level)) = queue.pop_front() {
        if seen.contains(&entity_id) {
            continue;
        }
        let Some(entity) = fetch_entity_version(
            connection,
            auth_key,
            &entity_id,
            as_of,
            query.include_retired,
            clearance,
        )?
        else {
            continue;
        };
        seen.insert(entity_id.clone());
        if entities.len() >= query.limit {
            truncated = true;
            break;
        }
        entities.insert(entity_id.clone(), entity);
        if level >= query.depth {
            continue;
        }
        for edge in incident_edges(
            connection,
            auth_key,
            &entity_id,
            as_of,
            query.include_retired,
            clearance,
        )? {
            let next = if edge.from == entity_id {
                edge.to
            } else {
                edge.from
            };
            if !seen.contains(&next) {
                queue.push_back((next, level + 1));
            }
        }
    }
    Ok(NeighborhoodPage {
        entities: entities.into_values().collect(),
        truncated,
    })
}

fn fetch_entity_version(
    connection: &Connection,
    auth_key: &[u8; 32],
    entity_id: &EnterpriseEntityId,
    as_of: Option<i64>,
    include_retired: bool,
    clearance: i64,
) -> Result<Option<MaterializedEntity>, String> {
    let mut statement = connection
        .prepare(
            "SELECT version_key, entity_id, kind, provider_namespace, authority_scope,
                    critical, classification_rank, labels_folded, valid_from_ms,
                    valid_to_ms, materialized_json, mac
             FROM entity_versions
             WHERE entity_id = ?1 AND classification_rank <= ?2
               AND (
                 (?3 IS NOT NULL AND valid_from_ms <= ?3
                    AND (valid_to_ms IS NULL OR ?3 < valid_to_ms))
                 OR (?3 IS NULL AND (?4 = 1 OR valid_to_ms IS NULL))
               )
             ORDER BY valid_from_ms DESC LIMIT 1",
        )
        .map_err(sql_error)?;
    let mut rows = statement
        .query(params![
            entity_id.as_str(),
            clearance,
            as_of,
            include_retired
        ])
        .map_err(sql_error)?;
    let Some(row) = rows.next().map_err(sql_error)? else {
        return Ok(None);
    };
    let values = (
        row.get::<_, String>(0).map_err(sql_error)?,
        row.get::<_, String>(1).map_err(sql_error)?,
        row.get::<_, String>(2).map_err(sql_error)?,
        row.get::<_, String>(3).map_err(sql_error)?,
        row.get::<_, String>(4).map_err(sql_error)?,
        row.get::<_, bool>(5).map_err(sql_error)?,
        row.get::<_, i64>(6).map_err(sql_error)?,
        row.get::<_, String>(7).map_err(sql_error)?,
        row.get::<_, i64>(8).map_err(sql_error)?,
        row.get::<_, Option<i64>>(9).map_err(sql_error)?,
        row.get::<_, String>(10).map_err(sql_error)?,
        row.get::<_, String>(11).map_err(sql_error)?,
    );
    crate::index::verify_index_mac(
        auth_key,
        "entity_version",
        &(
            &values.0, &values.1, &values.2, &values.3, &values.4, values.5, values.6, &values.7,
            values.8, values.9, &values.10,
        ),
        &values.11,
    )?;
    serde_json::from_str(&values.10)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn incident_edges(
    connection: &Connection,
    auth_key: &[u8; 32],
    entity_id: &EnterpriseEntityId,
    as_of: Option<i64>,
    include_retired: bool,
    clearance: i64,
) -> Result<Vec<MaterializedEdge>, String> {
    let mut statement = connection
        .prepare(
            "SELECT version_key, edge_id, from_id, to_id, kind, classification_rank,
                    valid_from_ms, valid_to_ms, materialized_json, mac
             FROM edge_versions
             WHERE (from_id = ?1 OR to_id = ?1) AND classification_rank <= ?2
               AND (
                 (?3 IS NOT NULL AND valid_from_ms <= ?3
                    AND (valid_to_ms IS NULL OR ?3 < valid_to_ms))
                 OR (?3 IS NULL AND (?4 = 1 OR valid_to_ms IS NULL))
               )
             ORDER BY edge_id, valid_from_ms DESC",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(
            params![entity_id.as_str(), clearance, as_of, include_retired],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .map_err(sql_error)?;
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();
    for row in rows {
        let row = row.map_err(sql_error)?;
        if !seen.insert(row.1.clone()) {
            continue;
        }
        crate::index::verify_index_mac(
            auth_key,
            "edge_version",
            &(
                &row.0, &row.1, &row.2, &row.3, &row.4, row.5, row.6, row.7, &row.8,
            ),
            &row.9,
        )?;
        edges.push(serde_json::from_str(&row.8).map_err(|error| error.to_string())?);
    }
    Ok(edges)
}

fn sqlite_time(value: Option<u64>) -> Result<Option<i64>, String> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|_| "qualified Scout as_of_ms exceeds SQLite integer range".to_string())
}

fn escaped_label(value: Option<&str>) -> Option<String> {
    value.map(|value| {
        format!(
            "%{}%",
            value
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    })
}
