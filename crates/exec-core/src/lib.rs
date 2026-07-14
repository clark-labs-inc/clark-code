//! The execution backend abstraction — the seam that lets the agent's tools run
//! against the **local** machine or a **remote** host (over the exec-server),
//! without the tools or engine knowing which.
//!
//! Mirrors codex's split of `ExecBackend` (processes) + `ExecutorFileSystem`
//! (files) into one trait here. Every coding tool (`read_file`, `write_file`,
//! `edit_file`, `list_dir`, `glob`, `grep`, `bash`) performs its I/O through an
//! [`Executor`], instead of touching `std::fs` / `tokio::process` directly.
//! [`LocalExecutor`] runs the primitives on this machine; the remote
//! `exec-server` is "`LocalExecutor` wrapped in a WebSocket server", and the
//! provider's `RemoteExecutor` forwards the same primitives to it.
//!
//! This crate is deliberately small (no HTTP, no `agent-core`) so the remote
//! `clark-exec-server` binary that links it stays lean.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

pub fn isolate_process_group(command: &mut tokio::process::Command) {
    #[cfg(unix)]
    command.process_group(0);
}

pub async fn terminate_pid_tree(root_pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = root_pid.and_then(|pid| i32::try_from(pid).ok()) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }

    #[cfg(windows)]
    if let Some(pid) = root_pid {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

pub async fn terminate_process_tree(child: &mut tokio::process::Child, root_pid: Option<u32>) {
    terminate_pid_tree(root_pid).await;
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Tool-facing result: the error is already a model-readable message.
pub type ExecResult<T> = Result<T, String>;

/// One entry returned by [`Executor::read_dir`].
#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Metadata for a path.
#[derive(Clone, Copy, Debug)]
pub struct FileMeta {
    pub modified: Option<SystemTime>,
    pub len: u64,
    pub is_dir: bool,
}

/// A file discovered by [`Executor::walk`] (files only; ignored dirs skipped).
#[derive(Clone, Debug)]
pub struct WalkEntry {
    /// Absolute path on the target machine.
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub len: u64,
}

/// The captured result of running a command.
#[derive(Clone, Debug)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Process exit code, or `None` if it was terminated by a signal.
    pub code: Option<i32>,
}

/// Incremental output callback for [`Executor::exec_streaming`]:
/// `(is_stderr, chunk_bytes)`, invoked as the process writes.
pub type OnOutput<'a> = &'a (dyn Fn(bool, &[u8]) + Send + Sync);

/// Directories never worth walking — keeps `glob`/`grep`/file-listing fast and
/// out of build artifacts and vendored deps. Shared by every walk so local and
/// remote agree on what's in scope.
pub fn is_ignored(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_string_lossy().as_ref(),
            ".git" | "node_modules" | "target" | "dist" | ".next" | ".venv"
        )
    })
}

/// The filesystem + process primitives the tools need, targeting either the
/// local machine ([`LocalExecutor`]) or a remote host.
#[async_trait]
pub trait Executor: Send + Sync {
    /// Read a file's bytes.
    async fn read(&self, path: &Path) -> ExecResult<Vec<u8>>;
    /// Write bytes to a file, creating parent directories as needed.
    async fn write(&self, path: &Path, data: &[u8]) -> ExecResult<()>;
    /// Create a directory and all missing parents.
    async fn create_dir_all(&self, path: &Path) -> ExecResult<()>;
    /// List a directory's immediate entries.
    async fn read_dir(&self, path: &Path) -> ExecResult<Vec<DirEntry>>;
    /// Metadata for a path; `Err` if it doesn't exist / can't be stat'd.
    async fn metadata(&self, path: &Path) -> ExecResult<FileMeta>;
    /// Modification time, or `None` if the path can't be stat'd. (Convenience
    /// over [`metadata`](Executor::metadata) for the read-before-edit tracker.)
    async fn mtime(&self, path: &Path) -> Option<SystemTime> {
        self.metadata(path).await.ok().and_then(|m| m.modified)
    }
    /// Recursively list files under `root`, skipping ignored directories
    /// (`.git`, `node_modules`, `target`, …). Files only.
    async fn walk(&self, root: &Path) -> ExecResult<Vec<WalkEntry>>;
    /// Run `command` through `/bin/sh -c` at `cwd`, capturing stdout/stderr/code.
    /// Honors `cancel` (kills the process) and `timeout`. `Err` for spawn
    /// failure / cancellation / timeout; `Ok` even on a non-zero exit.
    async fn exec(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> ExecResult<ExecOutput>;

    /// Like [`exec`](Executor::exec), but also surfaces stdout/stderr chunks
    /// through `on_output` as the process produces them, so long commands can
    /// show live progress. The default just runs `exec` (correct, not live);
    /// executors that can stream override it.
    async fn exec_streaming(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
        _on_output: OnOutput<'_>,
    ) -> ExecResult<ExecOutput> {
        self.exec(command, cwd, timeout, cancel).await
    }

    async fn exec_streaming_pty(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
        on_output: OnOutput<'_>,
    ) -> ExecResult<ExecOutput> {
        self.exec_streaming(command, cwd, timeout, cancel, on_output)
            .await
    }

    /// Whether this executor runs on the same machine as the caller. Local
    /// tools that need to spawn a process directly (not through `exec()`) —
    /// e.g. a backgrounded shell task — check this first, since only a local
    /// process is reachable to poll/kill afterward.
    fn is_local(&self) -> bool {
        true
    }
}

/// Runs every primitive on the local machine — today's behavior, behind the
/// [`Executor`] trait. The remote `exec-server` delegates to this same impl.
pub struct LocalExecutor;

#[async_trait]
impl Executor for LocalExecutor {
    async fn read(&self, path: &Path) -> ExecResult<Vec<u8>> {
        tokio::fs::read(path).await.map_err(|e| e.to_string())
    }

    async fn write(&self, path: &Path, data: &[u8]) -> ExecResult<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::write(path, data)
            .await
            .map_err(|e| e.to_string())
    }

    async fn create_dir_all(&self, path: &Path) -> ExecResult<()> {
        tokio::fs::create_dir_all(path)
            .await
            .map_err(|e| e.to_string())
    }

    async fn read_dir(&self, path: &Path) -> ExecResult<Vec<DirEntry>> {
        let mut rd = tokio::fs::read_dir(path).await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(entry) = rd.next_entry().await.map_err(|e| e.to_string())? {
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            out.push(DirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir,
            });
        }
        Ok(out)
    }

    async fn metadata(&self, path: &Path) -> ExecResult<FileMeta> {
        let m = tokio::fs::metadata(path).await.map_err(|e| e.to_string())?;
        Ok(FileMeta {
            modified: m.modified().ok(),
            len: m.len(),
            is_dir: m.is_dir(),
        })
    }

    async fn walk(&self, root: &Path) -> ExecResult<Vec<WalkEntry>> {
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for entry in walkdir::WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| !is_ignored(e.path()))
            {
                let Ok(entry) = entry else { continue };
                if !entry.file_type().is_file() {
                    continue;
                }
                let (modified, len) = entry
                    .metadata()
                    .map(|m| (m.modified().ok(), m.len()))
                    .unwrap_or((None, 0));
                out.push(WalkEntry {
                    path: entry.path().to_path_buf(),
                    modified,
                    len,
                });
            }
            out
        })
        .await
        .map_err(|e| format!("walk failed: {e}"))
    }

    async fn exec(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> ExecResult<ExecOutput> {
        self.exec_streaming(command, cwd, timeout, cancel, &|_, _| {})
            .await
    }

    async fn exec_streaming(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
        on_output: OnOutput<'_>,
    ) -> ExecResult<ExecOutput> {
        use tokio::io::AsyncReadExt;

        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        isolate_process_group(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn shell: {e}"))?;
        let root_pid = child.id();

        // Pipe readers run as tasks feeding one channel, so both pipes drain
        // concurrently (no deadlock on a full pipe) while this future observes
        // every chunk in arrival order.
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
                        if is_stderr {
                            stderr.extend_from_slice(&bytes);
                        } else {
                            stdout.extend_from_slice(&bytes);
                        }
                    }
                    None => pipes_open = false,
                },
                status = child.wait(), if !pipes_open => {
                    return match status {
                        Ok(status) => Ok(ExecOutput { stdout, stderr, code: status.code() }),
                        Err(e) => Err(format!("command failed: {e}")),
                    };
                }
            }
        }
    }

    async fn exec_streaming_pty(
        &self,
        command: &str,
        cwd: &Path,
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
            .map_err(|e| format!("failed to open terminal: {e}"))?;

        #[cfg(unix)]
        let mut cmd = {
            let mut cmd = CommandBuilder::new("/bin/sh");
            cmd.args(["-c", command]);
            cmd
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut cmd = CommandBuilder::new("cmd.exe");
            cmd.args(["/C", command]);
            cmd
        };
        cmd.cwd(cwd);
        cmd.env("TERM", "dumb");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("failed to spawn terminal command: {e}"))?;
        drop(pair.slave);
        let root_pid = child.process_id();
        let mut killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("failed to read terminal output: {e}"))?;
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
                        .map_err(|e| format!("command task failed: {e}"))?
                        .map_err(|e| format!("command failed: {e}"))?;
                    let drained = !output_open || tokio::time::timeout(Duration::from_millis(500), async {
                            while let Some(bytes) = rx.recv().await {
                                on_output(false, &bytes);
                                stdout.extend_from_slice(&bytes);
                            }
                        })
                        .await
                        .is_ok();
                    if !drained {
                        terminate_pid_tree(root_pid).await;
                    }
                    let code = status
                        .signal()
                        .is_none()
                        .then(|| i32::try_from(status.exit_code()).ok())
                        .flatten();
                    return Ok(ExecOutput { stdout, stderr: Vec::new(), code });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exec_streaming_surfaces_chunks_and_captures_output() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(bool, Vec<u8>)>::new()));
        let sink = seen.clone();
        let out = exec
            .exec_streaming(
                "printf out; printf err 1>&2",
                dir.path(),
                Duration::from_secs(10),
                &CancellationToken::new(),
                &move |is_stderr, chunk| sink.lock().unwrap().push((is_stderr, chunk.to_vec())),
            )
            .await
            .unwrap();
        assert_eq!(out.stdout, b"out");
        assert_eq!(out.stderr, b"err");
        assert_eq!(out.code, Some(0));
        let seen = seen.lock().unwrap();
        let stdout_stream: Vec<u8> = seen
            .iter()
            .filter(|(e, _)| !*e)
            .flat_map(|(_, c)| c.clone())
            .collect();
        let stderr_stream: Vec<u8> = seen
            .iter()
            .filter(|(e, _)| *e)
            .flat_map(|(_, c)| c.clone())
            .collect();
        assert_eq!(stdout_stream, b"out");
        assert_eq!(stderr_stream, b"err");
    }

    #[tokio::test]
    async fn exec_streaming_still_honors_timeout_and_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        let err = exec
            .exec_streaming(
                "sleep 5",
                dir.path(),
                Duration::from_millis(50),
                &CancellationToken::new(),
                &|_, _| {},
            )
            .await
            .unwrap_err();
        assert!(err.contains("timed out"), "{err}");

        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = exec
            .exec_streaming(
                "sleep 5",
                dir.path(),
                Duration::from_secs(10),
                &cancel,
                &|_, _| {},
            )
            .await
            .unwrap_err();
        assert!(err.contains("cancelled"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_descendants_that_hold_output_pipes_open() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        let err = tokio::time::timeout(
            Duration::from_secs(2),
            exec.exec_streaming(
                "sleep 30 & echo $! > descendant.pid; wait",
                dir.path(),
                Duration::from_millis(150),
                &CancellationToken::new(),
                &|_, _| {},
            ),
        )
        .await
        .expect("executor must return after its own timeout")
        .unwrap_err();
        assert!(err.contains("timed out"), "{err}");

        let pid: i32 = std::fs::read_to_string(dir.path().join("descendant.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..20 {
            if unsafe { libc::kill(pid, 0) } == -1 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("descendant process {pid} survived command timeout");
    }

    #[tokio::test]
    async fn pty_execution_exposes_a_terminal_and_streams_output() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let out = exec
            .exec_streaming_pty(
                "test -t 0 && test -t 1 && printf terminal",
                dir.path(),
                Duration::from_secs(10),
                &CancellationToken::new(),
                &move |_, chunk| sink.lock().unwrap().extend_from_slice(chunk),
            )
            .await
            .unwrap();
        assert_eq!(out.code, Some(0));
        assert!(String::from_utf8_lossy(&out.stdout).contains("terminal"));
        assert!(String::from_utf8_lossy(&seen.lock().unwrap()).contains("terminal"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pty_timeout_kills_the_terminal_process_tree() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        let err = tokio::time::timeout(
            Duration::from_secs(2),
            exec.exec_streaming_pty(
                "sleep 30 & echo $! > pty-descendant.pid; wait",
                dir.path(),
                Duration::from_millis(150),
                &CancellationToken::new(),
                &|_, _| {},
            ),
        )
        .await
        .expect("terminal executor must return after its own timeout")
        .unwrap_err();
        assert!(err.contains("timed out"), "{err}");

        let pid: i32 = std::fs::read_to_string(dir.path().join("pty-descendant.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..20 {
            if unsafe { libc::kill(pid, 0) } == -1 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("terminal descendant process {pid} survived command timeout");
    }

    #[tokio::test]
    async fn local_read_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        let p = dir.path().join("a/b.txt");
        exec.write(&p, b"hello").await.unwrap();
        assert_eq!(exec.read(&p).await.unwrap(), b"hello");
        let m = exec.metadata(&p).await.unwrap();
        assert_eq!(m.len, 5);
        assert!(!m.is_dir);
    }

    #[tokio::test]
    async fn local_read_dir_and_walk_skip_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        std::fs::write(dir.path().join("top.rs"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
        std::fs::write(dir.path().join("node_modules/x/y.js"), "").unwrap();

        let entries = exec.read_dir(dir.path()).await.unwrap();
        assert!(entries.iter().any(|e| e.name == "src" && e.is_dir));
        assert!(entries.iter().any(|e| e.name == "top.rs" && !e.is_dir));

        let files: Vec<_> = exec
            .walk(dir.path())
            .await
            .unwrap()
            .into_iter()
            .map(|w| w.path.to_string_lossy().to_string())
            .collect();
        assert!(files.iter().any(|f| f.ends_with("src/main.rs")));
        assert!(files.iter().any(|f| f.ends_with("top.rs")));
        assert!(!files.iter().any(|f| f.contains("node_modules")));
    }

    #[tokio::test]
    async fn local_exec_captures_output_and_code() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        let out = exec
            .exec(
                "echo hi; exit 3",
                dir.path(),
                Duration::from_secs(10),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
        assert_eq!(out.code, Some(3));
    }

    #[tokio::test]
    async fn local_exec_honors_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let exec = LocalExecutor;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = exec
            .exec("sleep 5", dir.path(), Duration::from_secs(10), &cancel)
            .await
            .unwrap_err();
        assert!(err.contains("cancel"));
    }
}
