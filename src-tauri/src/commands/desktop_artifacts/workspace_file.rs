//! Handle-bound reads from a Clark workspace directory.
//!
//! A pathname is mutable after validation. These helpers instead open the
//! workspace as a directory capability, validate the resulting file handle,
//! and return that exact handle to the caller for consumption.

use std::{
    fs::File,
    io::Read,
    path::{Component, Path},
};

pub(super) struct CheckedMarkdown {
    pub(super) filename: String,
    file: File,
}

pub(super) fn open_markdown_file(
    workspace: &Path,
    relative: &Path,
    max_bytes: u64,
) -> Result<CheckedMarkdown, String> {
    validate_relative_path(relative)?;
    if !provider_local::is_markdown(relative) {
        return Err("artifact is not Markdown in this conversation workspace".into());
    }
    let filename = relative
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "artifact filename is not valid UTF-8".to_string())?
        .to_string();
    let file = platform::open_workspace_relative_file(workspace, relative)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("artifact metadata failed: {error}"))?;
    if !metadata.is_file() {
        return Err("artifact is not a regular file".into());
    }
    if metadata.len() > max_bytes {
        return Err("Markdown artifact exceeds the 8 MB cloud limit".into());
    }
    Ok(CheckedMarkdown { filename, file })
}

pub(super) fn read_checked_bytes(
    checked: CheckedMarkdown,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let mut limited = checked.file.take(
        max_bytes
            .checked_add(1)
            .ok_or_else(|| "Markdown artifact size limit overflow".to_string())?,
    );
    let mut bytes = Vec::with_capacity(
        usize::try_from(max_bytes)
            .map_err(|_| "Markdown artifact size limit is unsupported".to_string())?,
    );
    limited
        .read_to_end(&mut bytes)
        .map_err(|error| format!("artifact read failed: {error}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err("Markdown artifact exceeds the 8 MB cloud limit".into());
    }
    Ok(bytes)
}

fn validate_relative_path(relative: &Path) -> Result<(), String> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("workspace artifact path is not confined".into());
    }
    Ok(())
}

#[cfg(unix)]
mod platform {
    use std::{
        ffi::{CString, OsStr},
        fs::File,
        os::unix::{
            ffi::OsStrExt,
            fs::MetadataExt,
            io::{AsRawFd, FromRawFd},
        },
        path::{Component, Path},
    };

    pub(super) fn open_workspace_relative_file(
        workspace: &Path,
        relative: &Path,
    ) -> Result<File, String> {
        let mut directory = open_workspace_directory(workspace)?;
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err("workspace artifact path is not confined".into());
            };
            let is_leaf = components.peek().is_none();
            let flags = libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | if is_leaf { 0 } else { libc::O_DIRECTORY };
            let opened = open_at(&directory, name, flags, "artifact")?;
            if is_leaf {
                return Ok(opened);
            }
            directory = opened;
        }
        Err("workspace artifact path is not confined".into())
    }

    fn open_workspace_directory(workspace: &Path) -> Result<File, String> {
        let canonical = workspace
            .canonicalize()
            .map_err(|error| format!("workspace is unavailable: {error}"))?;
        let expected = canonical
            .metadata()
            .map_err(|error| format!("workspace metadata failed: {error}"))?;
        if !expected.is_dir() {
            return Err("workspace is not a real directory".into());
        }
        let mut components = canonical.components();
        if !matches!(components.next(), Some(Component::RootDir)) {
            return Err("workspace path is not absolute".into());
        }
        let root = CString::new("/").expect("root path contains no NUL bytes");
        // SAFETY: `root` is NUL-terminated and the flags do not require a mode
        // argument. The returned descriptor is owned below.
        let root_fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if root_fd < 0 {
            return Err(format!(
                "workspace is unavailable: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: `root_fd` is a successful `open` result, so ownership
        // transfers to `File` exactly once.
        let mut directory = unsafe { File::from_raw_fd(root_fd) };
        let mut has_component = false;
        for component in components {
            let Component::Normal(name) = component else {
                return Err("workspace path is not confined".into());
            };
            has_component = true;
            directory = open_at(
                &directory,
                name,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                "workspace",
            )?;
        }
        if !has_component {
            return Err("workspace path is not confined".into());
        }
        let opened = directory
            .metadata()
            .map_err(|error| format!("workspace metadata failed: {error}"))?;
        if opened.dev() != expected.dev() || opened.ino() != expected.ino() {
            return Err("workspace changed while opening".into());
        }
        Ok(directory)
    }

    fn open_at(
        directory: &File,
        name: &OsStr,
        flags: libc::c_int,
        kind: &str,
    ) -> Result<File, String> {
        let name = CString::new(name.as_bytes())
            .map_err(|_| format!("{kind} path contains an invalid byte"))?;
        // SAFETY: `directory` owns a live directory descriptor; `name` is
        // NUL-terminated; and the flags do not require a mode argument.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(format!(
                "{kind} is unavailable: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: `fd` is a successful `openat` result, so ownership transfers
        // to `File` exactly once.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        fs::{File, OpenOptions},
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::Path,
    };
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            GetFinalPathNameByHandleW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
        },
    };

    pub(super) fn open_workspace_relative_file(
        workspace: &Path,
        relative: &Path,
    ) -> Result<File, String> {
        let mut directory_options = OpenOptions::new();
        directory_options
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let workspace_file = directory_options
            .open(workspace)
            .map_err(|error| format!("workspace is unavailable: {error}"))?;
        let workspace_metadata = workspace_file
            .metadata()
            .map_err(|error| format!("workspace metadata failed: {error}"))?;
        if !workspace_metadata.is_dir()
            || workspace_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err("workspace is not a real directory".into());
        }
        let workspace_path = final_path(&workspace_file)?;

        let mut file_options = OpenOptions::new();
        file_options
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        let file = file_options
            .open(workspace.join(relative))
            .map_err(|error| format!("artifact is unavailable: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("artifact metadata failed: {error}"))?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("workspace artifact must not be a reparse point".into());
        }
        if !is_descendant(&final_path(&file)?, &workspace_path) {
            return Err("workspace artifact path is not confined".into());
        }
        Ok(file)
    }

    fn final_path(file: &File) -> Result<Vec<u16>, String> {
        let mut path = vec![0u16; 260];
        loop {
            let length = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle() as HANDLE,
                    path.as_mut_ptr(),
                    path.len() as u32,
                    FILE_NAME_NORMALIZED,
                )
            };
            if length == 0 {
                return Err(format!(
                    "artifact path validation failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if (length as usize) < path.len() {
                path.truncate(length as usize);
                return Ok(path);
            }
            path.resize(length as usize + 1, 0);
        }
    }

    fn is_descendant(path: &[u16], workspace: &[u16]) -> bool {
        let separator = path.get(workspace.len()).copied();
        path.len() > workspace.len()
            && path.starts_with(workspace)
            && separator.is_some_and(|value| value == u16::from(b'\\') || value == u16::from(b'/'))
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::{fs::File, path::Path};

    pub(super) fn open_workspace_relative_file(
        _workspace: &Path,
        _relative: &Path,
    ) -> Result<File, String> {
        Err("secure workspace artifact reads are unsupported on this platform".into())
    }
}
