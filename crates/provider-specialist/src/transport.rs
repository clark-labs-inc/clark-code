use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::config::ConnectedConfig;
use crate::protocol::{WorkerRequest, WorkerResponse};

const RESPONSE_LIMIT_BYTES: u64 = 8 * 1024 * 1024;
const REQUEST_LIMIT_BYTES: usize = 1024 * 1024;
const STDERR_LIMIT_BYTES: u64 = 64 * 1024;
const TURN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct ExecutionLimits {
    response_bytes: u64,
    stderr_bytes: usize,
    turn_timeout: Duration,
    exit_timeout: Duration,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            response_bytes: RESPONSE_LIMIT_BYTES,
            stderr_bytes: STDERR_LIMIT_BYTES as usize,
            turn_timeout: TURN_TIMEOUT,
            exit_timeout: EXIT_TIMEOUT,
        }
    }
}

#[derive(Debug)]
pub enum WorkerFailure {
    Cancelled,
    TimedOut,
    Failed(String),
}

pub async fn execute(
    config: &ConnectedConfig,
    worker_config_path: &Path,
    request: &WorkerRequest,
    expected_kind: &str,
    cancel: &CancellationToken,
) -> Result<Value, WorkerFailure> {
    execute_with_limits(
        config,
        worker_config_path,
        request,
        expected_kind,
        cancel,
        ExecutionLimits::default(),
    )
    .await
}

async fn execute_with_limits(
    config: &ConnectedConfig,
    worker_config_path: &Path,
    request: &WorkerRequest,
    expected_kind: &str,
    cancel: &CancellationToken,
    limits: ExecutionLimits,
) -> Result<Value, WorkerFailure> {
    let line = encode_request_line(request)?;
    let executable = config
        .command
        .first()
        .ok_or_else(|| WorkerFailure::Failed("worker command is empty".into()))?;
    verify_worker_digest(Path::new(executable), &config.worker_sha256).await?;
    let mut command = Command::new(executable);
    command
        .args(&config.command[1..])
        .arg("--config")
        .arg(worker_config_path)
        .current_dir(&config.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    suppress_console_window(&mut command);
    // The specialist subprocess receives exactly one model credential through
    // the native provider contract. Do not let credentials from a developer
    // shell or a parent Clark process silently become alternate provider
    // routes inside the worker.
    for name in [
        "OPENROUTER_API_KEY",
        "CLARK_CODE_API_KEY",
        "CLARK_API_KEY",
        "CLARK_SPECIALIST_MODEL_KEY",
    ] {
        command.env_remove(name);
    }
    if let Some(model_key) = &config.model_key {
        command.env(config.child_key_env(), model_key);
    }
    let mut child = command.spawn().map_err(|error| {
        WorkerFailure::Failed(format!("could not start specialist worker: {error}"))
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| WorkerFailure::Failed("specialist worker stdin was unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| WorkerFailure::Failed("specialist worker stdout was unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| WorkerFailure::Failed("specialist worker stderr was unavailable".into()))?;
    let stderr_task =
        tokio::spawn(async move { bounded_diagnostics(stderr, limits.stderr_bytes).await });

    if let Err(error) = stdin.write_all(&line).await {
        let _ = child.kill().await;
        return Err(WorkerFailure::Failed(format!(
            "could not write specialist request: {error}"
        )));
    }
    if let Err(error) = stdin.flush().await {
        let _ = child.kill().await;
        return Err(WorkerFailure::Failed(format!(
            "could not flush specialist request: {error}"
        )));
    }

    let mut reader = BufReader::new(stdout).take(limits.response_bytes);
    let mut response_line = String::new();
    let read = tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = finish_diagnostics(stderr_task, limits.exit_timeout).await;
            return Err(WorkerFailure::Cancelled);
        }
        _ = tokio::time::sleep(limits.turn_timeout) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = finish_diagnostics(stderr_task, limits.exit_timeout).await;
            return Err(WorkerFailure::TimedOut);
        }
        result = reader.read_line(&mut response_line) => result,
    };
    let read = match read {
        Ok(read) => read,
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let stderr = finish_diagnostics(stderr_task, limits.exit_timeout).await;
            return Err(WorkerFailure::Failed(format!(
                "could not read specialist response: {error}; {}",
                concise_stderr(&stderr)
            )));
        }
    };
    drop(stdin);
    let exit_status = match tokio::time::timeout(limits.exit_timeout, child.wait()).await {
        Ok(Ok(status)) => Some(status),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let stderr = finish_diagnostics(stderr_task, limits.exit_timeout).await;
            return Err(WorkerFailure::Failed(format!(
                "could not wait for specialist worker: {error}; {}",
                concise_stderr(&stderr)
            )));
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            None
        }
    };
    let stderr = finish_diagnostics(stderr_task, limits.exit_timeout).await;
    if exit_status.is_none() {
        return Err(WorkerFailure::Failed(format!(
            "specialist worker did not exit after responding; {}",
            concise_stderr(&stderr)
        )));
    }
    if exit_status.is_some_and(|status| !status.success()) {
        return Err(WorkerFailure::Failed(format!(
            "specialist worker exited unsuccessfully; {}",
            concise_stderr(&stderr)
        )));
    }
    if read == 0 || response_line.trim().is_empty() {
        return Err(WorkerFailure::Failed(format!(
            "specialist worker exited without a response; {}",
            concise_stderr(&stderr)
        )));
    }
    let response = WorkerResponse::from_json_str(response_line.trim()).map_err(|error| {
        WorkerFailure::Failed(format!(
            "specialist worker returned invalid JSON: {error}; {}",
            concise_stderr(&stderr)
        ))
    })?;
    response
        .into_result(&request.request_id, expected_kind)
        .map_err(WorkerFailure::Failed)
}

fn encode_request_line(request: &WorkerRequest) -> Result<Vec<u8>, WorkerFailure> {
    let mut line = serde_json::to_vec(request).map_err(|error| {
        WorkerFailure::Failed(format!("could not encode worker request: {error}"))
    })?;
    line.push(b'\n');
    if line.len() > REQUEST_LIMIT_BYTES {
        return Err(WorkerFailure::Failed(format!(
            "specialist request exceeds {REQUEST_LIMIT_BYTES} bytes"
        )));
    }
    Ok(line)
}

/// A GUI parent must never surface the headless specialist as a console
/// window. On Windows, launching a console-subsystem sidecar without this
/// creation flag can make Windows Terminal open a visible tab.
fn suppress_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

async fn verify_worker_digest(path: &Path, expected: &str) -> Result<(), WorkerFailure> {
    let mut file = tokio::fs::File::open(path).await.map_err(|error| {
        WorkerFailure::Failed(format!(
            "could not open specialist worker for verification: {error}"
        ))
    })?;
    let mut digest = Sha256::new();
    let mut chunk = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut chunk).await.map_err(|error| {
            WorkerFailure::Failed(format!(
                "could not hash specialist worker before launch: {error}"
            ))
        })?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    let actual = format!("{:x}", digest.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(WorkerFailure::Failed(
            "specialist worker checksum changed before launch".into(),
        ));
    }
    Ok(())
}

async fn bounded_diagnostics(mut reader: impl AsyncRead + Unpin, limit: usize) -> String {
    let mut retained = Vec::with_capacity(limit.min(8 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let remaining = limit.saturating_sub(retained.len());
                retained.extend_from_slice(&chunk[..read.min(remaining)]);
            }
        }
    }
    String::from_utf8_lossy(&retained).into_owned()
}

async fn finish_diagnostics(
    mut task: tokio::task::JoinHandle<String>,
    timeout: Duration,
) -> String {
    tokio::select! {
        result = &mut task => result.unwrap_or_default(),
        _ = tokio::time::sleep(timeout) => {
            task.abort();
            let _ = task.await;
            "worker diagnostics stream did not close".into()
        }
    }
}

fn concise_stderr(stderr: &str) -> String {
    let value = stderr.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() {
        "no worker diagnostics".into()
    } else {
        value.chars().take(1_000).collect()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config::{ConnectedConfig, ModelRoute, SpecialistKind};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn config(temp: &tempfile::TempDir, body: &str) -> ConnectedConfig {
        let script = temp.path().join("worker.sh");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let worker_sha256 = format!("{:x}", Sha256::digest(std::fs::read(&script).unwrap()));
        ConnectedConfig {
            command: vec![script.to_string_lossy().into_owned()],
            cwd: temp.path().to_path_buf(),
            specialist: SpecialistKind::Scientist,
            workflow: "scientist:discover".into(),
            organization_id: None,
            workspace_id: None,
            scout_context: None,
            runtime_root: temp.path().join("runtime"),
            worker_sha256,
            project_id: "project".into(),
            model_route: ModelRoute::ClarkFree,
            max_iterations: 1,
            advisor_training_enabled: false,
            model_key: None,
            remote: None,
            remote_worker_binaries: Default::default(),
        }
    }

    fn ping() -> WorkerRequest {
        WorkerRequest {
            schema_version: 1,
            request_id: "request-1".into(),
            command: crate::protocol::WorkerCommand::Ping,
        }
    }

    fn test_limits() -> ExecutionLimits {
        ExecutionLimits {
            response_bytes: 32 * 1024,
            stderr_bytes: 1024,
            // Parallel macOS CI can take well over 150 ms just to schedule a
            // freshly spawned shell. Keep these tests bounded without making
            // process startup latency indistinguishable from protocol failure.
            turn_timeout: Duration::from_secs(2),
            exit_timeout: Duration::from_secs(2),
        }
    }

    #[tokio::test]
    async fn cancellation_kills_the_bounded_worker() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = config(&temp, "read request\nsleep 10");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = execute_with_limits(
            &config,
            &PathBuf::from("/ignored"),
            &ping(),
            "pong",
            &cancel,
            test_limits(),
        )
        .await;
        assert!(matches!(result, Err(WorkerFailure::Cancelled)));
    }

    #[tokio::test]
    async fn deadline_kills_a_silent_worker() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = config(&temp, "read request\nsleep 10");
        let result = execute_with_limits(
            &config,
            &PathBuf::from("/ignored"),
            &ping(),
            "pong",
            &CancellationToken::new(),
            test_limits(),
        )
        .await;
        assert!(matches!(result, Err(WorkerFailure::TimedOut)));
    }

    #[tokio::test]
    async fn malformed_output_fails_closed() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = config(&temp, "read request\nprintf 'not-json\\n'");
        let result = execute_with_limits(
            &config,
            &PathBuf::from("/ignored"),
            &ping(),
            "pong",
            &CancellationToken::new(),
            test_limits(),
        )
        .await;
        assert!(matches!(
            result,
            Err(WorkerFailure::Failed(message)) if message.contains("invalid JSON")
        ));
    }

    #[test]
    fn oversized_request_fails_before_worker_launch() {
        let request = WorkerRequest {
            schema_version: 1,
            request_id: "request-1".into(),
            command: crate::protocol::WorkerCommand::SpecialistTurn {
                session_id: "session-1".into(),
                specialist: "scientist".into(),
                workflow: "scientist:discover".into(),
                project_id: "project".into(),
                scout_context: None,
                message: "x".repeat(REQUEST_LIMIT_BYTES),
                now_ms: 1,
            },
        };
        assert!(matches!(
            encode_request_line(&request),
            Err(WorkerFailure::Failed(message)) if message.contains("exceeds")
        ));
    }

    #[tokio::test]
    async fn worker_tampering_after_native_registration_fails_before_launch() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = config(
            &temp,
            "read request\nprintf '%s\\n' '{\"type\":\"result\",\"schema_version\":1,\"request_id\":\"request-1\",\"kind\":\"pong\",\"data\":{}}'",
        );
        std::fs::write(&config.command[0], "#!/bin/sh\nexit 0\n").unwrap();
        let result = execute_with_limits(
            &config,
            &PathBuf::from("/ignored"),
            &ping(),
            "pong",
            &CancellationToken::new(),
            test_limits(),
        )
        .await;
        assert!(matches!(
            result,
            Err(WorkerFailure::Failed(message)) if message.contains("checksum changed")
        ));
    }

    #[tokio::test]
    async fn mismatched_request_identity_fails_closed() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = config(
            &temp,
            "read request\nprintf '%s\\n' '{\"type\":\"result\",\"schema_version\":1,\"request_id\":\"other\",\"kind\":\"pong\",\"data\":{}}'",
        );
        let result = execute_with_limits(
            &config,
            &PathBuf::from("/ignored"),
            &ping(),
            "pong",
            &CancellationToken::new(),
            test_limits(),
        )
        .await;
        assert!(matches!(
            result,
            Err(WorkerFailure::Failed(message)) if message.contains("request identity")
        ));
    }

    #[tokio::test]
    async fn nonzero_exit_rejects_an_apparently_valid_response() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = config(
            &temp,
            "read request\nprintf '%s\\n' '{\"type\":\"result\",\"schema_version\":1,\"request_id\":\"request-1\",\"kind\":\"pong\",\"data\":{}}'\nexit 7",
        );
        let result = execute_with_limits(
            &config,
            &PathBuf::from("/ignored"),
            &ping(),
            "pong",
            &CancellationToken::new(),
            test_limits(),
        )
        .await;
        assert!(matches!(
            result,
            Err(WorkerFailure::Failed(message)) if message.contains("exited unsuccessfully")
        ));
    }

    #[tokio::test]
    async fn worker_that_does_not_exit_after_response_fails_closed() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = config(
            &temp,
            "read request\nprintf '%s\\n' '{\"type\":\"result\",\"schema_version\":1,\"request_id\":\"request-1\",\"kind\":\"pong\",\"data\":{}}'\nsleep 10",
        );
        let result = execute_with_limits(
            &config,
            &PathBuf::from("/ignored"),
            &ping(),
            "pong",
            &CancellationToken::new(),
            test_limits(),
        )
        .await;
        assert!(matches!(
            result,
            Err(WorkerFailure::Failed(message)) if message.contains("did not exit")
        ));
    }
}
