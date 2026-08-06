use std::fs;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::ledger_authority::LEDGER_DATABASE_NAME;

pub(in crate::ledger_authority) const AUTH_KEY_BYTES: usize = 32;
const AUTH_KEY_FILE: &str = "ledger-authority-auth.key";

pub(in crate::ledger_authority) fn prepare_root(root: &Path) -> Result<(), String> {
    ensure_real_directory(root)?;
    ensure_real_directory(&root.join("private"))?;
    let database_path = root.join(LEDGER_DATABASE_NAME);
    match fs::symlink_metadata(&database_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("Scout ledger database path is unsafe".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub(in crate::ledger_authority) fn load_or_create_auth_key(
    root: &Path,
) -> Result<[u8; AUTH_KEY_BYTES], String> {
    let path = root.join("private").join(AUTH_KEY_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("Scout ledger authentication key path is unsafe".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut key = [0_u8; AUTH_KEY_BYTES];
            getrandom::fill(&mut key)
                .map_err(|_| "Scout ledger authentication key generation failed".to_string())?;
            exec_private_fs::write_private_new(&path, &key).map_err(|error| error.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.len() != AUTH_KEY_BYTES as u64 {
        return Err("Scout ledger authentication key has the wrong length".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err("Scout ledger authentication key must not be hard-linked".into());
        }
    }
    fs::read(path)
        .map_err(|error| error.to_string())?
        .try_into()
        .map_err(|_| "Scout ledger authentication key has the wrong length".to_string())
}

pub(in crate::ledger_authority) fn auth_mac(
    key: &[u8; AUTH_KEY_BYTES],
    domain: &str,
    value: &impl Serialize,
) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(&(domain, value)).map_err(|error| error.to_string())?;
    Ok(hmac_sha256(key, &bytes).to_vec())
}

pub(in crate::ledger_authority) fn verify_mac(
    key: &[u8; AUTH_KEY_BYTES],
    domain: &str,
    value: &impl Serialize,
    observed: &[u8],
) -> Result<(), String> {
    let expected = auth_mac(key, domain, value)?;
    let difference = observed
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        });
    if observed.len() != expected.len() || difference != 0 {
        return Err("Scout ledger authority authentication failed".into());
    }
    Ok(())
}

pub(in crate::ledger_authority) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(in crate::ledger_authority) fn validate_hex_digest(
    label: &str,
    value: &str,
) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} digest must be 64 hexadecimal characters"));
    }
    Ok(())
}

pub(in crate::ledger_authority) fn validate_prefixed_hex(
    label: &str,
    value: &str,
    prefix: &str,
) -> Result<(), String> {
    let digest = value
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{label} has the wrong namespace"))?;
    validate_hex_digest(label, digest)
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(format!(
            "Scout ledger path is not a real directory: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn hmac_sha256(key: &[u8; AUTH_KEY_BYTES], message: &[u8]) -> [u8; 32] {
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
