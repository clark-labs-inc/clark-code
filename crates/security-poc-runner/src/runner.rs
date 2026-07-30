use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use exec_core::{
    spawn_process, terminate_process_tree, ExecOutput, Executor as _, ProcessFence, ProcessSpec,
};
use exec_sandbox::{SandboxPolicy, SandboxedExecutor};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::types::{
    sha256_hex, validate_id, PocControl, PocExecutionMetadata, PocLanguage, SecurityPocReceipt,
    SecurityPocRunRequest,
};

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_WORKSPACE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_SCRIPT_BYTES: usize = 256 * 1024;
const MAX_TIMEOUT_SECONDS: u64 = 60;
/// Mirrors the desktop contract version the receipt is validated against.
const CONTRACT_VERSION: u32 = 2;

/// Validate the request, then run the PoC control in a disposable, network-
/// denied workspace under `root`, returning the sealed receipt and raw output.
///
/// `root` is the repository root on this target; `request.run_root` is the
/// repository-relative artifact directory (e.g. `.clark/security-scans/…`).
pub async fn run(root: &Path, request: &SecurityPocRunRequest) -> Result<RunOutcome, String> {
    validate_request(request)?;
    let run_root = resolve_run_root(root, &request.run_root)?;
    let workspace = run_root.join("workspace");
    tokio::fs::create_dir_all(&workspace)
        .await
        .map_err(|error| format!("cannot create disposable PoC workspace: {error}"))?;
    let workspace_sha256 = stage_inventory(&request.inventory, &workspace).await?;

    let script_dir = workspace.join("__clark_poc__");
    tokio::fs::create_dir_all(&script_dir)
        .await
        .map_err(|error| format!("cannot create PoC script directory: {error}"))?;
    let script_path = script_dir.join(request.language.script_name());
    tokio::fs::write(&script_path, request.script.as_bytes())
        .await
        .map_err(|error| format!("cannot persist PoC script: {error}"))?;
    let temp_root = script_dir.join("tmp");
    tokio::fs::create_dir_all(&temp_root)
        .await
        .map_err(|error| format!("cannot create PoC temporary directory: {error}"))?;

    let policy = SandboxPolicy::read_only()
        .with_write_roots([workspace.clone()])
        .with_process_temp_root(temp_root.clone());
    let poc_executor = SandboxedExecutor::new(policy).map_err(|error| {
        format!("PoC refused because disposable OS containment is unavailable: {error}")
    })?;
    let process = process_spec(request.language, &workspace, &script_path, &temp_root);
    let process = poc_executor
        .prepare_process(process)
        .map_err(|error| format!("cannot sandbox PoC process: {error}"))?;

    let started_at_ms = epoch_ms()?;
    let cancel = tokio_util::sync::CancellationToken::new();
    let output = run_bounded(
        &process,
        Duration::from_secs(request.timeout_seconds),
        &cancel,
    )
    .await
    .map_err(|error| format!("PoC execution failed: {error}"))?;
    let completed_at_ms = epoch_ms()?;

    let receipt = build_receipt(
        request,
        &run_root,
        root,
        &workspace_sha256,
        &output,
        started_at_ms,
        completed_at_ms,
    )?;
    // Persist the receipt on the target so the artifact exists where the run
    // happened; the caller additionally writes stdout/stderr logs.
    persist_receipt(&run_root, &receipt).await?;
    Ok(RunOutcome {
        receipt,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

#[derive(Debug)]
pub struct RunOutcome {
    pub receipt: SecurityPocReceipt,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

fn validate_request(request: &SecurityPocRunRequest) -> Result<(), String> {
    validate_id("scan_id", &request.scan_id)?;
    validate_id("candidate_id", &request.candidate_id)?;
    validate_id("inventory_id", &request.inventory_id)?;
    if request.expected_observation.trim().is_empty() {
        return Err("expected_observation must not be empty".into());
    }
    if request.script.is_empty() || request.script.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "script must contain between 1 and {MAX_SCRIPT_BYTES} bytes"
        ));
    }
    if request.timeout_seconds == 0 || request.timeout_seconds > MAX_TIMEOUT_SECONDS {
        return Err(format!(
            "timeout_seconds must be between 1 and {MAX_TIMEOUT_SECONDS}"
        ));
    }
    Ok(())
}

/// Join `run_root` under `root`, refusing absolute paths and any escape.
fn resolve_run_root(root: &Path, run_root: &str) -> Result<PathBuf, String> {
    let relative = Path::new(run_root);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "PoC run root `{run_root}` is not a safe relative path"
        ));
    }
    Ok(root.join(relative))
}

/// Stage the inventory snapshot into the disposable workspace, returning the
/// content digest of exactly what was written (the receipt's `workspace_sha256`).
async fn stage_inventory(
    inventory: &[crate::types::PocInventoryFile],
    workspace: &Path,
) -> Result<String, String> {
    let mut total = 0u64;
    let mut hasher = Sha256::new();
    for file in inventory {
        let relative_path = Path::new(&file.path);
        if relative_path.is_absolute()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(format!("inventory contains unsafe path `{}`", file.path));
        }
        if file.bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!(
                "PoC snapshot refuses `{}` because it exceeds {MAX_FILE_BYTES} bytes",
                file.path
            ));
        }
        total = total.saturating_add(file.bytes.len() as u64);
        if total > MAX_WORKSPACE_BYTES {
            return Err(format!(
                "PoC snapshot exceeds the {MAX_WORKSPACE_BYTES}-byte limit"
            ));
        }
        hasher.update(file.path.as_bytes());
        hasher.update([0]);
        hasher.update((file.bytes.len() as u64).to_le_bytes());
        hasher.update(&file.bytes);
        let dest = workspace.join(relative_path);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("cannot stage PoC inventory: {error}"))?;
        }
        tokio::fs::write(&dest, &file.bytes)
            .await
            .map_err(|error| format!("cannot stage PoC inventory: {error}"))?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn process_spec(
    language: PocLanguage,
    workspace: &Path,
    script_path: &Path,
    temp_root: &Path,
) -> ProcessSpec {
    let process = match language {
        PocLanguage::Shell => {
            #[cfg(windows)]
            {
                ProcessSpec::argv("powershell.exe", workspace).args([
                    std::ffi::OsString::from("-NoLogo"),
                    std::ffi::OsString::from("-NoProfile"),
                    std::ffi::OsString::from("-NonInteractive"),
                    std::ffi::OsString::from("-File"),
                    script_path.as_os_str().to_os_string(),
                ])
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

async fn run_bounded(
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

fn build_receipt(
    request: &SecurityPocRunRequest,
    run_root: &Path,
    root: &Path,
    workspace_sha256: &str,
    output: &ExecOutput,
    started_at_ms: i64,
    completed_at_ms: i64,
) -> Result<SecurityPocReceipt, String> {
    let relative_run_root = run_root
        .strip_prefix(root)
        .map_err(|_| "PoC run root escaped the repository root".to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let artifact_path = format!("{relative_run_root}/receipt.json");
    let script_artifact_path = format!(
        "{relative_run_root}/workspace/__clark_poc__/{}",
        request.language.script_name()
    );
    let control_label = match request.control {
        PocControl::Positive => "positive",
        PocControl::Negative => "negative",
    };
    let _ = control_label;
    let mut receipt = SecurityPocReceipt {
        contract_version: CONTRACT_VERSION,
        receipt_id: String::new(),
        scan_id: request.scan_id.clone(),
        candidate_id: request.candidate_id.clone(),
        inventory_id: request.inventory_id.clone(),
        control: request.control,
        language: request.language.label().to_string(),
        script_sha256: sha256_hex(request.script.as_bytes()),
        expected_observation_sha256: sha256_hex(request.expected_observation.as_bytes()),
        workspace_sha256: workspace_sha256.to_string(),
        stdout_sha256: sha256_hex(&output.stdout),
        stderr_sha256: sha256_hex(&output.stderr),
        expected_exit_code: request.expected_exit_code,
        exit_code: output.code,
        passed: output.code == Some(request.expected_exit_code),
        containment: "managed_disposable".to_string(),
        artifact_path: artifact_path.clone(),
        execution: Some(PocExecutionMetadata {
            expected_observation: request.expected_observation.clone(),
            started_at_ms,
            completed_at_ms,
            timeout_ms: request.timeout_seconds.saturating_mul(1_000),
            output_limit_bytes: MAX_OUTPUT_BYTES as u64,
            sandbox_provider: "clark-desktop-native".into(),
            sandbox_profile_sha256: sha256_hex(
                format!(
                    "clark-security-native-sandbox/v1\0{}\0{}\0offline\0disposable-write-root",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )
                .as_bytes(),
            ),
            script_path: script_artifact_path,
            stdout_path: format!("{relative_run_root}/stdout.log"),
            stderr_path: format!("{relative_run_root}/stderr.log"),
        }),
    };
    let preimage = serde_json::to_vec(&receipt)
        .map_err(|error| format!("cannot encode PoC receipt: {error}"))?;
    receipt.receipt_id = format!("poc-{}", &sha256_hex(&preimage)[..32]);
    Ok(receipt)
}

async fn persist_receipt(run_root: &Path, receipt: &SecurityPocReceipt) -> Result<(), String> {
    let mut encoded = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("cannot encode PoC receipt: {error}"))?;
    encoded.push(b'\n');
    tokio::fs::write(run_root.join("receipt.json"), &encoded)
        .await
        .map_err(|error| format!("cannot persist PoC receipt: {error}"))
}

fn epoch_ms() -> Result<i64, String> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?;
    i64::try_from(duration.as_millis())
        .map_err(|_| "system clock exceeds the Clark Security timestamp range".to_string())
}
