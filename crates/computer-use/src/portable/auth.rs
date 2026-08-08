use std::fs;
use std::path::{Path, PathBuf};

use interprocess::local_socket::traits::StreamCommon;
use interprocess::local_socket::Stream;
#[cfg(feature = "helper-service")]
use sha2::{Digest, Sha256};

use crate::ComputerUseError;

#[cfg(feature = "helper-service")]
pub fn verify_own_executable() -> Result<(), ComputerUseError> {
    let executable = std::env::current_exe()
        .map_err(|error| ComputerUseError::HelperRejected(error.to_string()))?;
    let metadata = fs::metadata(&executable)
        .map_err(|error| ComputerUseError::HelperRejected(error.to_string()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(ComputerUseError::HelperRejected(
            "service executable is missing or empty".to_string(),
        ));
    }
    Ok(())
}

#[cfg(feature = "helper-service")]
pub fn authenticate_client(
    stream: &Stream,
    expected_pid: u32,
) -> Result<PathBuf, ComputerUseError> {
    let credentials = stream
        .peer_creds()
        .map_err(|error| ComputerUseError::HelperRejected(error.to_string()))?;
    let peer_pid = credentials.pid().and_then(normalize_pid).ok_or_else(|| {
        ComputerUseError::HelperRejected("local IPC did not provide a peer PID".to_string())
    })?;
    if peer_pid != expected_pid {
        return Err(ComputerUseError::HelperRejected(format!(
            "client PID mismatch: expected {expected_pid}, got {peer_pid}"
        )));
    }
    #[cfg(unix)]
    {
        let peer_uid = credentials.euid().ok_or_else(|| {
            ComputerUseError::HelperRejected("local IPC did not provide a peer UID".to_string())
        })?;
        let own_uid = unsafe { libc::geteuid() };
        if peer_uid != own_uid {
            return Err(ComputerUseError::HelperRejected(format!(
                "client UID mismatch: expected {own_uid}, got {peer_uid}"
            )));
        }
    }
    process_executable(peer_pid)
}

pub fn verify_service_peer(
    stream: &Stream,
    expected_pid: u32,
    expected_executable: &Path,
) -> Result<(), ComputerUseError> {
    let credentials = stream
        .peer_creds()
        .map_err(|error| ComputerUseError::HelperRejected(error.to_string()))?;
    let peer_pid = credentials.pid().and_then(normalize_pid).ok_or_else(|| {
        ComputerUseError::HelperRejected("local IPC did not provide a service PID".to_string())
    })?;
    if peer_pid != expected_pid {
        return Err(ComputerUseError::HelperRejected(format!(
            "service PID mismatch: expected {expected_pid}, got {peer_pid}"
        )));
    }
    let live = process_executable(peer_pid)?;
    let expected = canonical(expected_executable)?;
    if live != expected {
        return Err(ComputerUseError::HelperRejected(format!(
            "service executable mismatch: expected {}, got {}",
            expected.display(),
            live.display()
        )));
    }
    Ok(())
}

#[cfg(feature = "helper-service")]
pub fn application_identity(
    pid: u32,
    bundle_id: &str,
) -> Result<crate::ApplicationIdentity, ComputerUseError> {
    let executable = process_executable(pid)?;
    let bytes = fs::read(&executable).map_err(|error| {
        ComputerUseError::HelperRejected(format!(
            "could not hash target executable {}: {error}",
            executable.display()
        ))
    })?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    let requirement = format!("sha256:{digest}");
    let identity_key = format!("{bundle_id}|{requirement}");
    Ok(crate::ApplicationIdentity {
        bundle_id: bundle_id.to_string(),
        team_identifier: None,
        designated_requirement: requirement,
        identity_key,
        // Raw executable hashes change on every update. Keep durable approval
        // disabled until a platform publisher identity is available.
        durable_approval_eligible: false,
    })
}

#[cfg(target_os = "linux")]
fn process_executable(pid: u32) -> Result<PathBuf, ComputerUseError> {
    canonical(Path::new(&format!("/proc/{pid}/exe")))
}

#[cfg(target_os = "windows")]
fn process_executable(pid: u32) -> Result<PathBuf, ComputerUseError> {
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|error| ComputerUseError::HelperRejected(error.to_string()))?;
    let mut buffer = vec![0_u16; 32_768];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.map_err(|error| ComputerUseError::HelperRejected(error.to_string()))?;
    canonical(&PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length as usize],
    )))
}

fn canonical(path: &Path) -> Result<PathBuf, ComputerUseError> {
    fs::canonicalize(path).map_err(|error| {
        ComputerUseError::HelperRejected(format!(
            "could not resolve executable {}: {error}",
            path.display()
        ))
    })
}

#[cfg(unix)]
fn normalize_pid(pid: libc::pid_t) -> Option<u32> {
    u32::try_from(pid).ok()
}

#[cfg(windows)]
fn normalize_pid(pid: u32) -> Option<u32> {
    Some(pid)
}
