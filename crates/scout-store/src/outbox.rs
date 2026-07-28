use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use agent_orchestration::{EnterpriseBatchBundle, EnterpriseBatchId, EnterpriseId};
use base64::Engine;

use crate::index::{ensure_real_directory, read_pinned_chain, sync_directory};
use crate::model::{
    OutboxEntry, OutboxPage, OutboxPageCursor, OutboxResolution, OutboxState, OutboxStateFilter,
};

const MAX_OUTBOX_STATE_BYTES: u64 = 64 * 1024;
const MAX_OUTBOX_PAGE_ENTRIES: usize = 1_000;
const MAX_OUTBOX_FILES: usize = 100_000;
const OUTBOX_CURSOR_VERSION: u16 = 1;

pub(super) fn enqueue(
    root: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
) -> Result<(OutboxEntry, bool), String> {
    verify_signed_batch_reference(root, enterprise_id, batch_id)?;
    let directory = outbox_directory(root)?;
    if let Some(entry) = read_entry(&directory, enterprise_id, batch_id)? {
        return Ok((entry, true));
    }
    let entry = OutboxEntry {
        enterprise_id: enterprise_id.clone(),
        batch_id: batch_id.clone(),
        revision: 1,
        state: OutboxState::Pending,
    };
    write_entry(&directory, &entry)?;
    Ok((entry, false))
}

pub(super) fn begin_delivery(
    root: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
    attempt_id: &str,
    previous_attempt_id: Option<&str>,
) -> Result<(OutboxEntry, bool), String> {
    validate_digest_reference("outbox attempt", attempt_id, "outbox-attempt:")?;
    if let Some(previous) = previous_attempt_id {
        validate_digest_reference("previous outbox attempt", previous, "outbox-attempt:")?;
    }
    verify_signed_batch_reference(root, enterprise_id, batch_id)?;
    let directory = outbox_directory(root)?;
    let mut entry = read_entry(&directory, enterprise_id, batch_id)?
        .ok_or_else(|| "central ingestion outbox entry has not been enqueued".to_string())?;
    match &entry.state {
        OutboxState::Pending => {
            if previous_attempt_id.is_some() {
                return Err("pending outbox delivery has no previous attempt".into());
            }
        }
        OutboxState::InFlight {
            attempt_id: current,
        } if current == attempt_id => {
            if previous_attempt_id.is_none() || previous_attempt_id == Some(current.as_str()) {
                return Ok((entry, true));
            }
            return Err("idempotent outbox retry names a conflicting previous attempt".into());
        }
        OutboxState::InFlight {
            attempt_id: current,
        } => {
            if previous_attempt_id != Some(current.as_str()) {
                return Err(
                    "replacing an in-flight outbox delivery requires its exact attempt id".into(),
                );
            }
        }
        OutboxState::Acked { .. } | OutboxState::Rejected { .. } => {
            return Err(
                "terminal central ingestion outbox entry cannot begin another delivery".into(),
            )
        }
    }
    entry.revision = next_revision(entry.revision)?;
    entry.state = OutboxState::InFlight {
        attempt_id: attempt_id.to_owned(),
    };
    write_entry(&directory, &entry)?;
    Ok((entry, false))
}

pub(super) fn resolve_delivery(
    root: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
    attempt_id: &str,
    resolution: OutboxResolution,
    resolution_id: &str,
) -> Result<(OutboxEntry, bool), String> {
    validate_digest_reference("outbox attempt", attempt_id, "outbox-attempt:")?;
    validate_digest_reference(
        "central ingestion resolution",
        resolution_id,
        "central-ingestion:",
    )?;
    verify_signed_batch_reference(root, enterprise_id, batch_id)?;
    let directory = outbox_directory(root)?;
    let mut entry = read_entry(&directory, enterprise_id, batch_id)?
        .ok_or_else(|| "central ingestion outbox entry has not been enqueued".to_string())?;
    match &entry.state {
        OutboxState::InFlight {
            attempt_id: current,
        } if current == attempt_id => {}
        OutboxState::InFlight { .. } => {
            return Err("central ingestion resolution names a stale delivery attempt".into())
        }
        OutboxState::Acked {
            attempt_id: current_attempt,
            resolution_id: current_resolution,
        } => {
            if resolution == OutboxResolution::Acked
                && current_attempt == attempt_id
                && current_resolution == resolution_id
            {
                return Ok((entry, true));
            }
            return Err("conflicting central ingestion acknowledgment".into());
        }
        OutboxState::Rejected {
            attempt_id: current_attempt,
            resolution_id: current_resolution,
        } => {
            if resolution == OutboxResolution::Rejected
                && current_attempt == attempt_id
                && current_resolution == resolution_id
            {
                return Ok((entry, true));
            }
            return Err("conflicting central ingestion acknowledgment".into());
        }
        OutboxState::Pending => {
            return Err("pending outbox delivery must become in-flight before resolution".into())
        }
    }
    entry.revision = next_revision(entry.revision)?;
    entry.state = match resolution {
        OutboxResolution::Acked => OutboxState::Acked {
            attempt_id: attempt_id.to_owned(),
            resolution_id: resolution_id.to_owned(),
        },
        OutboxResolution::Rejected => OutboxState::Rejected {
            attempt_id: attempt_id.to_owned(),
            resolution_id: resolution_id.to_owned(),
        },
    };
    write_entry(&directory, &entry)?;
    Ok((entry, false))
}

pub(super) fn status(
    root: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
) -> Result<Option<OutboxEntry>, String> {
    batch_digest(batch_id)?;
    let directory = outbox_directory(root)?;
    let entry = read_entry(&directory, enterprise_id, batch_id)?;
    if entry.is_some() {
        verify_signed_batch_reference(root, enterprise_id, batch_id)?;
    }
    Ok(entry)
}

pub(super) fn list(
    root: &Path,
    enterprise_id: &EnterpriseId,
    filter: OutboxStateFilter,
    cursor: Option<&str>,
    limit: usize,
) -> Result<OutboxPage, String> {
    validate_list_limit(limit)?;
    let last_batch_id = decode_list_cursor(cursor, enterprise_id, filter)?;
    let directory = outbox_directory(root)?;
    let candidates = scan_entry_paths(&directory)?;
    let mut entries = Vec::with_capacity(limit + 1);
    for (batch_id, path) in candidates {
        if batch_id.as_str() <= last_batch_id.as_str() {
            continue;
        }
        let entry = read_entry_at(&path, &batch_id)?;
        if entry.enterprise_id != *enterprise_id || !filter.matches(&entry.state) {
            continue;
        }
        entries.push(entry);
        if entries.len() > limit {
            break;
        }
    }
    let has_more = entries.len() > limit;
    entries.truncate(limit);
    let next_cursor = has_more
        .then(|| entries.last())
        .flatten()
        .map(|entry| encode_list_cursor(enterprise_id, filter, &entry.batch_id))
        .transpose()?;
    Ok(OutboxPage {
        entries,
        next_cursor,
    })
}

fn verify_signed_batch_reference(
    root: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
) -> Result<(), String> {
    delivery_bundle(root, enterprise_id, batch_id).map(|_| ())
}

pub(super) fn delivery_bundle(
    root: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
) -> Result<EnterpriseBatchBundle, String> {
    batch_digest(batch_id)?;
    let ledger = crate::index::ledger_authority::open(root, enterprise_id)?;
    let envelope = ledger
        .authority
        .read_envelope(batch_id)?
        .envelope
        .ok_or_else(|| {
            "outbox batch reference is missing from the authenticated ledger".to_string()
        })?;
    let (chain, _) = read_pinned_chain(root, enterprise_id)?;
    let verified = chain.verify_signed_batch(envelope)?;
    if &verified.batch().enterprise_id != enterprise_id || &verified.batch().batch_id != batch_id {
        return Err("outbox batch reference does not match the stored signed batch".into());
    }
    Ok(EnterpriseBatchBundle {
        trust_chain: chain,
        signed_batch: verified.into_envelope(),
    })
}

fn read_entry(
    directory: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
) -> Result<Option<OutboxEntry>, String> {
    let path = entry_path(directory, batch_id)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
        Ok(_) => {
            let entry = read_entry_at(&path, batch_id)?;
            validate_entry(&entry, enterprise_id, batch_id)?;
            Ok(Some(entry))
        }
    }
}

fn read_entry_at(path: &Path, batch_id: &EnterpriseBatchId) -> Result<OutboxEntry, String> {
    let bytes = read_outbox_state(path)?;
    let entry: OutboxEntry = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid central ingestion outbox state: {error}"))?;
    validate_stored_entry(&entry, batch_id)?;
    Ok(entry)
}

fn validate_entry(
    entry: &OutboxEntry,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
) -> Result<(), String> {
    validate_stored_entry(entry, batch_id)?;
    if &entry.enterprise_id != enterprise_id {
        return Err("central ingestion outbox state does not match its target reference".into());
    }
    Ok(())
}

fn validate_stored_entry(entry: &OutboxEntry, batch_id: &EnterpriseBatchId) -> Result<(), String> {
    if &entry.batch_id != batch_id || entry.revision == 0 {
        return Err("central ingestion outbox state does not match its target reference".into());
    }
    batch_digest(batch_id)?;
    match &entry.state {
        OutboxState::Pending => Ok(()),
        OutboxState::InFlight { attempt_id } => {
            validate_digest_reference("outbox attempt", attempt_id, "outbox-attempt:")
        }
        OutboxState::Acked {
            attempt_id,
            resolution_id,
        }
        | OutboxState::Rejected {
            attempt_id,
            resolution_id,
        } => {
            validate_digest_reference("outbox attempt", attempt_id, "outbox-attempt:")?;
            validate_digest_reference(
                "central ingestion resolution",
                resolution_id,
                "central-ingestion:",
            )
        }
    }
}

impl OutboxStateFilter {
    fn matches(self, state: &OutboxState) -> bool {
        match self {
            Self::Pending => matches!(state, OutboxState::Pending),
            Self::InFlight => matches!(state, OutboxState::InFlight { .. }),
            Self::PendingOrInFlight => {
                matches!(state, OutboxState::Pending | OutboxState::InFlight { .. })
            }
        }
    }
}

fn scan_entry_paths(directory: &Path) -> Result<Vec<(EnterpriseBatchId, PathBuf)>, String> {
    let mut paths = Vec::new();
    let mut scanned = 0usize;
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        scanned += 1;
        if scanned > MAX_OUTBOX_FILES {
            return Err(format!(
                "central ingestion outbox exceeds the {MAX_OUTBOX_FILES}-file scan limit"
            ));
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            return Err("central ingestion outbox refuses a symlink entry".into());
        }
        if !file_type.is_file() {
            return Err("central ingestion outbox contains a non-regular entry".into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "central ingestion outbox entry name is not UTF-8".to_string())?;
        if is_pending_file_name(&name) {
            continue;
        }
        let batch_id = batch_id_from_state_file_name(&name)?;
        paths.push((batch_id, entry.path()));
    }
    paths.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    Ok(paths)
}

fn batch_id_from_state_file_name(name: &str) -> Result<EnterpriseBatchId, String> {
    let digest = name
        .strip_suffix(".json")
        .ok_or_else(|| "central ingestion outbox contains an unexpected entry".to_string())?;
    validate_digest("outbox state file", digest)?;
    EnterpriseBatchId::new(format!("batch:{digest}"))
}

fn is_pending_file_name(name: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|name| name.strip_suffix(".state.pending"))
        .is_some_and(|digest| validate_digest("outbox pending file", digest).is_ok())
}

fn read_outbox_state(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("central ingestion outbox state path is not a regular file".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_OUTBOX_STATE_BYTES {
        return Err("central ingestion outbox state is unsafe or oversized".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("central ingestion outbox state must not be hard-linked".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("central ingestion outbox state must not be a reparse point".into());
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_OUTBOX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_OUTBOX_STATE_BYTES {
        return Err("central ingestion outbox state exceeds its storage limit".into());
    }
    Ok(bytes)
}

fn decode_list_cursor(
    cursor: Option<&str>,
    enterprise_id: &EnterpriseId,
    filter: OutboxStateFilter,
) -> Result<String, String> {
    let Some(cursor) = cursor else {
        return Ok(String::new());
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| "invalid central ingestion outbox cursor encoding".to_string())?;
    let cursor: OutboxPageCursor = serde_json::from_slice(&bytes)
        .map_err(|_| "invalid central ingestion outbox cursor payload".to_string())?;
    if cursor.version != OUTBOX_CURSOR_VERSION
        || cursor.enterprise_id != *enterprise_id
        || cursor.filter != filter
    {
        return Err("mismatched central ingestion outbox cursor; restart enumeration".to_string());
    }
    let batch_id = EnterpriseBatchId::new(cursor.last_batch_id)?;
    batch_digest(&batch_id)?;
    Ok(batch_id.to_string())
}

fn encode_list_cursor(
    enterprise_id: &EnterpriseId,
    filter: OutboxStateFilter,
    batch_id: &EnterpriseBatchId,
) -> Result<String, String> {
    let cursor = OutboxPageCursor {
        version: OUTBOX_CURSOR_VERSION,
        enterprise_id: enterprise_id.clone(),
        filter,
        last_batch_id: batch_id.to_string(),
    };
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&cursor).map_err(|error| error.to_string())?))
}

fn validate_list_limit(limit: usize) -> Result<(), String> {
    if limit == 0 || limit > MAX_OUTBOX_PAGE_ENTRIES {
        return Err(format!(
            "central ingestion outbox limit must be in 1..={MAX_OUTBOX_PAGE_ENTRIES}"
        ));
    }
    Ok(())
}

fn write_entry(directory: &Path, entry: &OutboxEntry) -> Result<(), String> {
    let path = entry_path(directory, &entry.batch_id)?;
    let digest = batch_digest(&entry.batch_id)?;
    let temporary = directory.join(format!(".{digest}.state.pending"));
    clear_interrupted_temporary(&temporary)?;
    let bytes = serde_json::to_vec(entry).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_OUTBOX_STATE_BYTES {
        return Err("central ingestion outbox state exceeds its storage limit".into());
    }
    write_private_new(&temporary, &bytes)?;
    replace_file(&temporary, &path)?;
    sync_directory(directory)
}

fn clear_interrupted_temporary(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("central ingestion outbox pending path is unsafe".into())
        }
        Ok(_) => {
            fs::remove_file(path).map_err(|error| error.to_string())?;
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
    }
}

fn outbox_directory(root: &Path) -> Result<PathBuf, String> {
    let private = root.join("private");
    ensure_real_directory(&private)?;
    let directory = private.join("central-ingestion-outbox");
    ensure_private_directory(&directory)?;
    Ok(directory)
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err("central ingestion outbox path must be a real directory".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(not(unix))]
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path).map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn entry_path(directory: &Path, batch_id: &EnterpriseBatchId) -> Result<PathBuf, String> {
    Ok(directory.join(format!("{}.json", batch_digest(batch_id)?)))
}

fn batch_digest(batch_id: &EnterpriseBatchId) -> Result<&str, String> {
    let digest = batch_id
        .as_str()
        .strip_prefix("batch:")
        .ok_or_else(|| "outbox batch id has an invalid prefix".to_string())?;
    validate_digest("outbox batch id", digest)?;
    Ok(digest)
}

fn validate_digest(label: &str, digest: &str) -> Result<(), String> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} has an invalid digest"));
    }
    Ok(())
}

fn validate_digest_reference(label: &str, value: &str, prefix: &str) -> Result<(), String> {
    let digest = value
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{label} has an invalid prefix"))?;
    validate_digest(label, digest)
}

fn next_revision(current: u64) -> Result<u64, String> {
    current
        .checked_add(1)
        .ok_or_else(|| "central ingestion outbox revision overflow".to_string())
}

fn write_private_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match exec_private_fs::write_private_new(path, bytes) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "private outbox path already exists: {}",
            path.display()
        )),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from = from
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
