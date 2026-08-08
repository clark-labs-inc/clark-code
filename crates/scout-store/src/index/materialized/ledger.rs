use std::fs;
use std::path::Path;
#[cfg(not(any(unix, windows)))]
use std::time::UNIX_EPOCH;

#[cfg(not(unix))]
use crate::index::io_error;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct FileChangeToken {
    identity_high: u64,
    identity_low: u64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(unix)]
pub(super) fn file_change_token(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<FileChangeToken, String> {
    use std::os::unix::fs::MetadataExt as _;

    Ok(FileChangeToken {
        identity_high: metadata.dev(),
        identity_low: metadata.ino(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

#[cfg(windows)]
pub(super) fn file_change_token(
    path: &Path,
    _metadata: &fs::Metadata,
) -> Result<FileChangeToken, String> {
    use std::mem::{size_of, MaybeUninit};
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
    };

    let file = fs::File::open(path).map_err(io_error)?;
    let handle = file.as_raw_handle() as _;
    let mut identity = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    let mut basic = MaybeUninit::<FILE_BASIC_INFO>::zeroed();
    // SAFETY: both calls receive a live handle and correctly sized outputs.
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
        return Err(io_error(std::io::Error::last_os_error()));
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
pub(super) fn file_change_token(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<FileChangeToken, String> {
    let created = metadata
        .created()
        .map_err(io_error)?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Scout index creation time predates the Unix epoch".to_string())?;
    Ok(FileChangeToken {
        identity_high: 0,
        identity_low: 0,
        changed_seconds: i64::try_from(created.as_secs())
            .map_err(|_| "Scout index creation time exceeds i64".to_string())?,
        changed_nanoseconds: i64::from(created.subsec_nanos()),
    })
}
