use std::path::Path;

use agent_orchestration::EnterpriseId;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use scout_accumulator::PartitionedAccumulatorHead;
use serde::Serialize;

use super::accumulator::{empty_heads, validate_heads};
use super::{
    LedgerAuthorityWork, LedgerHead, LEDGER_AUTHORITY_SCHEMA_VERSION, LEDGER_DATABASE_NAME,
};

mod auth;
mod rows;

pub(super) use auth::{
    auth_mac, load_or_create_auth_key, prepare_root, sha256_hex, validate_hex_digest, verify_mac,
    AUTH_KEY_BYTES,
};
pub(super) use rows::{
    insert_batch, insert_event, read_batch, read_batch_range, read_event,
    read_event_ids_for_first_batch,
};

const HEAD_PREFIX: &str = "ledger-head:";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS ledger_head (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
  schema_version INTEGER NOT NULL,
  enterprise_id TEXT NOT NULL,
  generation INTEGER NOT NULL CHECK (generation >= 0),
  head_id TEXT NOT NULL,
  previous_head_id TEXT,
  trust_chain_digest TEXT NOT NULL,
  batch_count INTEGER NOT NULL CHECK (batch_count >= 0),
  event_count INTEGER NOT NULL CHECK (event_count >= 0),
  batch_accumulator_head_json BLOB NOT NULL,
  event_accumulator_head_json BLOB NOT NULL,
  mac BLOB NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS ledger_head_history (
  head_id TEXT PRIMARY KEY NOT NULL,
  generation INTEGER NOT NULL CHECK (generation >= 0),
  head_json BLOB NOT NULL,
  mac BLOB NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS ledger_head_history_generation
  ON ledger_head_history(generation, head_id);
CREATE TABLE IF NOT EXISTS ledger_batches (
  batch_id TEXT PRIMARY KEY NOT NULL,
  generation INTEGER NOT NULL UNIQUE CHECK (generation > 0),
  envelope_sha256 TEXT NOT NULL,
  event_count INTEGER NOT NULL CHECK (event_count > 0),
  envelope_json BLOB NOT NULL,
  mac BLOB NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS ledger_events (
  event_id TEXT PRIMARY KEY NOT NULL,
  event_sha256 TEXT NOT NULL,
  first_batch_id TEXT NOT NULL,
  event_json BLOB NOT NULL,
  mac BLOB NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS ledger_events_first_batch
  ON ledger_events(first_batch_id, event_id);
CREATE TABLE IF NOT EXISTS ledger_accumulator_nodes (
  lane TEXT NOT NULL,
  partition_id INTEGER NOT NULL CHECK (partition_id >= 0),
  node_digest TEXT NOT NULL,
  node_json BLOB NOT NULL,
  mac BLOB NOT NULL,
  PRIMARY KEY (lane, partition_id, node_digest)
) STRICT, WITHOUT ROWID;
"#;

#[derive(Serialize)]
struct HeadContent<'a> {
    schema_version: u16,
    enterprise_id: &'a EnterpriseId,
    generation: u64,
    previous_head_id: &'a Option<String>,
    trust_chain_digest: &'a str,
    batch_count: u64,
    event_count: u64,
    batch_accumulator: &'a PartitionedAccumulatorHead,
    event_accumulator: &'a PartitionedAccumulatorHead,
}

pub(super) struct LedgerHeadFields {
    pub enterprise_id: EnterpriseId,
    pub generation: u64,
    pub previous_head_id: Option<String>,
    pub trust_chain_digest: String,
    pub batch_count: u64,
    pub event_count: u64,
    pub batch_accumulator: PartitionedAccumulatorHead,
    pub event_accumulator: PartitionedAccumulatorHead,
}

pub(super) fn open_connection(root: &Path) -> Result<Connection, String> {
    let connection =
        Connection::open(root.join(LEDGER_DATABASE_NAME)).map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=DELETE;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

pub(super) fn open_lock(root: &Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("ledger-authority.lock"))
        .map_err(|error| error.to_string())
}

pub(super) fn initialize(
    connection: &mut Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    trust_chain_digest: &str,
) -> Result<(LedgerHead, LedgerAuthorityWork), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let mut work = LedgerAuthorityWork::default();
    let existing = read_head_optional(&transaction, auth_key, &mut work)?;
    let head = if let Some(head) = existing {
        validate_head(&head, enterprise_id, trust_chain_digest)?;
        head
    } else {
        let (batch_accumulator, event_accumulator) = empty_heads(enterprise_id)?;
        let head = make_head(LedgerHeadFields {
            enterprise_id: enterprise_id.clone(),
            generation: 0,
            previous_head_id: None,
            trust_chain_digest: trust_chain_digest.to_owned(),
            batch_count: 0,
            event_count: 0,
            batch_accumulator,
            event_accumulator,
        })?;
        write_head(&transaction, auth_key, &head)?;
        work.head_rows_written += 1;
        head
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok((head, work))
}

pub(super) fn read_head(
    connection: &Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    trust_chain_digest: &str,
    work: &mut LedgerAuthorityWork,
) -> Result<LedgerHead, String> {
    let head = read_head_optional(connection, auth_key, work)?
        .ok_or_else(|| "Scout ledger authority head is missing".to_string())?;
    validate_head(&head, enterprise_id, trust_chain_digest)?;
    Ok(head)
}

fn read_head_optional(
    connection: &Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    work: &mut LedgerAuthorityWork,
) -> Result<Option<LedgerHead>, String> {
    work.head_rows_read += 1;
    let row = connection
        .query_row(
            "SELECT schema_version, enterprise_id, generation, head_id, previous_head_id,
                    trust_chain_digest, batch_count, event_count,
                    batch_accumulator_head_json, event_accumulator_head_json, mac
             FROM ledger_head WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, u16>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, u64>(6)?,
                    row.get::<_, u64>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                    row.get::<_, Vec<u8>>(9)?,
                    row.get::<_, Vec<u8>>(10)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    row.map(
        |(
            schema_version,
            enterprise,
            generation,
            head_id,
            previous_head_id,
            trust_chain_digest,
            batch_count,
            event_count,
            batch_bytes,
            event_bytes,
            observed_mac,
        )| {
            let head = LedgerHead {
                schema_version,
                enterprise_id: EnterpriseId::new(enterprise)?,
                generation,
                head_id,
                previous_head_id,
                trust_chain_digest,
                batch_count,
                event_count,
                batch_accumulator: serde_json::from_slice(&batch_bytes)
                    .map_err(|error| error.to_string())?,
                event_accumulator: serde_json::from_slice(&event_bytes)
                    .map_err(|error| error.to_string())?,
            };
            verify_mac(auth_key, "ledger-head-v1", &head, &observed_mac)?;
            Ok(head)
        },
    )
    .transpose()
}

pub(super) fn make_head(fields: LedgerHeadFields) -> Result<LedgerHead, String> {
    let content = HeadContent {
        schema_version: LEDGER_AUTHORITY_SCHEMA_VERSION,
        enterprise_id: &fields.enterprise_id,
        generation: fields.generation,
        previous_head_id: &fields.previous_head_id,
        trust_chain_digest: &fields.trust_chain_digest,
        batch_count: fields.batch_count,
        event_count: fields.event_count,
        batch_accumulator: &fields.batch_accumulator,
        event_accumulator: &fields.event_accumulator,
    };
    let head_id = format!(
        "{HEAD_PREFIX}{}",
        sha256_hex(&serde_json::to_vec(&content).map_err(|error| error.to_string())?)
    );
    Ok(LedgerHead {
        schema_version: LEDGER_AUTHORITY_SCHEMA_VERSION,
        enterprise_id: fields.enterprise_id,
        generation: fields.generation,
        head_id,
        previous_head_id: fields.previous_head_id,
        trust_chain_digest: fields.trust_chain_digest,
        batch_count: fields.batch_count,
        event_count: fields.event_count,
        batch_accumulator: fields.batch_accumulator,
        event_accumulator: fields.event_accumulator,
    })
}

pub(super) fn write_head(
    transaction: &Transaction<'_>,
    auth_key: &[u8; AUTH_KEY_BYTES],
    head: &LedgerHead,
) -> Result<(), String> {
    let mac = auth_mac(auth_key, "ledger-head-v1", head)?;
    let oldest_recovery_generation = head.generation.saturating_sub(1);
    transaction
        .execute(
            "DELETE FROM ledger_head_history WHERE generation < ?1",
            [oldest_recovery_generation],
        )
        .map_err(|error| error.to_string())?;
    insert_head_history(transaction, auth_key, head)?;
    transaction
        .execute(
            "INSERT INTO ledger_head VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(singleton) DO UPDATE SET
               schema_version=excluded.schema_version, enterprise_id=excluded.enterprise_id,
               generation=excluded.generation, head_id=excluded.head_id,
               previous_head_id=excluded.previous_head_id,
               trust_chain_digest=excluded.trust_chain_digest,
               batch_count=excluded.batch_count, event_count=excluded.event_count,
               batch_accumulator_head_json=excluded.batch_accumulator_head_json,
               event_accumulator_head_json=excluded.event_accumulator_head_json, mac=excluded.mac",
            params![
                head.schema_version,
                head.enterprise_id.as_str(),
                head.generation,
                head.head_id,
                head.previous_head_id,
                head.trust_chain_digest,
                head.batch_count,
                head.event_count,
                serde_json::to_vec(&head.batch_accumulator).map_err(|error| error.to_string())?,
                serde_json::to_vec(&head.event_accumulator).map_err(|error| error.to_string())?,
                mac
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn insert_head_history(
    transaction: &Transaction<'_>,
    auth_key: &[u8; AUTH_KEY_BYTES],
    head: &LedgerHead,
) -> Result<(), String> {
    let head_json = serde_json::to_vec(head).map_err(|error| error.to_string())?;
    let history_mac = auth_mac(
        auth_key,
        "ledger-head-history-v1",
        &(&head.head_id, head.generation, &head_json),
    )?;
    transaction
        .execute(
            "INSERT INTO ledger_head_history VALUES (?1, ?2, ?3, ?4)",
            params![head.head_id, head.generation, head_json, history_mac],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn read_head_history_by_id(
    connection: &Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    trust_chain_digest: &str,
    head_id: &str,
) -> Result<Option<LedgerHead>, String> {
    let row = connection
        .query_row(
            "SELECT generation, head_json, mac
             FROM ledger_head_history WHERE head_id = ?1",
            [head_id],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    row.map(|(generation, head_json, observed_mac)| {
        verify_mac(
            auth_key,
            "ledger-head-history-v1",
            &(head_id, generation, &head_json),
            &observed_mac,
        )?;
        let head: LedgerHead =
            serde_json::from_slice(&head_json).map_err(|error| error.to_string())?;
        if head.head_id != head_id || head.generation != generation {
            return Err("Scout ledger head history identity mismatch".into());
        }
        validate_head(&head, enterprise_id, trust_chain_digest)?;
        Ok(head)
    })
    .transpose()
}

pub(super) fn read_head_history_generation(
    connection: &Connection,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    trust_chain_digest: &str,
    generation: u64,
) -> Result<Vec<LedgerHead>, String> {
    let mut statement = connection
        .prepare(
            "SELECT head_id, head_json, mac
             FROM ledger_head_history WHERE generation = ?1 ORDER BY head_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([generation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut heads = Vec::new();
    for row in rows {
        let (head_id, head_json, observed_mac) = row.map_err(|error| error.to_string())?;
        verify_mac(
            auth_key,
            "ledger-head-history-v1",
            &(&head_id, generation, &head_json),
            &observed_mac,
        )?;
        let head: LedgerHead =
            serde_json::from_slice(&head_json).map_err(|error| error.to_string())?;
        if head.head_id != head_id || head.generation != generation {
            return Err("Scout ledger head history identity mismatch".into());
        }
        validate_head(&head, enterprise_id, trust_chain_digest)?;
        heads.push(head);
    }
    Ok(heads)
}

pub(super) fn validate_head(
    head: &LedgerHead,
    enterprise_id: &EnterpriseId,
    trust_chain_digest: &str,
) -> Result<(), String> {
    if head.schema_version != LEDGER_AUTHORITY_SCHEMA_VERSION {
        return Err("unsupported Scout ledger authority schema".into());
    }
    if head.enterprise_id != *enterprise_id {
        return Err("Scout ledger authority is pinned to another enterprise".into());
    }
    if head.trust_chain_digest != trust_chain_digest {
        return Err("Scout ledger authority trust-chain digest changed".into());
    }
    validate_hex_digest("trust chain", &head.trust_chain_digest)?;
    validate_heads(head)?;
    if head.generation != head.batch_count {
        return Err("Scout ledger generation disagrees with its batch count".into());
    }
    if (head.generation == 0) != head.previous_head_id.is_none() {
        return Err("Scout ledger predecessor shape is invalid".into());
    }
    if let Some(previous) = &head.previous_head_id {
        auth::validate_prefixed_hex("previous ledger head", previous, HEAD_PREFIX)?;
    }
    let expected = make_head(LedgerHeadFields {
        enterprise_id: head.enterprise_id.clone(),
        generation: head.generation,
        previous_head_id: head.previous_head_id.clone(),
        trust_chain_digest: head.trust_chain_digest.clone(),
        batch_count: head.batch_count,
        event_count: head.event_count,
        batch_accumulator: head.batch_accumulator.clone(),
        event_accumulator: head.event_accumulator.clone(),
    })?;
    if expected.head_id != head.head_id {
        return Err("Scout ledger head content digest mismatch".into());
    }
    Ok(())
}
