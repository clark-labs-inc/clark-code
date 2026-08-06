use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection, OptionalExtension};
use scout_accumulator::Digest;

use super::{CommitmentWork, EVENT_LANE, PROJECTION_LANE};
use crate::index::database::{
    index_mac_bytes, sql_error, verify_index_mac_bytes, COMMITMENT_ENTRIES_SCHEMA,
    INDEX_AUTH_KEY_BYTES,
};

const EVENT_VALUE: &[u8] = &[];
type StoredValueAndMac = (Vec<u8>, Vec<u8>);

pub(super) fn partition_ids(
    connection: &Connection,
) -> Result<BTreeMap<String, BTreeSet<u16>>, String> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT lane, partition_id FROM commitment_entries
             ORDER BY lane, partition_id",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(sql_error)?;
    let mut partitions = BTreeMap::<String, BTreeSet<u16>>::new();
    for row in rows {
        let (lane, partition) = row.map_err(sql_error)?;
        let partition = u16::try_from(partition)
            .map_err(|_| "Scout commitment partition id is out of range".to_string())?;
        partitions.entry(lane).or_default().insert(partition);
    }
    Ok(partitions)
}

pub(super) fn validate_counts(
    connection: &Connection,
    event_count: u64,
    projection_count: u64,
) -> Result<(), String> {
    let observed_events = lane_count(connection, EVENT_LANE)?;
    let observed_projection = lane_count(connection, PROJECTION_LANE)?;
    if observed_events != event_count || observed_projection != projection_count {
        return Err("Scout compact commitment entry count is inconsistent".into());
    }
    Ok(())
}

pub(super) fn replace_all(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    event_entries: &BTreeMap<u16, BTreeSet<String>>,
    projection_entries: &BTreeMap<u16, BTreeMap<String, Digest>>,
) -> Result<CommitmentWork, String> {
    let mut rows_deleted = ensure_compact_layout(connection)?;
    let existing = existing_keys(connection)?;
    for (lane, partition, object_id) in existing {
        if !expected_contains(
            event_entries,
            projection_entries,
            &lane,
            partition,
            &object_id,
        ) {
            rows_deleted = rows_deleted
                .saturating_add(delete_entry(connection, &lane, partition, &object_id)?);
        }
    }
    let mut rows_written = 0;
    for (partition, entries) in event_entries {
        for object_id in entries {
            rows_written += upsert_entry(
                connection,
                auth_key,
                EVENT_LANE,
                *partition,
                object_id,
                EVENT_VALUE,
            )?;
        }
    }
    for (partition, entries) in projection_entries {
        for (object_id, value_digest) in entries {
            rows_written += upsert_entry(
                connection,
                auth_key,
                PROJECTION_LANE,
                *partition,
                object_id,
                value_digest.as_bytes(),
            )?;
        }
    }
    Ok(CommitmentWork {
        rows_written,
        rows_deleted,
    })
}

pub(super) fn read_event_partition(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    partition: u16,
) -> Result<BTreeSet<String>, String> {
    let rows = read_partition(connection, auth_key, EVENT_LANE, partition)?;
    rows.into_iter()
        .map(|(object_id, value_digest)| {
            if !value_digest.is_empty() {
                return Err("Scout event commitment entry carries a value digest".into());
            }
            Ok(object_id)
        })
        .collect()
}

pub(super) fn read_projection_partition(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    partition: u16,
) -> Result<BTreeMap<String, Digest>, String> {
    read_partition(connection, auth_key, PROJECTION_LANE, partition)?
        .into_iter()
        .map(|(object_id, value_digest)| {
            let bytes: [u8; 32] = value_digest
                .try_into()
                .map_err(|_| "Scout projection commitment digest has the wrong length")?;
            Ok((object_id, Digest::from_bytes(bytes)))
        })
        .collect()
}

pub(super) fn insert_event_entries(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    entries: &BTreeMap<u16, BTreeSet<String>>,
) -> Result<CommitmentWork, String> {
    let mut rows_written = 0;
    for (partition, object_ids) in entries {
        for object_id in object_ids {
            rows_written += write_entry(
                connection,
                auth_key,
                EVENT_LANE,
                *partition,
                object_id,
                EVENT_VALUE,
            )?;
        }
    }
    Ok(CommitmentWork {
        rows_written,
        rows_deleted: 0,
    })
}

pub(super) fn upsert_projection_entries(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    entries: &BTreeMap<u16, BTreeMap<String, Digest>>,
) -> Result<CommitmentWork, String> {
    let mut rows_written = 0;
    for (partition, partition_entries) in entries {
        for (object_id, value_digest) in partition_entries {
            rows_written += upsert_entry(
                connection,
                auth_key,
                PROJECTION_LANE,
                *partition,
                object_id,
                value_digest.as_bytes(),
            )?;
        }
    }
    Ok(CommitmentWork {
        rows_written,
        rows_deleted: 0,
    })
}

pub(super) fn mutate_projection_entries(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    puts: &BTreeMap<u16, BTreeMap<String, Digest>>,
    removals: &BTreeMap<u16, BTreeSet<String>>,
) -> Result<CommitmentWork, String> {
    connection
        .execute_batch("SAVEPOINT scout_projection_mutation_v1")
        .map_err(sql_error)?;
    let result = mutate_projection_entries_inner(connection, auth_key, puts, removals);
    match result {
        Ok(work) => {
            connection
                .execute_batch("RELEASE SAVEPOINT scout_projection_mutation_v1")
                .map_err(sql_error)?;
            Ok(work)
        }
        Err(error) => {
            let rollback = connection.execute_batch(
                "ROLLBACK TO SAVEPOINT scout_projection_mutation_v1;
                 RELEASE SAVEPOINT scout_projection_mutation_v1;",
            );
            match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!(
                    "{error}; Scout projection mutation rollback failed: {}",
                    sql_error(rollback_error)
                )),
            }
        }
    }
}

fn mutate_projection_entries_inner(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    puts: &BTreeMap<u16, BTreeMap<String, Digest>>,
    removals: &BTreeMap<u16, BTreeSet<String>>,
) -> Result<CommitmentWork, String> {
    let mut rows_deleted = 0usize;
    for (partition, object_ids) in removals {
        for object_id in object_ids {
            rows_deleted = rows_deleted.saturating_add(delete_entry(
                connection,
                PROJECTION_LANE,
                i64::from(*partition),
                object_id,
            )?);
        }
    }
    let mut work = upsert_projection_entries(connection, auth_key, puts)?;
    work.rows_deleted = rows_deleted;
    Ok(work)
}

fn read_partition(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    lane: &str,
    partition: u16,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT object_id, value_digest, mac FROM commitment_entries
             WHERE lane = ?1 AND partition_id = ?2 ORDER BY object_id",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![lane, i64::from(partition)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(sql_error)?;
    let mut entries = Vec::new();
    for row in rows {
        let (object_id, value_digest, mac) = row.map_err(sql_error)?;
        verify_index_mac_bytes(
            auth_key,
            "commitment-entry",
            &(
                lane,
                i64::from(partition),
                object_id.as_str(),
                value_digest.as_slice(),
            ),
            &mac,
        )?;
        entries.push((object_id, value_digest));
    }
    Ok(entries)
}

fn ensure_compact_layout(connection: &Connection) -> Result<usize, String> {
    let schema = connection
        .query_row(
            "SELECT sql FROM sqlite_schema
             WHERE type = 'table' AND name = 'commitment_entries'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sql_error)?;
    let mut rows_deleted = 0;
    if schema.as_deref().is_some_and(|sql| {
        !sql.contains("WITHOUT ROWID")
            || !sql.contains("value_digest BLOB")
            || !sql.contains("mac BLOB")
    }) {
        rows_deleted = usize::try_from(table_count(connection)?)
            .map_err(|_| "Scout commitment migration count exceeds usize".to_string())?;
        connection
            .execute_batch("DROP TABLE commitment_entries;")
            .map_err(sql_error)?;
    }
    connection
        .execute_batch("DROP TABLE IF EXISTS commitment_nodes;")
        .map_err(sql_error)?;
    connection
        .execute_batch(COMMITMENT_ENTRIES_SCHEMA)
        .map_err(sql_error)?;
    Ok(rows_deleted)
}

fn existing_keys(connection: &Connection) -> Result<Vec<(String, i64, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT lane, partition_id, object_id FROM commitment_entries
             ORDER BY lane, partition_id, object_id",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    Ok(rows)
}

fn expected_contains(
    event_entries: &BTreeMap<u16, BTreeSet<String>>,
    projection_entries: &BTreeMap<u16, BTreeMap<String, Digest>>,
    lane: &str,
    partition: i64,
    object_id: &str,
) -> bool {
    let Ok(partition) = u16::try_from(partition) else {
        return false;
    };
    match lane {
        EVENT_LANE => event_entries
            .get(&partition)
            .is_some_and(|entries| entries.contains(object_id)),
        PROJECTION_LANE => projection_entries
            .get(&partition)
            .is_some_and(|entries| entries.contains_key(object_id)),
        _ => false,
    }
}

fn delete_entry(
    connection: &Connection,
    lane: &str,
    partition: i64,
    object_id: &str,
) -> Result<usize, String> {
    connection
        .execute(
            "DELETE FROM commitment_entries
             WHERE lane = ?1 AND partition_id = ?2 AND object_id = ?3",
            params![lane, partition, object_id],
        )
        .map_err(sql_error)
}

fn write_entry(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    lane: &str,
    partition: u16,
    object_id: &str,
    value_digest: &[u8],
) -> Result<usize, String> {
    let mac = entry_mac(auth_key, lane, partition, object_id, value_digest)?;
    let inserted = connection
        .execute(
            "INSERT INTO commitment_entries VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(lane, partition_id, object_id) DO NOTHING",
            params![
                lane,
                i64::from(partition),
                object_id,
                value_digest,
                mac.as_slice()
            ],
        )
        .map_err(sql_error)?;
    let observed = read_entry(connection, lane, partition, object_id)?;
    if observed.as_ref() != Some(&(value_digest.to_owned(), mac.to_vec())) {
        return Err("Scout commitment entry identity collision".into());
    }
    Ok(inserted)
}

fn upsert_entry(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    lane: &str,
    partition: u16,
    object_id: &str,
    value_digest: &[u8],
) -> Result<usize, String> {
    let mac = entry_mac(auth_key, lane, partition, object_id, value_digest)?;
    connection
        .execute(
            "INSERT INTO commitment_entries VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(lane, partition_id, object_id) DO UPDATE SET
               value_digest=excluded.value_digest, mac=excluded.mac
             WHERE commitment_entries.value_digest != excluded.value_digest
                OR commitment_entries.mac != excluded.mac",
            params![
                lane,
                i64::from(partition),
                object_id,
                value_digest,
                mac.as_slice()
            ],
        )
        .map_err(sql_error)
}

fn read_entry(
    connection: &Connection,
    lane: &str,
    partition: u16,
    object_id: &str,
) -> Result<Option<StoredValueAndMac>, String> {
    connection
        .query_row(
            "SELECT value_digest, mac FROM commitment_entries
             WHERE lane = ?1 AND partition_id = ?2 AND object_id = ?3",
            params![lane, i64::from(partition), object_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(sql_error)
}

fn entry_mac(
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    lane: &str,
    partition: u16,
    object_id: &str,
    value_digest: &[u8],
) -> Result<[u8; 32], String> {
    index_mac_bytes(
        auth_key,
        "commitment-entry",
        &(lane, i64::from(partition), object_id, value_digest),
    )
}

fn lane_count(connection: &Connection, lane: &str) -> Result<u64, String> {
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM commitment_entries WHERE lane = ?1",
            [lane],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sql_error)?;
    u64::try_from(count).map_err(|_| "Scout commitment entry count is negative".into())
}

fn table_count(connection: &Connection) -> Result<u64, String> {
    let count = connection
        .query_row("SELECT COUNT(*) FROM commitment_entries", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(sql_error)?;
    u64::try_from(count).map_err(|_| "Scout commitment entry count is negative".into())
}
