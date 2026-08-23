use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Response;

const RECEIPT_VERSION: u32 = 2;
const MAX_RECEIPT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECEIPT_STORE_BYTES: u64 = 512 * 1024 * 1024;
/// How long a settled receipt stays replayable once capacity is needed.
///
/// A receipt's purpose is to answer a RETRY of the same request id — which
/// happens within a reconnect window measured in seconds, not days. Under
/// capacity pressure, settled receipts older than this are evicted to make
/// room; in-progress receipts are never touched (they detect duplicate
/// concurrent execution, and evicting one would license exactly that).
///
/// Without eviction the store only grew — request ids are fresh UUIDs — and a
/// worker that reached the 512 MiB cap answered `receipt_capacity_exhausted`
/// to every request from then on: permanently bricked until someone deleted
/// `request-receipts/` by hand.
const SETTLED_RECEIPT_RETENTION: std::time::Duration = std::time::Duration::from_secs(60 * 60);

#[derive(Clone, Debug)]
pub(crate) struct IdempotencyStore {
    root: PathBuf,
    capacity_bytes: u64,
    state: Arc<Mutex<StoreState>>,
}

#[derive(Debug, Default)]
struct StoreState {
    usage_bytes: Option<u64>,
}

#[derive(Debug)]
pub(crate) enum Reservation {
    Fresh {
        request_hash: String,
    },
    Replay {
        progress: Vec<Response>,
        terminal: Response,
    },
    Ambiguous,
    Conflict,
    CapacityExhausted,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum ReceiptEntry {
    Started {
        version: u32,
        request_hash: String,
    },
    Completed {
        version: u32,
        request_hash: String,
        progress: Vec<Response>,
        terminal: Response,
    },
    ReplayUnavailable {
        version: u32,
        request_hash: String,
        reason: ReplayUnavailableReason,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReplayUnavailableReason {
    CaptureLimit,
    StoreCapacity,
}

impl IdempotencyStore {
    pub(crate) fn new(trajectory_root: &Path) -> Self {
        Self {
            root: trajectory_root.join("request-receipts"),
            capacity_bytes: MAX_RECEIPT_STORE_BYTES,
            state: Arc::new(Mutex::new(StoreState::default())),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_capacity(trajectory_root: &Path, capacity_bytes: u64) -> Self {
        Self {
            root: trajectory_root.join("request-receipts"),
            capacity_bytes,
            state: Arc::new(Mutex::new(StoreState::default())),
        }
    }

    pub(crate) async fn reserve(
        &self,
        request_id: &str,
        request: &serde_json::Value,
    ) -> Result<Reservation, String> {
        let root = self.root.clone();
        let capacity_bytes = self.capacity_bytes;
        let state = self.state.clone();
        let request_id = request_id.to_string();
        let request = request.clone();
        tokio::task::spawn_blocking(move || {
            let mut state = state
                .lock()
                .map_err(|_| "remote request receipt state lock failed".to_string())?;
            reserve_blocking(&root, capacity_bytes, &mut state, &request_id, &request)
        })
        .await
        .map_err(|_| "remote request receipt task failed".to_string())?
    }

    pub(crate) async fn complete(
        &self,
        request_id: &str,
        request_hash: String,
        progress: Result<Vec<Response>, ()>,
        terminal: Response,
    ) -> Result<(), String> {
        let root = self.root.clone();
        let capacity_bytes = self.capacity_bytes;
        let state = self.state.clone();
        let request_id = request_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut state = state
                .lock()
                .map_err(|_| "remote request receipt state lock failed".to_string())?;
            complete_blocking(
                &root,
                capacity_bytes,
                &mut state,
                &request_id,
                request_hash,
                progress,
                terminal,
            )
        })
        .await
        .map_err(|_| "remote request receipt task failed".to_string())?
    }
}

fn reserve_blocking(
    root: &Path,
    capacity_bytes: u64,
    state: &mut StoreState,
    request_id: &str,
    request: &serde_json::Value,
) -> Result<Reservation, String> {
    private_directory(root)?;
    let usage_bytes = store_usage(root, state)?;
    let path = receipt_path(root, request_id);
    let request_hash = hash_json(request)?;
    if path.exists() {
        return existing_reservation(&path, &request_hash);
    }
    let started = ReceiptEntry::Started {
        version: RECEIPT_VERSION,
        request_hash: request_hash.clone(),
    };
    let mut line = serde_json::to_vec(&started).map_err(|error| error.to_string())?;
    line.push(b'\n');
    let mut usage_bytes = usage_bytes;
    if usage_bytes.saturating_add(line.len() as u64) > capacity_bytes {
        usage_bytes = evict_settled_receipts(root, state)?;
        if usage_bytes.saturating_add(line.len() as u64) > capacity_bytes {
            // Everything left is in-progress or inside the replay window;
            // refusing is the only honest answer now.
            return Ok(Reservation::CapacityExhausted);
        }
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(&line).and_then(|()| file.sync_all()) {
                state.usage_bytes = None;
                return Err(error.to_string());
            }
            if let Err(error) = sync_directory(root) {
                state.usage_bytes = None;
                return Err(error);
            }
            state.usage_bytes = Some(usage_bytes + line.len() as u64);
            Ok(Reservation::Fresh { request_hash })
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            existing_reservation(&path, &request_hash)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn existing_reservation(path: &Path, request_hash: &str) -> Result<Reservation, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_RECEIPT_BYTES {
        return Ok(Reservation::Ambiguous);
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|error| error.to_string())?;
    let mut lines = contents.lines();
    let Some(first) = lines.next() else {
        return Ok(Reservation::Ambiguous);
    };
    let Ok(ReceiptEntry::Started {
        version,
        request_hash: stored_hash,
    }) = serde_json::from_str::<ReceiptEntry>(first)
    else {
        return Ok(Reservation::Ambiguous);
    };
    if version != RECEIPT_VERSION {
        return Ok(Reservation::Ambiguous);
    }
    if stored_hash != request_hash {
        return Ok(Reservation::Conflict);
    }
    let Some(second) = lines.next() else {
        return Ok(Reservation::Ambiguous);
    };
    if lines.next().is_some() {
        return Ok(Reservation::Ambiguous);
    }
    let Ok(ReceiptEntry::Completed {
        version,
        request_hash: completed_hash,
        progress,
        terminal,
    }) = serde_json::from_str::<ReceiptEntry>(second)
    else {
        return Ok(Reservation::Ambiguous);
    };
    if version != RECEIPT_VERSION || completed_hash != request_hash {
        return Ok(Reservation::Ambiguous);
    }
    Ok(Reservation::Replay { progress, terminal })
}

fn complete_blocking(
    root: &Path,
    capacity_bytes: u64,
    state: &mut StoreState,
    request_id: &str,
    request_hash: String,
    progress: Result<Vec<Response>, ()>,
    terminal: Response,
) -> Result<(), String> {
    let path = receipt_path(root, request_id);
    let usage_bytes = store_usage(root, state)?;
    let entry = match progress {
        Ok(progress) => ReceiptEntry::Completed {
            version: RECEIPT_VERSION,
            request_hash: request_hash.clone(),
            progress,
            terminal,
        },
        Err(()) => ReceiptEntry::ReplayUnavailable {
            version: RECEIPT_VERSION,
            request_hash: request_hash.clone(),
            reason: ReplayUnavailableReason::CaptureLimit,
        },
    };
    let mut line = serde_json::to_vec(&entry).map_err(|error| error.to_string())?;
    line.push(b'\n');
    let existing_bytes = std::fs::metadata(&path)
        .map_err(|error| error.to_string())?
        .len();
    if existing_bytes.saturating_add(line.len() as u64) > MAX_RECEIPT_BYTES
        || usage_bytes.saturating_add(line.len() as u64) > capacity_bytes
    {
        let unavailable = ReceiptEntry::ReplayUnavailable {
            version: RECEIPT_VERSION,
            request_hash,
            reason: ReplayUnavailableReason::StoreCapacity,
        };
        line = serde_json::to_vec(&unavailable).map_err(|error| error.to_string())?;
        line.push(b'\n');
        if existing_bytes.saturating_add(line.len() as u64) > MAX_RECEIPT_BYTES
            || usage_bytes.saturating_add(line.len() as u64) > capacity_bytes
        {
            return Ok(());
        }
    }
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    if let Err(error) = file.write_all(&line).and_then(|()| file.sync_all()) {
        state.usage_bytes = None;
        return Err(error.to_string());
    }
    state.usage_bytes = Some(usage_bytes + line.len() as u64);
    Ok(())
}

/// Delete settled receipts older than the replay window and return the new
/// usage. Runs only under capacity pressure, so the common path never scans.
fn evict_settled_receipts(root: &Path, state: &mut StoreState) -> Result<u64, String> {
    let now = std::time::SystemTime::now();
    let mut usage_bytes = 0_u64;
    for entry in std::fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        let expired = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > SETTLED_RECEIPT_RETENTION);
        if expired && receipt_is_settled(&path) {
            match std::fs::remove_file(&path) {
                Ok(()) => continue,
                // Raced with another remover; either way it no longer counts.
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    state.usage_bytes = None;
                    return Err(error.to_string());
                }
            }
        }
        usage_bytes = usage_bytes.saturating_add(metadata.len());
    }
    state.usage_bytes = Some(usage_bytes);
    Ok(usage_bytes)
}

/// True only when the receipt provably reached a terminal entry. Unreadable or
/// ambiguous files are conservatively treated as live and kept.
fn receipt_is_settled(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    contents.lines().nth(1).is_some_and(|second| {
        matches!(
            serde_json::from_str::<ReceiptEntry>(second),
            Ok(ReceiptEntry::Completed { .. } | ReceiptEntry::ReplayUnavailable { .. })
        )
    })
}

fn store_usage(root: &Path, state: &mut StoreState) -> Result<u64, String> {
    if let Some(usage_bytes) = state.usage_bytes {
        return Ok(usage_bytes);
    }
    let mut usage_bytes = 0_u64;
    for entry in std::fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        usage_bytes =
            usage_bytes.saturating_add(entry.metadata().map_err(|error| error.to_string())?.len());
    }
    state.usage_bytes = Some(usage_bytes);
    Ok(usage_bytes)
}

fn receipt_path(root: &Path, request_id: &str) -> PathBuf {
    let mut digest = Sha256::new();
    digest.update(request_id.as_bytes());
    root.join(format!("{:x}.jsonl", digest.finalize()))
}

fn hash_json(value: &serde_json::Value) -> Result<String, String> {
    let encoded = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn private_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Age a receipt file past the replay retention window.
    fn expire(root: &Path, request_id: &str) {
        let path = receipt_path(&root.join("request-receipts"), request_id);
        let stale = std::time::SystemTime::now() - SETTLED_RECEIPT_RETENTION * 2;
        let file = File::options().append(true).open(&path).unwrap();
        file.set_modified(stale).unwrap();
    }

    /// Bytes currently on disk under the store root.
    fn usage(root: &Path) -> u64 {
        std::fs::read_dir(root.join("request-receipts"))
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum()
    }

    #[tokio::test]
    async fn a_full_store_evicts_expired_settled_receipts_and_keeps_serving() {
        let root = tempfile::tempdir().unwrap();
        let store = IdempotencyStore::with_capacity(root.path(), 10_000);

        let request = json!({"work": 1});
        let Reservation::Fresh { request_hash } =
            store.reserve("request-1", &request).await.unwrap()
        else {
            panic!("first reservation must fit");
        };
        store
            .complete(
                "request-1",
                request_hash,
                Ok(Vec::new()),
                Response::result(Some("request-1".into()), "done", json!({})),
            )
            .await
            .unwrap();

        // A second store over the same root, sized so nothing further fits
        // while the settled receipt is present.
        let full = IdempotencyStore::with_capacity(root.path(), usage(root.path()) + 4);

        // Within the replay window the settled receipt is protected: the next
        // reservation that does not fit is refused, not served by eviction.
        assert!(matches!(
            full.reserve("request-2", &json!({"work": 2}))
                .await
                .unwrap(),
            Reservation::CapacityExhausted
        ));

        // Past the window it is reclaimable, and the store keeps serving
        // instead of staying bricked until manual cleanup.
        expire(root.path(), "request-1");
        assert!(matches!(
            full.reserve("request-2", &json!({"work": 2}))
                .await
                .unwrap(),
            Reservation::Fresh { .. }
        ));
    }

    #[tokio::test]
    async fn eviction_never_touches_an_in_progress_receipt() {
        let root = tempfile::tempdir().unwrap();
        let store = IdempotencyStore::with_capacity(root.path(), 10_000);

        // In progress: reserved, never completed. Evicting it would license
        // duplicate concurrent execution of the same request id.
        assert!(matches!(
            store
                .reserve("request-1", &json!({"work": 1}))
                .await
                .unwrap(),
            Reservation::Fresh { .. }
        ));
        expire(root.path(), "request-1");

        let full = IdempotencyStore::with_capacity(root.path(), usage(root.path()) + 4);
        assert!(matches!(
            full.reserve("request-2", &json!({"work": 2}))
                .await
                .unwrap(),
            Reservation::CapacityExhausted
        ));
        // The aged in-progress receipt still answers as a duplicate.
        assert!(matches!(
            full.reserve("request-1", &json!({"work": 1}))
                .await
                .unwrap(),
            Reservation::Ambiguous
        ));
    }

    #[tokio::test]
    async fn completion_that_exceeds_store_budget_remains_ambiguous() {
        let root = tempfile::tempdir().unwrap();
        let store = IdempotencyStore::with_capacity(root.path(), 220);
        let request = json!({"work": 1});
        let Reservation::Fresh { request_hash } =
            store.reserve("request-1", &request).await.unwrap()
        else {
            panic!("small started receipt must fit");
        };
        store
            .complete(
                "request-1",
                request_hash,
                Ok(Vec::new()),
                Response::result(
                    Some("request-1".into()),
                    "large",
                    json!({"payload": "x".repeat(1_024)}),
                ),
            )
            .await
            .unwrap();

        let bytes = std::fs::read_dir(root.path().join("request-receipts"))
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum::<u64>();
        assert!(bytes <= 220);
        assert!(matches!(
            store.reserve("request-1", &request).await.unwrap(),
            Reservation::Ambiguous
        ));
    }
}
