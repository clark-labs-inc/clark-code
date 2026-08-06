use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::super::database::{index_mac, verify_index_mac, DB_NAME, INDEX_AUTH_KEY_BYTES};
use super::ledger::{file_change_token, FileChangeToken};
use crate::checkpoint::replace_private_json;
use crate::index::{io_error, read_regular_bounded};

const SEAL_FILE: &str = "index-storage-seal.json";
const SEAL_SCHEMA_VERSION: u16 = 1;
const MAX_SEAL_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexStorageSeal {
    schema_version: u16,
    file_len: u64,
    change_token: FileChangeToken,
    mac: String,
}

pub(super) fn validate(root: &Path, auth_key: &[u8; INDEX_AUTH_KEY_BYTES]) -> Result<bool, String> {
    let Some(current) = current_content(root)? else {
        return Ok(false);
    };
    let seal_path = root.join("private").join(SEAL_FILE);
    let bytes = match fs::symlink_metadata(&seal_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error(error)),
        Ok(_) => read_regular_bounded(&seal_path, MAX_SEAL_BYTES, "index storage seal")?,
    };
    let seal: IndexStorageSeal =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if seal.schema_version != SEAL_SCHEMA_VERSION {
        return Ok(false);
    }
    verify_index_mac(
        auth_key,
        "index-storage-seal",
        &(seal.schema_version, seal.file_len, &seal.change_token),
        &seal.mac,
    )?;
    Ok((seal.file_len, seal.change_token) == current)
}

pub(super) fn write(root: &Path, auth_key: &[u8; INDEX_AUTH_KEY_BYTES]) -> Result<(), String> {
    let (file_len, change_token) = current_content(root)?
        .ok_or_else(|| "Scout index database is missing after commit".to_string())?;
    let mac = index_mac(
        auth_key,
        "index-storage-seal",
        &(SEAL_SCHEMA_VERSION, file_len, &change_token),
    )?;
    replace_private_json(
        &root.join("private"),
        SEAL_FILE,
        &IndexStorageSeal {
            schema_version: SEAL_SCHEMA_VERSION,
            file_len,
            change_token,
            mac,
        },
    )
}

fn current_content(root: &Path) -> Result<Option<(u64, FileChangeToken)>, String> {
    let path = root.join(DB_NAME);
    let metadata = match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
        Ok(metadata) => metadata,
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Scout index database path is unsafe".into());
    }
    Ok(Some((metadata.len(), file_change_token(&path, &metadata)?)))
}
