use std::fs::{self, File};
use std::path::Path;

use exec_private_fs::PrivateFileOptions;
use sha2::{Digest, Sha256};

use crate::error::{RuntimeError, RuntimeResult};

pub(crate) fn private_options() -> PrivateFileOptions {
    PrivateFileOptions::new()
}

pub(crate) fn open_private(path: &Path, create: bool) -> RuntimeResult<File> {
    reject_symlink(path)?;
    let mut options = private_options();
    options.read(true).write(true).create(create);
    let file = options.open(path).map_err(|_| RuntimeError::Vault)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = file.metadata().map_err(|_| RuntimeError::Vault)?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(RuntimeError::Vault);
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| RuntimeError::Vault)?;
    }
    Ok(file)
}

pub(crate) fn reject_symlink(path: &Path) -> RuntimeResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Vault),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(RuntimeError::Vault),
    }
}

#[cfg(windows)]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> RuntimeResult<()> {
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
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    (result != 0).then_some(()).ok_or(RuntimeError::Vault)
}

#[cfg(not(windows))]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> RuntimeResult<()> {
    fs::rename(source, destination).map_err(|_| RuntimeError::Vault)
}

pub(crate) fn sync_directory(path: &Path) -> RuntimeResult<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| RuntimeError::Vault)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

pub(crate) fn random_digest() -> RuntimeResult<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| RuntimeError::Vault)?;
    Ok(digest(&bytes))
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
