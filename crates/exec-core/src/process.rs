use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::{
    configure_noninteractive, isolate_process_group, scripted_shell, terminate_pid_tree,
    terminate_process_tree, ExecOutput, ExecResult, OnOutput, ProcessFence, NONINTERACTIVE_ENV,
};

/// A host-native process request. Policy layers transform this value before the
/// local executor creates a child, keeping quoting and shell interpretation out
/// of sandbox adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
}

impl ProcessSpec {
    pub fn shell(command: &str, cwd: &Path) -> Self {
        let shell = scripted_shell(command);
        Self {
            program: shell.program,
            args: shell.args.into_iter().map(OsString::from).collect(),
            cwd: cwd.to_path_buf(),
            env: Vec::new(),
        }
    }

    pub fn argv(program: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: Vec::new(),
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

pub fn spawn_process(
    spec: &ProcessSpec,
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
) -> ExecResult<tokio::process::Child> {
    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .envs(spec.env.iter().cloned())
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr)
        .kill_on_drop(true);
    configure_noninteractive(&mut command);
    isolate_process_group(&mut command);
    command.spawn().map_err(|error| {
        format!(
            "failed to spawn {}: {error}",
            spec.program.to_string_lossy()
        )
    })
}

pub async fn run_process_streaming(
    spec: &ProcessSpec,
    timeout: Duration,
    cancel: &CancellationToken,
    on_output: OnOutput<'_>,
) -> ExecResult<ExecOutput> {
    use tokio::io::AsyncReadExt;

    let mut child = spawn_process(spec, Stdio::null(), Stdio::piped(), Stdio::piped())?;
    let root_pid = child.id();
    let _process_fence = ProcessFence::attach(root_pid);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, Vec<u8>)>();
    if let Some(mut pipe) = child.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut buf).await {
                if n == 0 || tx.send((false, buf[..n].to_vec())).is_err() {
                    break;
                }
            }
        });
    }
    if let Some(mut pipe) = child.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut buf).await {
                if n == 0 || tx.send((true, buf[..n].to_vec())).is_err() {
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
                return Err("command cancelled".into());
            }
            _ = &mut deadline => {
                terminate_process_tree(&mut child, root_pid).await;
                return Err(format!("command timed out after {} ms", timeout.as_millis()));
            }
            chunk = rx.recv(), if pipes_open => match chunk {
                Some((is_stderr, bytes)) => {
                    on_output(is_stderr, &bytes);
                    if is_stderr { stderr.extend_from_slice(&bytes); }
                    else { stdout.extend_from_slice(&bytes); }
                }
                None => pipes_open = false,
            },
            status = child.wait(), if !pipes_open => {
                return match status {
                    Ok(status) => Ok(ExecOutput { stdout, stderr, code: status.code() }),
                    Err(error) => Err(format!("command failed: {error}")),
                };
            }
        }
    }
}

pub async fn run_process_streaming_pty(
    spec: &ProcessSpec,
    timeout: Duration,
    cancel: &CancellationToken,
    on_output: OnOutput<'_>,
) -> ExecResult<ExecOutput> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::Read;

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("failed to open terminal: {error}"))?;
    let mut command = CommandBuilder::new(&spec.program);
    command.args(&spec.args);
    command.cwd(&spec.cwd);
    for (name, value) in NONINTERACTIVE_ENV {
        command.env(name, value);
    }
    for (name, value) in &spec.env {
        command.env(name, value);
    }

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("failed to spawn terminal command: {error}"))?;
    drop(pair.slave);
    let root_pid = child.process_id();
    let _process_fence = ProcessFence::attach(root_pid);
    let mut killer = child.clone_killer();
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("failed to read terminal output: {error}"))?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) if tx.send(buf[..n].to_vec()).is_err() => return,
                Ok(_) => {}
            }
        }
    });

    let mut wait = tokio::task::spawn_blocking(move || child.wait());
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut stdout = Vec::new();
    let mut output_open = true;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                terminate_pid_tree(root_pid).await;
                let _ = tokio::task::spawn_blocking(move || killer.kill()).await;
                let _ = tokio::time::timeout(Duration::from_millis(500), &mut wait).await;
                return Err("command cancelled".into());
            }
            _ = &mut deadline => {
                terminate_pid_tree(root_pid).await;
                let _ = tokio::task::spawn_blocking(move || killer.kill()).await;
                let _ = tokio::time::timeout(Duration::from_millis(500), &mut wait).await;
                return Err(format!("command timed out after {} ms", timeout.as_millis()));
            }
            chunk = rx.recv(), if output_open => match chunk {
                Some(bytes) => {
                    on_output(false, &bytes);
                    stdout.extend_from_slice(&bytes);
                }
                None => output_open = false,
            },
            status = &mut wait => {
                let status = status
                    .map_err(|error| format!("command task failed: {error}"))?
                    .map_err(|error| format!("command failed: {error}"))?;
                let drained = !output_open || tokio::time::timeout(Duration::from_millis(500), async {
                    while let Some(bytes) = rx.recv().await {
                        on_output(false, &bytes);
                        stdout.extend_from_slice(&bytes);
                    }
                }).await.is_ok();
                if !drained {
                    terminate_pid_tree(root_pid).await;
                }
                let code = status.signal().is_none()
                    .then(|| i32::try_from(status.exit_code()).ok())
                    .flatten();
                return Ok(ExecOutput { stdout, stderr: Vec::new(), code });
            }
        }
    }
}
