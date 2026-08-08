use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Error, ErrorCode};
use sha2::{Digest, Sha256};

use super::{ensure_real_directory, io_error, read_regular_bounded, sync_directory};

pub(super) const DB_NAME: &str = "index-v4.sqlite3";
pub(super) const INDEX_AUTH_KEY_BYTES: usize = 32;
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY NOT NULL,
  value TEXT NOT NULL,
  mac TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS entities (
  entity_id TEXT PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL,
  provider_namespace TEXT NOT NULL,
  authority_scope TEXT NOT NULL,
  critical INTEGER NOT NULL,
  classification_rank INTEGER NOT NULL,
  labels_folded TEXT NOT NULL,
  materialized_json TEXT NOT NULL,
  mac TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS entities_kind ON entities(kind, entity_id);
CREATE INDEX IF NOT EXISTS entities_provider_scope
  ON entities(provider_namespace, authority_scope, entity_id);
CREATE INDEX IF NOT EXISTS entities_critical ON entities(critical, entity_id);
CREATE TABLE IF NOT EXISTS edges (
  edge_id TEXT PRIMARY KEY NOT NULL,
  from_id TEXT NOT NULL,
  to_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  classification_rank INTEGER NOT NULL,
  materialized_json TEXT NOT NULL,
  mac TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS edges_kind ON edges(kind, edge_id);
CREATE INDEX IF NOT EXISTS edges_from ON edges(from_id, edge_id);
CREATE INDEX IF NOT EXISTS edges_to ON edges(to_id, edge_id);
CREATE TABLE IF NOT EXISTS entity_versions (
  version_key TEXT PRIMARY KEY NOT NULL,
  entity_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  provider_namespace TEXT NOT NULL,
  authority_scope TEXT NOT NULL,
  critical INTEGER NOT NULL,
  classification_rank INTEGER NOT NULL,
  labels_folded TEXT NOT NULL,
  valid_from_ms INTEGER NOT NULL,
  valid_to_ms INTEGER,
  materialized_json TEXT NOT NULL,
  mac TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS entity_versions_query
  ON entity_versions(classification_rank, valid_from_ms, valid_to_ms, version_key);
CREATE TABLE IF NOT EXISTS edge_versions (
  version_key TEXT PRIMARY KEY NOT NULL,
  edge_id TEXT NOT NULL,
  from_id TEXT NOT NULL,
  to_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  classification_rank INTEGER NOT NULL,
  valid_from_ms INTEGER NOT NULL,
  valid_to_ms INTEGER,
  materialized_json TEXT NOT NULL,
  mac TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS edge_versions_query
  ON edge_versions(classification_rank, valid_from_ms, valid_to_ms, version_key);
CREATE TABLE IF NOT EXISTS batches (
  batch_id TEXT PRIMARY KEY NOT NULL,
  event_count INTEGER NOT NULL,
  mac TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS cached_events (
  event_id TEXT PRIMARY KEY NOT NULL,
  batch_id TEXT NOT NULL,
  projection_kind TEXT NOT NULL,
  projection_key TEXT NOT NULL,
  source_position TEXT NOT NULL,
  event_json TEXT NOT NULL,
  active INTEGER NOT NULL,
  mac TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS cached_events_projection
  ON cached_events(projection_kind, projection_key, active, event_id);
CREATE INDEX IF NOT EXISTS cached_events_source_position
  ON cached_events(source_position, event_id);
CREATE TABLE IF NOT EXISTS auxiliary_projection (
  lane TEXT NOT NULL,
  object_id TEXT NOT NULL,
  materialized_json TEXT NOT NULL,
  mac TEXT NOT NULL,
  PRIMARY KEY (lane, object_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS projection_conflicts (
  conflict_key TEXT PRIMARY KEY NOT NULL,
  kind_rank INTEGER NOT NULL,
  locator_a TEXT NOT NULL,
  locator_b TEXT NOT NULL,
  visible_internal INTEGER NOT NULL CHECK (visible_internal IN (0, 1)),
  materialized_json TEXT NOT NULL,
  mac TEXT NOT NULL
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS projection_conflicts_visible_order
  ON projection_conflicts(
    visible_internal,
    kind_rank,
    locator_a COLLATE BINARY,
    locator_b COLLATE BINARY,
    conflict_key COLLATE BINARY
  );
CREATE INDEX IF NOT EXISTS projection_conflicts_locator
  ON projection_conflicts(kind_rank, locator_a, conflict_key);
"#;

pub(super) const COMMITMENT_ENTRIES_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS commitment_entries (
  lane TEXT NOT NULL,
  partition_id INTEGER NOT NULL,
  object_id TEXT NOT NULL,
  value_digest BLOB NOT NULL,
  mac BLOB NOT NULL,
  PRIMARY KEY (lane, partition_id, object_id)
) STRICT, WITHOUT ROWID;
"#;

pub(super) fn open_database(root: &Path) -> Result<Connection, String> {
    let path = root.join(DB_NAME);
    match open_once(&path) {
        Ok(connection) => Ok(connection),
        Err(error) if is_corruption(&error) => {
            quarantine_database(&path)?;
            open_once(&path).map_err(sql_error)
        }
        Err(error) => Err(sql_error(error)),
    }
}

pub(super) fn read_meta(
    connection: &Connection,
    key: &str,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<String, String> {
    let (value, mac): (String, String) = connection
        .query_row("SELECT value, mac FROM meta WHERE key = ?1", [key], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(sql_error)?;
    verify_index_mac(auth_key, "meta", &(key, &value), &mac)?;
    Ok(value)
}

pub(super) fn read_meta_json<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    key: &str,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<T, String> {
    serde_json::from_str(&read_meta(connection, key, auth_key)?).map_err(|error| error.to_string())
}

pub(super) fn write_meta(
    connection: &Connection,
    key: &str,
    value: &str,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<(), String> {
    let mac = index_mac(auth_key, "meta", &(key, value))?;
    connection
        .execute("INSERT INTO meta VALUES (?1, ?2, ?3)", [key, value, &mac])
        .map_err(sql_error)?;
    Ok(())
}

pub(super) fn write_meta_json(
    connection: &Connection,
    key: &str,
    value: &impl serde::Serialize,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<(), String> {
    write_meta(
        connection,
        key,
        &serde_json::to_string(value).map_err(|error| error.to_string())?,
        auth_key,
    )
}

pub(crate) fn load_or_create_index_auth_key(
    root: &Path,
) -> Result<[u8; INDEX_AUTH_KEY_BYTES], String> {
    let private_dir = root.join("private");
    ensure_real_directory(&private_dir)?;
    let path = private_dir.join("index-auth.key");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("Scout index authentication key path is unsafe".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut key = [0_u8; INDEX_AUTH_KEY_BYTES];
            getrandom::fill(&mut key)
                .map_err(|_| "Scout index authentication key generation failed".to_string())?;
            match exec_private_fs::write_private_new(&path, &key) {
                Ok(true) => {
                    sync_directory(&private_dir)?;
                }
                Ok(false) => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        Err(error) => return Err(io_error(error)),
    }
    let bytes = read_regular_bounded(
        &path,
        INDEX_AUTH_KEY_BYTES as u64,
        "index authentication key",
    )?;
    bytes
        .try_into()
        .map_err(|_| "Scout index authentication key has the wrong length".to_string())
}

pub(crate) fn index_mac(
    key: &[u8; INDEX_AUTH_KEY_BYTES],
    domain: &str,
    value: &impl serde::Serialize,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(domain, value)).map_err(|error| error.to_string())?;
    Ok(hmac_sha256(key, &bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn verify_index_mac(
    key: &[u8; INDEX_AUTH_KEY_BYTES],
    domain: &str,
    value: &impl serde::Serialize,
    observed: &str,
) -> Result<(), String> {
    let expected = index_mac(key, domain, value)?;
    let difference = observed
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        });
    if observed.len() != expected.len() || difference != 0 {
        return Err("Scout derived index authentication failed".into());
    }
    Ok(())
}

pub(crate) fn index_mac_bytes(
    key: &[u8; INDEX_AUTH_KEY_BYTES],
    domain: &str,
    value: &impl serde::Serialize,
) -> Result<[u8; 32], String> {
    let bytes = serde_json::to_vec(&(domain, value)).map_err(|error| error.to_string())?;
    Ok(hmac_sha256(key, &bytes))
}

pub(crate) fn verify_index_mac_bytes(
    key: &[u8; INDEX_AUTH_KEY_BYTES],
    domain: &str,
    value: &impl serde::Serialize,
    observed: &[u8],
) -> Result<(), String> {
    let expected = index_mac_bytes(key, domain, value)?;
    let difference = observed
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        });
    if observed.len() != expected.len() || difference != 0 {
        return Err("Scout derived index authentication failed".into());
    }
    Ok(())
}

pub(super) fn sql_error(error: Error) -> String {
    format!("Scout index SQLite: {error}")
}

fn open_once(path: &Path) -> Result<Connection, Error> {
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;",
    )?;
    connection.execute_batch(SCHEMA)?;
    connection.execute_batch(COMMITMENT_ENTRIES_SCHEMA)?;
    Ok(connection)
}

fn is_corruption(error: &Error) -> bool {
    matches!(
        error,
        Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase,
                ..
            },
            _
        )
    )
}

fn quarantine_database(path: &Path) -> Result<(), String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    for source in sqlite_paths(path) {
        if source.exists() {
            let name = source
                .file_name()
                .ok_or_else(|| "SQLite path has no file name".to_string())?
                .to_string_lossy();
            fs::rename(
                &source,
                source.with_file_name(format!("{name}.corrupt-{timestamp}")),
            )
            .map_err(io_error)?;
        }
    }
    Ok(())
}

fn sqlite_paths(path: &Path) -> [PathBuf; 3] {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
}

fn hmac_sha256(key: &[u8; INDEX_AUTH_KEY_BYTES], message: &[u8]) -> [u8; 32] {
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for (index, byte) in key.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}
