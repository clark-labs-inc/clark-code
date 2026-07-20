use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CREDENTIAL_FILE: &str = "offline-credential-v1.json";
pub const CREDENTIAL_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CredentialEnvelope {
    pub version: u32,
    pub username: String,
    pub sid: String,
    pub protected_password_b64: String,
}

impl CredentialEnvelope {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != CREDENTIAL_VERSION {
            return Err("Windows sandbox credential version is unsupported".into());
        }
        if self.username.is_empty() || self.sid.is_empty() || self.protected_password_b64.is_empty()
        {
            return Err("Windows sandbox credential is incomplete".into());
        }
        Ok(())
    }
}

pub fn credential_path(state_dir: &Path) -> PathBuf {
    state_dir.join(CREDENTIAL_FILE)
}

pub fn read_credential(state_dir: &Path) -> Result<CredentialEnvelope, String> {
    let path = credential_path(state_dir);
    let bytes = std::fs::read(&path).map_err(|error| {
        format!(
            "read Windows sandbox credential {}: {error}",
            path.display()
        )
    })?;
    let credential: CredentialEnvelope = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "parse Windows sandbox credential {}: {error}",
            path.display()
        )
    })?;
    credential.validate()?;
    Ok(credential)
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("state path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create Windows sandbox state directory: {error}"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sandbox-state"),
        std::process::id()
    ));
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("serialize Windows sandbox state: {error}"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            format!(
                "create Windows sandbox state {}: {error}",
                temporary.display()
            )
        })?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!(
            "write Windows sandbox state {}: {error}",
            temporary.display()
        ));
    }
    drop(file);
    replace_file(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(format!(
            "atomically replace Windows sandbox state: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(source, destination)
        .map_err(|error| format!("atomically replace Windows sandbox state: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_round_trip_is_versioned_and_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let credential = CredentialEnvelope {
            version: CREDENTIAL_VERSION,
            username: "ClarkSandboxOffline".into(),
            sid: "S-1-5-21-1000".into(),
            protected_password_b64: "fixture".into(),
        };
        write_json_atomic(&credential_path(dir.path()), &credential).unwrap();
        assert_eq!(read_credential(dir.path()).unwrap(), credential);
        let mut replacement = credential.clone();
        replacement.protected_password_b64 = "replacement".into();
        write_json_atomic(&credential_path(dir.path()), &replacement).unwrap();
        assert_eq!(read_credential(dir.path()).unwrap(), replacement);
    }
}
