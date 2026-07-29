#[cfg(any(windows, test))]
use std::ffi::OsString;
use std::path::{Component, Path};
use std::process::Stdio;
use std::time::Duration;

use exec_core::{spawn_process, terminate_process_tree, ExecOutput, ProcessFence, ProcessSpec};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use super::PocLanguage;
#[cfg(test)]
use super::MAX_TIMEOUT_SECONDS;
use crate::security::{SecurityPocControl, SecurityPocReceipt};
use crate::tools::ToolCtx;

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_WORKSPACE_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_PREVIEW_BYTES: usize = 8 * 1024;

#[cfg(any(windows, test))]
fn powershell_script_args(script_path: &Path) -> [OsString; 5] {
    [
        OsString::from("-NoLogo"),
        OsString::from("-NoProfile"),
        OsString::from("-NonInteractive"),
        OsString::from("-File"),
        script_path.as_os_str().to_os_string(),
    ]
}

pub(super) async fn copy_inventory(
    ctx: &ToolCtx,
    paths: &[String],
    workspace: &Path,
) -> Result<String, String> {
    let mut total = 0u64;
    let mut hasher = Sha256::new();
    for relative in paths {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(format!("inventory contains unsafe path `{relative}`"));
        }
        let source = ctx.sandbox.root().join(relative_path);
        let metadata = ctx.executor.metadata(&source).await?;
        if metadata.len > MAX_FILE_BYTES {
            return Err(format!(
                "PoC snapshot refuses `{relative}` because it exceeds {MAX_FILE_BYTES} bytes"
            ));
        }
        total = total.saturating_add(metadata.len);
        if total > MAX_WORKSPACE_BYTES {
            return Err(format!(
                "PoC snapshot exceeds the {MAX_WORKSPACE_BYTES}-byte limit"
            ));
        }
        let bytes = ctx.executor.read(&source).await?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        ctx.executor
            .write(&workspace.join(relative_path), &bytes)
            .await?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(super) fn process_spec(
    language: PocLanguage,
    workspace: &Path,
    script_path: &Path,
    temp_root: &Path,
) -> ProcessSpec {
    let process = match language {
        PocLanguage::Shell => {
            #[cfg(windows)]
            {
                ProcessSpec::argv("powershell.exe", workspace)
                    .args(powershell_script_args(script_path))
            }
            #[cfg(not(windows))]
            {
                ProcessSpec::argv("/bin/sh", workspace).args([script_path.as_os_str()])
            }
        }
        PocLanguage::Python => ProcessSpec::argv(
            if cfg!(windows) {
                "python.exe"
            } else {
                "python3"
            },
            workspace,
        )
        .args([script_path.as_os_str()]),
        PocLanguage::Javascript => {
            ProcessSpec::argv(if cfg!(windows) { "node.exe" } else { "node" }, workspace)
                .args([script_path.as_os_str()])
        }
    };
    process
        .env("HOME", temp_root.as_os_str())
        .env("TMPDIR", temp_root.as_os_str())
        .env("TMP", temp_root.as_os_str())
        .env("TEMP", temp_root.as_os_str())
        .env("CLARK_SECURITY_POC", "1")
}

pub(super) async fn run_bounded(
    process: &ProcessSpec,
    timeout: Duration,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<ExecOutput, String> {
    let mut child = spawn_process(process, Stdio::null(), Stdio::piped(), Stdio::piped())?;
    let root_pid = child.id();
    let _process_fence = ProcessFence::attach(root_pid);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, Vec<u8>)>();
    if let Some(mut pipe) = child.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut buffer = [0u8; 8192];
            while let Ok(count) = pipe.read(&mut buffer).await {
                if count == 0 || tx.send((false, buffer[..count].to_vec())).is_err() {
                    break;
                }
            }
        });
    }
    if let Some(mut pipe) = child.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut buffer = [0u8; 8192];
            while let Ok(count) = pipe.read(&mut buffer).await {
                if count == 0 || tx.send((true, buffer[..count].to_vec())).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut pipes_open = true;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                terminate_process_tree(&mut child, root_pid).await;
                return Err("cancelled".to_string());
            }
            _ = &mut deadline => {
                terminate_process_tree(&mut child, root_pid).await;
                return Err(format!("timed out after {} ms", timeout.as_millis()));
            }
            chunk = rx.recv(), if pipes_open => match chunk {
                Some((is_stderr, bytes)) => {
                    if stdout.len().saturating_add(stderr.len()).saturating_add(bytes.len())
                        > MAX_OUTPUT_BYTES
                    {
                        terminate_process_tree(&mut child, root_pid).await;
                        return Err(format!("output exceeded the {MAX_OUTPUT_BYTES}-byte limit"));
                    }
                    if is_stderr {
                        stderr.extend_from_slice(&bytes);
                    } else {
                        stdout.extend_from_slice(&bytes);
                    }
                }
                None => pipes_open = false,
            },
            status = child.wait(), if !pipes_open => {
                return status
                    .map(|status| ExecOutput {
                        stdout,
                        stderr,
                        code: status.code(),
                    })
                    .map_err(|error| format!("process wait failed: {error}"));
            }
        }
    }
}

pub(super) async fn persist_artifacts(
    ctx: &ToolCtx,
    run_root: &Path,
    receipt: &SecurityPocReceipt,
    output: &ExecOutput,
) -> Result<(), String> {
    let mut encoded = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("cannot encode PoC receipt: {error}"))?;
    encoded.push(b'\n');
    ctx.executor
        .write(&run_root.join("receipt.json"), &encoded)
        .await
        .map_err(|error| format!("cannot persist PoC receipt: {error}"))?;
    ctx.executor
        .write(&run_root.join("stdout.log"), &output.stdout)
        .await
        .map_err(|error| format!("cannot persist PoC stdout: {error}"))?;
    ctx.executor
        .write(&run_root.join("stderr.log"), &output.stderr)
        .await
        .map_err(|error| format!("cannot persist PoC stderr: {error}"))
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn preview(bytes: &[u8]) -> String {
    let end = bytes.len().min(MAX_OUTPUT_PREVIEW_BYTES);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

pub(super) fn control_label(control: SecurityPocControl) -> &'static str {
    match control {
        SecurityPocControl::Positive => "positive",
        SecurityPocControl::Negative => "negative",
    }
}

pub(super) fn language_label(language: PocLanguage) -> &'static str {
    match language {
        PocLanguage::Shell => "shell",
        PocLanguage::Python => "python",
        PocLanguage::Javascript => "javascript",
    }
}

pub(super) fn script_name(language: PocLanguage) -> &'static str {
    match language {
        PocLanguage::Shell => {
            if cfg!(windows) {
                "control.ps1"
            } else {
                "control.sh"
            }
        }
        PocLanguage::Python => "control.py",
        PocLanguage::Javascript => "control.mjs",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_runner_stops_output_floods() {
        let temp = tempfile::tempdir().unwrap();
        let process = ProcessSpec::argv("/bin/sh", temp.path())
            .args(["-c", "while :; do printf 1234567890; done"]);
        let error = run_bounded(
            &process,
            Duration::from_secs(5),
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("output exceeded"));
    }

    #[test]
    fn timeout_cap_stays_bounded() {
        assert_eq!(MAX_TIMEOUT_SECONDS, 60);
    }

    #[test]
    fn powershell_script_arguments_preserve_the_path_as_an_os_string() {
        let script = Path::new(r"C:\Clark Code\control.ps1");
        assert_eq!(
            powershell_script_args(script),
            [
                OsString::from("-NoLogo"),
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-File"),
                script.as_os_str().to_os_string(),
            ]
        );
    }
}
