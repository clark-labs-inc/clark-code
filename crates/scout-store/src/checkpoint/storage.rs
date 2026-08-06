use std::path::Path;

#[cfg(not(windows))]
use std::fs;

use serde::Serialize;

use crate::index::sync_directory;

pub(crate) fn replace_private_json(
    directory: &Path,
    file_name: &str,
    value: &impl Serialize,
) -> Result<(), String> {
    let path = directory.join(file_name);
    let temporary = directory.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    write_private_new(&temporary, &bytes)?;
    replace_file(&temporary, &path)?;
    sync_directory(directory)
}

pub(crate) fn write_private_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match exec_private_fs::write_private_new(path, bytes) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "private checkpoint path already exists: {}",
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

pub(crate) fn checkpoint_file_name(sequence: u64) -> String {
    format!("{sequence:020}.json")
}

pub(crate) fn parse_checkpoint_file_name(value: &str) -> Option<u64> {
    let digits = value.strip_suffix(".json")?;
    (digits.len() == 20 && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}
