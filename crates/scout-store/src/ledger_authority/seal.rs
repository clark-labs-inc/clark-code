use std::fs;
use std::path::Path;
#[cfg(not(any(unix, windows)))]
use std::time::UNIX_EPOCH;

use agent_orchestration::EnterpriseId;
use serde::{Deserialize, Serialize};

use super::database::{auth_mac, verify_mac, AUTH_KEY_BYTES};
use super::{LedgerHead, LEDGER_DATABASE_NAME};

const SEAL_FILE: &str = "ledger-authority-storage-seal.json";
const SEAL_SCHEMA_VERSION: u16 = 1;
const MAX_SEAL_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileChangeToken {
    identity_high: u64,
    identity_low: u64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StorageSeal {
    schema_version: u16,
    enterprise_id: EnterpriseId,
    head_id: String,
    generation: u64,
    file_len: u64,
    change_token: FileChangeToken,
    mac: Vec<u8>,
}

impl StorageSeal {
    pub(super) fn head_id(&self) -> &str {
        &self.head_id
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }
}

pub(super) fn database_exists(root: &Path) -> Result<bool, String> {
    let path = root.join(LEDGER_DATABASE_NAME);
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("Scout ledger database path is unsafe".into())
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn validate(
    root: &Path,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
) -> Result<StorageSeal, String> {
    let seal = read_authenticated(root, auth_key, enterprise_id)?;
    if !matches_current(root, &seal)? {
        return Err("Scout ledger database changed outside its authenticated transaction".into());
    }
    Ok(seal)
}

pub(super) fn read_authenticated(
    root: &Path,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
) -> Result<StorageSeal, String> {
    let path = root.join("private").join(SEAL_FILE);
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_SEAL_BYTES
    {
        return Err("Scout ledger storage seal path is unsafe".into());
    }
    let seal: StorageSeal =
        serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if seal.schema_version != SEAL_SCHEMA_VERSION || seal.enterprise_id != *enterprise_id {
        return Err("Scout ledger storage seal belongs to another authority".into());
    }
    verify_mac(
        auth_key,
        "ledger-storage-seal-v1",
        &(
            seal.schema_version,
            &seal.enterprise_id,
            &seal.head_id,
            seal.generation,
            seal.file_len,
            &seal.change_token,
        ),
        &seal.mac,
    )?;
    Ok(seal)
}

pub(super) fn matches_current(root: &Path, seal: &StorageSeal) -> Result<bool, String> {
    let (file_len, change_token) = current_content(root)?;
    Ok((file_len, change_token) == (seal.file_len, seal.change_token.clone()))
}

pub(super) fn validate_unchanged(
    root: &Path,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    expected: &StorageSeal,
) -> Result<(), String> {
    let current = validate(root, auth_key, enterprise_id)?;
    if &current != expected {
        return Err("Scout ledger storage seal changed during an operation".into());
    }
    Ok(())
}

pub(super) fn require_head(seal: &StorageSeal, head: &LedgerHead) -> Result<(), String> {
    if seal.head_id != head.head_id || seal.generation != head.generation {
        return Err("Scout ledger storage seal does not authenticate the current head".into());
    }
    Ok(())
}

pub(super) fn write(
    root: &Path,
    auth_key: &[u8; AUTH_KEY_BYTES],
    head: &LedgerHead,
) -> Result<(), String> {
    let (file_len, change_token) = current_content(root)?;
    let mac = auth_mac(
        auth_key,
        "ledger-storage-seal-v1",
        &(
            SEAL_SCHEMA_VERSION,
            &head.enterprise_id,
            &head.head_id,
            head.generation,
            file_len,
            &change_token,
        ),
    )?;
    crate::checkpoint::replace_private_json(
        &root.join("private"),
        SEAL_FILE,
        &StorageSeal {
            schema_version: SEAL_SCHEMA_VERSION,
            enterprise_id: head.enterprise_id.clone(),
            head_id: head.head_id.clone(),
            generation: head.generation,
            file_len,
            change_token,
            mac,
        },
    )
}

fn current_content(root: &Path) -> Result<(u64, FileChangeToken), String> {
    let path = root.join(LEDGER_DATABASE_NAME);
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Scout ledger database path is unsafe".into());
    }
    Ok((metadata.len(), file_change_token(&path, &metadata)?))
}

#[cfg(unix)]
fn file_change_token(_path: &Path, metadata: &fs::Metadata) -> Result<FileChangeToken, String> {
    use std::os::unix::fs::MetadataExt as _;
    Ok(FileChangeToken {
        identity_high: metadata.dev(),
        identity_low: metadata.ino(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

#[cfg(windows)]
fn file_change_token(path: &Path, _metadata: &fs::Metadata) -> Result<FileChangeToken, String> {
    use std::mem::{size_of, MaybeUninit};
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
    };
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let handle = file.as_raw_handle() as _;
    let mut identity = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    let mut basic = MaybeUninit::<FILE_BASIC_INFO>::zeroed();
    // SAFETY: both calls receive a live handle and correctly sized output structs.
    if unsafe { GetFileInformationByHandle(handle, identity.as_mut_ptr()) } == 0
        || unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                basic.as_mut_ptr().cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: both successful calls initialized their output structures.
    let identity = unsafe { identity.assume_init() };
    let basic = unsafe { basic.assume_init() };
    Ok(FileChangeToken {
        identity_high: u64::from(identity.dwVolumeSerialNumber),
        identity_low: (u64::from(identity.nFileIndexHigh) << 32)
            | u64::from(identity.nFileIndexLow),
        changed_seconds: basic.ChangeTime,
        changed_nanoseconds: 0,
    })
}

#[cfg(not(any(unix, windows)))]
fn file_change_token(_path: &Path, metadata: &fs::Metadata) -> Result<FileChangeToken, String> {
    let created = metadata
        .created()
        .map_err(|error| error.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Scout ledger creation time predates the Unix epoch".to_string())?;
    Ok(FileChangeToken {
        identity_high: 0,
        identity_low: 0,
        changed_seconds: i64::try_from(created.as_secs())
            .map_err(|_| "Scout ledger creation time exceeds i64".to_string())?,
        changed_nanoseconds: i64::from(created.subsec_nanos()),
    })
}
