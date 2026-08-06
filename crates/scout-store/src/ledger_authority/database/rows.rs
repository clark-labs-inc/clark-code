use agent_orchestration::{EnterpriseId, EnterpriseSignedBatch};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::auth::{auth_mac, sha256_hex, verify_mac, AUTH_KEY_BYTES};
use crate::ledger_authority::{LedgerAuthorityWork, LedgerGenerationEnvelope};
use std::collections::BTreeSet;

pub(in crate::ledger_authority) struct StoredBatch {
    pub generation: u64,
    pub envelope_sha256: String,
    pub event_count: u64,
    pub envelope_json: Vec<u8>,
}

impl StoredBatch {
    pub fn decode(self) -> Result<EnterpriseSignedBatch, String> {
        let envelope: EnterpriseSignedBatch =
            serde_json::from_slice(&self.envelope_json).map_err(|error| error.to_string())?;
        envelope.batch.validate()?;
        if sha256_hex(&self.envelope_json) != self.envelope_sha256
            || envelope.batch.events.len() as u64 != self.event_count
        {
            return Err("authenticated ledger batch content is inconsistent".into());
        }
        Ok(envelope)
    }
}

pub(in crate::ledger_authority) struct StoredEvent {
    pub event_sha256: String,
    pub first_batch_id: String,
    pub event_json: Vec<u8>,
}

pub(in crate::ledger_authority) fn read_batch(
    connection: &Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    batch_id: &str,
    work: &mut LedgerAuthorityWork,
) -> Result<Option<StoredBatch>, String> {
    work.batch_lookups += 1;
    let row = connection
        .query_row(
            "SELECT generation, envelope_sha256, event_count, envelope_json, mac
             FROM ledger_batches WHERE batch_id = ?1",
            [batch_id],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    row.map(
        |(generation, envelope_sha256, event_count, envelope_json, observed_mac)| {
            verify_mac(
                auth_key,
                "ledger-batch-v1",
                &(
                    enterprise_id,
                    batch_id,
                    generation,
                    &envelope_sha256,
                    event_count,
                    &envelope_json,
                ),
                &observed_mac,
            )?;
            if sha256_hex(&envelope_json) != envelope_sha256 {
                return Err("authenticated ledger batch digest mismatch".into());
            }
            Ok(StoredBatch {
                generation,
                envelope_sha256,
                event_count,
                envelope_json,
            })
        },
    )
    .transpose()
}

pub(in crate::ledger_authority) fn insert_batch(
    transaction: &Transaction<'_>,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    batch_id: &str,
    generation: u64,
    event_count: u64,
    envelope_json: &[u8],
) -> Result<(), String> {
    let digest = sha256_hex(envelope_json);
    let mac = auth_mac(
        auth_key,
        "ledger-batch-v1",
        &(
            enterprise_id,
            batch_id,
            generation,
            &digest,
            event_count,
            envelope_json,
        ),
    )?;
    transaction
        .execute(
            "INSERT INTO ledger_batches VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                batch_id,
                generation,
                digest,
                event_count,
                envelope_json,
                mac
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(in crate::ledger_authority) fn read_event(
    connection: &Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    event_id: &str,
    work: &mut LedgerAuthorityWork,
) -> Result<Option<StoredEvent>, String> {
    work.event_lookups += 1;
    let row = connection
        .query_row(
            "SELECT event_sha256, first_batch_id, event_json, mac
             FROM ledger_events WHERE event_id = ?1",
            [event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    row.map(|(event_sha256, first_batch_id, event_json, observed_mac)| {
        verify_mac(
            auth_key,
            "ledger-event-v1",
            &(
                enterprise_id,
                event_id,
                &event_sha256,
                &first_batch_id,
                &event_json,
            ),
            &observed_mac,
        )?;
        if sha256_hex(&event_json) != event_sha256 {
            return Err("authenticated ledger event digest mismatch".into());
        }
        Ok(StoredEvent {
            event_sha256,
            first_batch_id,
            event_json,
        })
    })
    .transpose()
}

pub(in crate::ledger_authority) fn insert_event(
    transaction: &Transaction<'_>,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    event_id: &str,
    batch_id: &str,
    event_json: &[u8],
) -> Result<(), String> {
    let digest = sha256_hex(event_json);
    let mac = auth_mac(
        auth_key,
        "ledger-event-v1",
        &(enterprise_id, event_id, &digest, batch_id, event_json),
    )?;
    transaction
        .execute(
            "INSERT INTO ledger_events VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event_id, digest, batch_id, event_json, mac],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(in crate::ledger_authority) fn read_batch_range(
    connection: &Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    first_generation: u64,
    last_generation: u64,
    work: &mut LedgerAuthorityWork,
) -> Result<Vec<LedgerGenerationEnvelope>, String> {
    if first_generation > last_generation {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT batch_id, generation, envelope_sha256, event_count, envelope_json, mac
             FROM ledger_batches
             WHERE generation BETWEEN ?1 AND ?2
             ORDER BY generation",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![first_generation, last_generation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut envelopes = Vec::new();
    let mut expected_generation = first_generation;
    for row in rows {
        let (batch_id, generation, digest, event_count, bytes, observed_mac) =
            row.map_err(|error| error.to_string())?;
        if generation != expected_generation {
            return Err("Scout ledger generation range contains a gap".into());
        }
        verify_mac(
            auth_key,
            "ledger-batch-v1",
            &(
                enterprise_id,
                batch_id.as_str(),
                generation,
                &digest,
                event_count,
                &bytes,
            ),
            &observed_mac,
        )?;
        let byte_count = bytes.len();
        let envelope = StoredBatch {
            generation,
            envelope_sha256: digest,
            event_count,
            envelope_json: bytes,
        }
        .decode()?;
        if envelope.batch.enterprise_id != *enterprise_id
            || envelope.batch.batch_id.as_str() != batch_id
        {
            return Err("authenticated ledger range row has inconsistent identity".into());
        }
        work.envelope_rows_read += 1;
        work.envelope_bytes_read = work.envelope_bytes_read.saturating_add(byte_count);
        envelopes.push(LedgerGenerationEnvelope {
            generation,
            envelope,
        });
        expected_generation = expected_generation
            .checked_add(1)
            .ok_or_else(|| "Scout ledger generation overflow".to_string())?;
    }
    if expected_generation != last_generation.saturating_add(1) {
        return Err("Scout ledger generation range is incomplete".into());
    }
    Ok(envelopes)
}

pub(in crate::ledger_authority) fn read_event_ids_for_first_batch(
    connection: &Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    batch_id: &str,
    work: &mut LedgerAuthorityWork,
) -> Result<BTreeSet<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT event_id FROM ledger_events
             WHERE first_batch_id = ?1 ORDER BY event_id",
        )
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map([batch_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut authenticated = BTreeSet::new();
    for event_id in ids {
        let event_id = event_id.map_err(|error| error.to_string())?;
        let event = read_event(connection, auth_key, enterprise_id, &event_id, work)?
            .ok_or_else(|| "Scout ledger event disappeared during recovery".to_string())?;
        if event.first_batch_id != batch_id {
            return Err("Scout ledger first-batch event index is inconsistent".into());
        }
        authenticated.insert(event_id);
    }
    Ok(authenticated)
}
