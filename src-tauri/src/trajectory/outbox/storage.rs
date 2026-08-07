use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Error, ErrorCode};

use super::schema::SCHEMA;

const BACKUP_DIR_NAME: &str = "cloud-history-outbox-backups";
const INCREMENTAL_VACUUM_PAGES: i64 = 4096;

fn initialized_databases() -> &'static Mutex<HashSet<PathBuf>> {
    static INITIALIZED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    INITIALIZED.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn open(path: &Path) -> Result<Connection, String> {
    match open_once(path) {
        Ok(conn) => Ok(conn),
        Err(error) if is_corruption(&error) => {
            let backups = quarantine_corrupt_database(path)
                .map_err(|backup_error| format!("trajectory outbox recovery: {backup_error}"))?;
            tracing::warn!(
                database = %path.display(),
                backup_count = backups.len(),
                %error,
                "quarantined corrupt local cloud-history cache; rebuilding from Clark cloud"
            );
            open_once(path).map_err(sql_error)
        }
        Err(error) => Err(sql_error(error)),
    }
}

fn open_once(path: &Path) -> Result<Connection, Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::ToSqlConversionFailure(error.into()))?;
    }
    let is_new = fs::metadata(path).map_or(true, |metadata| metadata.len() == 0);
    let conn = Connection::open(path)?;
    // Auto-vacuum is a database-header choice and must precede WAL mode and
    // the first table creation or SQLite silently leaves the mode disabled.
    if is_new {
        conn.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")?;
    }
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    // Schema discovery and compatibility ALTERs used to run on every event
    // batch. A streaming session can open this boundary many times per second;
    // initialize each database path once per process instead.
    let mut initialized = initialized_databases()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if is_new || !initialized.contains(path) {
        // Incremental auto-vacuum must be selected before the first table is
        // created. Existing databases keep their current mode and are never
        // subjected to a surprise multi-gigabyte VACUUM on the render path.
        conn.execute_batch(SCHEMA)?;
        let _ = conn.execute(
            "ALTER TABLE trajectory_outbox ADD COLUMN replayable INTEGER NOT NULL DEFAULT 1",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE journal_conversation ADD COLUMN checkpoint_seq INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE journal_conversation ADD COLUMN local_live INTEGER NOT NULL DEFAULT 0",
            [],
        );
        initialized.insert(path.to_path_buf());
    }
    Ok(conn)
}

pub(super) fn reclaim_free_pages(conn: &Connection) -> Result<(), String> {
    let auto_vacuum: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .map_err(sql_error)?;
    if auto_vacuum == 2 {
        conn.execute_batch(&format!(
            "PRAGMA incremental_vacuum({INCREMENTAL_VACUUM_PAGES}); PRAGMA optimize;"
        ))
        .map_err(sql_error)?;
    }
    Ok(())
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

fn quarantine_corrupt_database(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::other(format!("database path has no parent: {}", path.display()))
    })?;
    let backup_root = parent.join(BACKUP_DIR_NAME);
    fs::create_dir_all(&backup_root)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup_dir = (0_u32..)
        .map(|sequence| backup_root.join(format!("sqlite-{timestamp}-{sequence}")))
        .find(|candidate| fs::create_dir(candidate).is_ok())
        .ok_or_else(|| std::io::Error::other("could not allocate outbox backup directory"))?;
    let mut backups = Vec::new();
    for source in sqlite_paths(path) {
        if !source.exists() {
            continue;
        }
        let file_name = source.file_name().ok_or_else(|| {
            std::io::Error::other(format!(
                "database path has no file name: {}",
                source.display()
            ))
        })?;
        let destination = backup_dir.join(file_name);
        fs::rename(&source, &destination)?;
        backups.push(destination);
    }
    if backups.is_empty() {
        let _ = fs::remove_dir(&backup_dir);
        return Err(std::io::Error::other("no corrupt outbox files were found"));
    }
    Ok(backups)
}

fn sqlite_paths(path: &Path) -> Vec<PathBuf> {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let mut shm = path.as_os_str().to_os_string();
    shm.push("-shm");
    vec![path.to_path_buf(), PathBuf::from(wal), PathBuf::from(shm)]
}

pub(super) fn owner_key(scope: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in scope.trim().to_ascii_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(super) fn sql_error(error: Error) -> String {
    format!("trajectory outbox: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_database_uses_incremental_auto_vacuum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outbox.sqlite3");

        let conn = open(&path).unwrap();
        let mode: i64 = conn
            .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, 2);

        drop(conn);
        let reopened = open(&path).unwrap();
        let table_count: i64 = reopened
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'trajectory_outbox'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);
    }

    #[test]
    fn corrupt_database_is_quarantined_and_rebuilt_without_touching_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outbox.sqlite3");
        let sibling = dir.path().join("other.sqlite3");
        fs::write(&path, b"not a sqlite database").unwrap();
        fs::write(&sibling, b"unrelated").unwrap();

        let conn = open(&path).unwrap();
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'trajectory_outbox'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(table_count, 1);
        assert_eq!(fs::read(&sibling).unwrap(), b"unrelated");
        let backup_count = fs::read_dir(dir.path().join(BACKUP_DIR_NAME))
            .unwrap()
            .flat_map(Result::ok)
            .flat_map(|entry| fs::read_dir(entry.path()).into_iter().flatten())
            .flat_map(Result::ok)
            .count();
        assert_eq!(backup_count, 1);
    }
}
