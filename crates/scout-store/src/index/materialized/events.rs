use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{EnterpriseBatch, EnterpriseEvent, EnterpriseEventId, EnterpriseFact};
use rusqlite::{params, Connection, OptionalExtension};

use super::super::database::{index_mac, sql_error, verify_index_mac, INDEX_AUTH_KEY_BYTES};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ProjectionLocator {
    pub kind: &'static str,
    pub key: String,
}

pub(super) fn locator(event: &EnterpriseEvent) -> Result<ProjectionLocator, String> {
    let (kind, key) = match &event.fact {
        EnterpriseFact::EntityObserved(value) => ("entity", value.entity_id.to_string()),
        EnterpriseFact::EdgeObserved(value) => ("edge", value.edge_id.to_string()),
        EnterpriseFact::CoverageObserved(value) => ("coverage", value.cell_id.to_string()),
        EnterpriseFact::FrontierObserved(value) => ("frontier", value.task_id.to_string()),
        EnterpriseFact::SimulationContractObserved(value) => {
            ("simulation", value.runtime_id.to_string())
        }
        EnterpriseFact::DiscoveryCharterObserved(_)
        | EnterpriseFact::DiscoveryPassSealed(_)
        | EnterpriseFact::ObservationRetracted { .. } => {
            return Err("Scout control-plane append requires an immutable cold rebuild".into())
        }
    };
    Ok(ProjectionLocator { kind, key })
}

pub(super) fn source_position(event: &EnterpriseEvent) -> String {
    let value = &event.provenance;
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        value.machine_id,
        value.run_id,
        value.adapter_instance_id,
        value.auth_context_id,
        value.discovery_epoch,
        value.discovery_epoch_sequence,
        value.source_sequence
    )
}

pub(super) fn validate_new_events(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    batch: &EnterpriseBatch,
) -> Result<BTreeSet<EnterpriseEventId>, String> {
    let mut inserted = BTreeSet::new();
    let mut positions = BTreeMap::<String, EnterpriseEventId>::new();
    for event in &batch.events {
        let existing = read_cached_event(connection, auth_key, event.event_id.as_str())?;
        match existing {
            Some(existing) if existing == *event => {}
            Some(_) => return Err("enterprise event-id collision".into()),
            None => {
                let position = source_position(event);
                if positions
                    .insert(position.clone(), event.event_id.clone())
                    .is_some()
                    || connection
                        .query_row(
                            "SELECT 1 FROM cached_events WHERE source_position = ?1 LIMIT 1",
                            [&position],
                            |_| Ok(()),
                        )
                        .optional()
                        .map_err(sql_error)?
                        .is_some()
                {
                    return Err(
                        "Scout source-position conflict requires an immutable cold rebuild".into(),
                    );
                }
                inserted.insert(event.event_id.clone());
            }
        }
    }
    Ok(inserted)
}

pub(super) fn read_projection_events(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    locators: &BTreeSet<ProjectionLocator>,
) -> Result<Vec<EnterpriseEvent>, String> {
    let mut events = Vec::new();
    for locator in locators {
        let mut statement = connection
            .prepare(
                "SELECT event_id, batch_id, projection_kind, projection_key,
                        source_position, event_json, active, mac
                 FROM cached_events
                 WHERE projection_kind = ?1 AND projection_key = ?2 AND active = 1
                 ORDER BY event_id",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map(params![locator.kind, locator.key], read_event_row)
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        for row in rows {
            events.push(verify_event_row(auth_key, row)?);
        }
    }
    Ok(events)
}

pub(super) fn upsert_batch_events(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    batch: &EnterpriseBatch,
    active: &BTreeSet<EnterpriseEventId>,
    replace_existing: bool,
) -> Result<(), String> {
    for event in &batch.events {
        upsert_event(
            connection,
            auth_key,
            batch.batch_id.as_str(),
            event,
            active.contains(&event.event_id),
            replace_existing,
        )?;
    }
    Ok(())
}

pub(super) fn upsert_event(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    batch_id: &str,
    event: &EnterpriseEvent,
    is_active: bool,
    replace_existing: bool,
) -> Result<(), String> {
    let statement = if replace_existing {
        "INSERT INTO cached_events
         (event_id, batch_id, projection_kind, projection_key, source_position,
          event_json, active, mac)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(event_id) DO UPDATE SET
           batch_id=excluded.batch_id, projection_kind=excluded.projection_kind,
           projection_key=excluded.projection_key,
           source_position=excluded.source_position, event_json=excluded.event_json,
           active=excluded.active, mac=excluded.mac"
    } else {
        "INSERT INTO cached_events
         (event_id, batch_id, projection_kind, projection_key, source_position,
          event_json, active, mac)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(event_id) DO NOTHING"
    };
    let locator = locator_for_cache(event);
    let event_id = event.event_id.as_str();
    let source_position = source_position(event);
    let event_json = serde_json::to_string(event).map_err(|error| error.to_string())?;
    let mac = index_mac(
        auth_key,
        "cached_event",
        &(
            event_id,
            batch_id,
            locator.kind,
            locator.key.as_str(),
            source_position.as_str(),
            event_json.as_str(),
            is_active,
        ),
    )?;
    connection
        .execute(
            statement,
            params![
                event_id,
                batch_id,
                locator.kind,
                locator.key,
                source_position,
                event_json,
                is_active,
                mac,
            ],
        )
        .map_err(sql_error)?;
    Ok(())
}

pub(super) fn delete_absent_events(
    connection: &Connection,
    retained: &BTreeSet<String>,
) -> Result<usize, String> {
    let mut statement = connection
        .prepare("SELECT event_id FROM cached_events")
        .map_err(sql_error)?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    drop(statement);
    let mut deleted = 0;
    for event_id in ids {
        if !retained.contains(&event_id) {
            deleted += connection
                .execute("DELETE FROM cached_events WHERE event_id = ?1", [&event_id])
                .map_err(sql_error)?;
        }
    }
    Ok(deleted)
}

fn read_cached_event(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    event_id: &str,
) -> Result<Option<EnterpriseEvent>, String> {
    connection
        .query_row(
            "SELECT event_id, batch_id, projection_kind, projection_key,
                    source_position, event_json, active, mac
             FROM cached_events WHERE event_id = ?1",
            [event_id],
            read_event_row,
        )
        .optional()
        .map_err(sql_error)?
        .map(|row| verify_event_row(auth_key, row))
        .transpose()
}

type EventRow = (String, String, String, String, String, String, bool, String);

fn read_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn verify_event_row(
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    row: EventRow,
) -> Result<EnterpriseEvent, String> {
    verify_index_mac(
        auth_key,
        "cached_event",
        &(
            row.0.as_str(),
            row.1.as_str(),
            row.2.as_str(),
            row.3.as_str(),
            row.4.as_str(),
            row.5.as_str(),
            row.6,
        ),
        &row.7,
    )?;
    let event: EnterpriseEvent = serde_json::from_str(&row.5).map_err(|error| error.to_string())?;
    let locator = locator_for_cache(&event);
    if event.event_id.as_str() != row.0
        || locator.kind != row.2
        || locator.key != row.3
        || source_position(&event) != row.4
    {
        return Err("Scout authenticated event cache identity mismatch".into());
    }
    event.validate()?;
    Ok(event)
}

fn locator_for_cache(event: &EnterpriseEvent) -> ProjectionLocator {
    match &event.fact {
        EnterpriseFact::EntityObserved(value) => ProjectionLocator {
            kind: "entity",
            key: value.entity_id.to_string(),
        },
        EnterpriseFact::EdgeObserved(value) => ProjectionLocator {
            kind: "edge",
            key: value.edge_id.to_string(),
        },
        EnterpriseFact::CoverageObserved(value) => ProjectionLocator {
            kind: "coverage",
            key: value.cell_id.to_string(),
        },
        EnterpriseFact::FrontierObserved(value) => ProjectionLocator {
            kind: "frontier",
            key: value.task_id.to_string(),
        },
        EnterpriseFact::SimulationContractObserved(value) => ProjectionLocator {
            kind: "simulation",
            key: value.runtime_id.to_string(),
        },
        EnterpriseFact::DiscoveryCharterObserved(_) => ProjectionLocator {
            kind: "control_charter",
            key: "charter".into(),
        },
        EnterpriseFact::DiscoveryPassSealed(value) => ProjectionLocator {
            kind: "control_pass",
            key: value.pass_id.clone(),
        },
        EnterpriseFact::ObservationRetracted {
            target_event_id, ..
        } => ProjectionLocator {
            kind: "control_retraction",
            key: target_event_id.to_string(),
        },
    }
}
