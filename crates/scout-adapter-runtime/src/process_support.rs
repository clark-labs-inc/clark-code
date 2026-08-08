use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Stdio;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::{RuntimeError, RuntimeResult};
use crate::process::TargetEnvironment;

const MAX_CAPTURE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PAGE_SIZE: u32 = 999;

pub(crate) fn discover_executable(name: &str, environment: &TargetEnvironment) -> Option<PathBuf> {
    let path = environment.values.get("PATH")?;
    std::env::split_paths(path).find_map(|directory| executable_in(&directory, name))
}

fn executable_in(directory: &Path, name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    let names = [
        format!("{name}.exe"),
        format!("{name}.cmd"),
        name.to_owned(),
    ];
    #[cfg(not(windows))]
    let names = [name.to_owned()];
    names
        .into_iter()
        .map(|name| directory.join(name))
        .find(|candidate| candidate.is_file())
}

pub(crate) async fn read_bounded(
    stream: impl tokio::io::AsyncRead + Unpin,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream
        .take(MAX_CAPTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_CAPTURE_BYTES {
        return Err(std::io::Error::other("bounded process output exceeded"));
    }
    Ok(bytes)
}

pub(crate) fn validate_page(page: u32, size: u32) -> RuntimeResult<()> {
    if page == 0 || size == 0 || size > MAX_PAGE_SIZE {
        return Err(RuntimeError::InvalidRequest);
    }
    Ok(())
}

pub(crate) fn validate_github_name(value: &str) -> RuntimeResult<()> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RuntimeError::InvalidRequest);
    }
    Ok(())
}

pub(crate) fn validate_region(value: &str) -> RuntimeResult<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(RuntimeError::InvalidRequest);
    }
    Ok(())
}

pub(crate) fn validate_opaque_token(value: &str) -> RuntimeResult<()> {
    if value.is_empty()
        || value.len() > 8_192
        || value
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(RuntimeError::ProviderProtocol);
    }
    Ok(())
}

pub(crate) fn validate_gcloud_value(value: &str) -> RuntimeResult<()> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > 512
        || value
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(RuntimeError::InvalidRequest);
    }
    Ok(())
}

pub(crate) fn isolate_process_group(command: &mut Command) {
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

pub(crate) async fn terminate_process_tree(
    child: &mut tokio::process::Child,
    root_pid: Option<u32>,
) {
    #[cfg(unix)]
    if let Some(pid) = root_pid.and_then(|pid| i32::try_from(pid).ok()) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    if let Some(pid) = root_pid {
        let mut command = Command::new("taskkill");
        command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        isolate_process_group(&mut command);
        let _ = command.status().await;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}
