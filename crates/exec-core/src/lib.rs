//! The execution backend abstraction — the seam that lets the agent's tools run
//! against the **local** machine or a **remote** host (over the exec-server),
//! without the tools or engine knowing which.
//!
//! Separates `ExecBackend` (processes) from `ExecutorFileSystem`
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
#[cfg(windows)]
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

mod capabilities;
mod local;
mod process;
mod process_fence;
pub use capabilities::{collect_system_capabilities, SystemCapabilityCensus};
pub use local::LocalExecutor;
pub use process::{run_process_streaming, run_process_streaming_pty, spawn_process, ProcessSpec};
pub use process_fence::ProcessFence;

pub const NONINTERACTIVE_ENV: &[(&str, &str)] = &[
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_TERMINAL_PROMPT", "0"),
    // Repository-selected fsmonitor executables can hang every innocent Git
    // command. Apply the same safe override to model-issued shell commands as
    // Clark's internal Git probes, including commands run on remote executors.
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "core.fsmonitor"),
    ("GIT_CONFIG_VALUE_0", "false"),
    ("TERM", "dumb"),
    ("NO_COLOR", "1"),
];

/// The command interpreter selected for locally executed scripts.
///
/// Windows prefers PowerShell, which is available on
/// supported Windows versions. CMD remains a profile-free fallback when a
/// PowerShell executable cannot be resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellKind {
    Posix,
    PowerShell,
    Cmd,
}

/// A shell executable plus the arguments required to run it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub kind: ShellKind,
}

/// Resolve a non-interactive shell for one script.
pub fn scripted_shell(command: &str) -> ShellInvocation {
    #[cfg(windows)]
    {
        let (kind, program) = default_windows_shell();
        ShellInvocation {
            program,
            args: windows_shell_args(kind, Some(command)),
            kind,
        }
    }

    #[cfg(not(windows))]
    {
        ShellInvocation {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), command.to_string()],
            kind: ShellKind::Posix,
        }
    }
}

/// Resolve an interactive shell for the embedded terminal.
pub fn interactive_shell() -> ShellInvocation {
    #[cfg(windows)]
    {
        let (kind, program) = default_windows_shell();
        ShellInvocation {
            program,
            args: windows_shell_args(kind, None),
            kind,
        }
    }

    #[cfg(not(windows))]
    {
        ShellInvocation {
            program: std::env::var_os("SHELL")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/bin/bash")),
            args: vec!["-l".to_string()],
            kind: ShellKind::Posix,
        }
    }
}

/// Return the platform's current non-interactive shell kind without spawning a
/// child. Callers that construct shell syntax themselves use this to quote or
/// prefix commands correctly.
pub fn scripted_shell_kind() -> ShellKind {
    #[cfg(windows)]
    {
        default_windows_shell().0
    }

    #[cfg(not(windows))]
    {
        ShellKind::Posix
    }
}

#[cfg(any(windows, test))]
fn windows_shell_args(kind: ShellKind, command: Option<&str>) -> Vec<String> {
    let mut args = match kind {
        ShellKind::PowerShell => vec!["-NoLogo".to_string(), "-NoProfile".to_string()],
        ShellKind::Cmd => vec!["/D".to_string(), "/Q".to_string()],
        ShellKind::Posix => unreachable!("Windows cannot select a POSIX shell"),
    };
    if let Some(command) = command {
        match kind {
            ShellKind::PowerShell => {
                args.push("-NonInteractive".to_string());
                args.push("-Command".to_string());
            }
            ShellKind::Cmd => args.push("/C".to_string()),
            ShellKind::Posix => unreachable!("Windows cannot select a POSIX shell"),
        }
        args.push(command.to_string());
    }
    args
}

#[cfg(windows)]
fn default_windows_shell() -> (ShellKind, PathBuf) {
    const PWSH_FALLBACK: &str = r"C:\Program Files\PowerShell\7\pwsh.exe";
    const POWERSHELL_FALLBACK: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

    find_windows_executable("pwsh.exe", Some(PWSH_FALLBACK))
        .map(|path| (ShellKind::PowerShell, path))
        .or_else(|| {
            find_windows_executable("powershell.exe", Some(POWERSHELL_FALLBACK))
                .map(|path| (ShellKind::PowerShell, path))
        })
        .unwrap_or_else(|| {
            (
                ShellKind::Cmd,
                std::env::var_os("COMSPEC")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("cmd.exe")),
            )
        })
}

#[cfg(windows)]
fn find_windows_executable(name: &str, fallback: Option<&str>) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    });
    on_path.or_else(|| fallback.map(PathBuf::from).filter(|path| path.is_file()))
}

pub fn configure_noninteractive(command: &mut tokio::process::Command) {
    command.envs(NONINTERACTIVE_ENV.iter().copied());
}

pub fn isolate_process_group(command: &mut tokio::process::Command) {
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(not(unix))]
    let _ = command;
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
    pub is_symlink: bool,
}

/// Metadata for a path.
#[derive(Clone, Copy, Debug)]
pub struct FileMeta {
    pub modified: Option<SystemTime>,
    pub len: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
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

/// Who owns containment for process and filesystem operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionContainment {
    /// The host executes directly; callers must classify this as a trusted host
    /// capability rather than an agent sandbox.
    Host,
    /// This executor applies a local OS sandbox and matching filesystem checks.
    Managed,
    /// A remote executor is responsible for enforcing its own boundary.
    External,
}

/// One ordered output chunk from a long-lived process.
#[derive(Clone, Debug)]
pub struct BackgroundOutput {
    pub seq: u64,
    pub is_stderr: bool,
    pub data: Vec<u8>,
}

/// Incremental status for a long-lived process. Callers pass their last cursor
/// and receive only newer chunks.
#[derive(Clone, Debug)]
pub struct BackgroundStatus {
    pub output: Vec<BackgroundOutput>,
    pub exit_code: Option<Option<i32>>,
    pub error: Option<String>,
    pub cursor: u64,
    pub truncated: bool,
}

/// Incremental output callback for [`Executor::exec_streaming`]:
/// `(is_stderr, chunk_bytes)`, invoked as the process writes.
pub type OnOutput<'a> = &'a (dyn Fn(bool, &[u8]) + Send + Sync);

/// Directories never worth walking in addition to repository ignore files —
/// keeps `glob`/`grep`/file-listing out of build artifacts and vendored deps.
/// Shared by every walk so local and remote agree on what's in scope.
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
    /// Enumerate executable names, environment-variable names, and known
    /// credential surfaces without executing discovered programs or reading
    /// credential values.
    async fn system_capability_census(&self) -> ExecResult<SystemCapabilityCensus> {
        Ok(collect_system_capabilities(None))
    }

    /// Read a file's bytes.
    async fn read(&self, path: &Path) -> ExecResult<Vec<u8>>;
    /// Write bytes to a file, creating parent directories as needed.
    async fn write(&self, path: &Path, data: &[u8]) -> ExecResult<()>;
    /// Create a directory and all missing parents.
    async fn create_dir_all(&self, path: &Path) -> ExecResult<()>;
    /// Remove one file or symlink. Missing paths are treated as success.
    async fn remove_file(&self, path: &Path) -> ExecResult<()>;
    /// Remove one directory tree. Missing paths are treated as success.
    async fn remove_dir_all(&self, path: &Path) -> ExecResult<()>;
    /// Atomically rename one path on the target filesystem when source and
    /// destination share a filesystem.
    async fn rename(&self, from: &Path, to: &Path) -> ExecResult<()>;
    /// List a directory's immediate entries.
    async fn read_dir(&self, path: &Path) -> ExecResult<Vec<DirEntry>>;
    /// Metadata for a path; `Err` if it doesn't exist / can't be stat'd.
    async fn metadata(&self, path: &Path) -> ExecResult<FileMeta>;
    /// Resolve a path through directory/file symlinks on the target filesystem.
    ///
    /// Callers use the returned target-owned absolute path as filesystem
    /// identity. This is deliberately an executor primitive so local and remote
    /// discovery make identical cycle and alias decisions.
    async fn canonicalize(&self, path: &Path) -> ExecResult<PathBuf>;
    /// Target environment's user home. This must describe the executor target,
    /// not the desktop process hosting a remote executor.
    async fn home_dir(&self, cwd: &Path) -> ExecResult<PathBuf>;
    /// Modification time, or `None` if the path can't be stat'd. (Convenience
    /// over [`metadata`](Executor::metadata) for the read-before-edit tracker.)
    async fn mtime(&self, path: &Path) -> Option<SystemTime> {
        self.metadata(path).await.ok().and_then(|m| m.modified)
    }
    /// Recursively list files under `root`, honoring repository ignore files
    /// and skipping fixed noisy directories (`.git`, `node_modules`, …).
    async fn walk(&self, root: &Path) -> ExecResult<Vec<WalkEntry>>;

    /// Transform an argv-shaped process before it is spawned. Agent-owned
    /// subprocesses that cannot use [`Executor::exec`] directly (background
    /// tasks, pinned helpers, stdio transports) must call this hook.
    fn prepare_process(&self, process: ProcessSpec) -> ExecResult<ProcessSpec> {
        Ok(process)
    }

    fn containment(&self) -> ExecutionContainment {
        ExecutionContainment::Host
    }
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

    /// Start a target-owned long-lived process and return its opaque id.
    async fn background_start(&self, _command: &str, _cwd: &Path) -> ExecResult<String> {
        Err("background processes are not supported by this executor".into())
    }

    /// Poll output and terminal state after `after_seq`.
    async fn background_status(
        &self,
        _process_id: &str,
        _after_seq: u64,
    ) -> ExecResult<BackgroundStatus> {
        Err("background processes are not supported by this executor".into())
    }

    /// Write bytes to a long-lived process, or close its stdin.
    async fn background_write(
        &self,
        _process_id: &str,
        _data: &[u8],
        _close: bool,
    ) -> ExecResult<()> {
        Err("background process input is not supported by this executor".into())
    }

    /// Stop a target-owned long-lived process.
    async fn background_kill(&self, _process_id: &str) -> ExecResult<()> {
        Err("background processes are not supported by this executor".into())
    }

    /// Whether this executor runs on the same machine as the caller. Local
    /// tools that need to spawn a process directly (not through `exec()`) —
    /// e.g. a backgrounded shell task — check this first, since only a local
    /// process is reachable to poll/kill afterward.
    fn is_local(&self) -> bool {
        true
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
        #[cfg(windows)]
        let command = "[Console]::Out.Write('out'); [Console]::Error.Write('err')";
        #[cfg(not(windows))]
        let command = "printf out; printf err 1>&2";
        let out = exec
            .exec_streaming(
                command,
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
        #[cfg(windows)]
        let command = "if ([Console]::IsInputRedirected -or [Console]::IsOutputRedirected) { exit 7 }; [Console]::Out.Write('terminal')";
        #[cfg(not(windows))]
        let command = "test -t 0 && test -t 1 && printf terminal";
        let out = exec
            .exec_streaming_pty(
                command,
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

    #[test]
    fn pty_builder_preserves_the_exact_process_path() {
        let Some(path) = std::env::var_os("PATH") else {
            return;
        };
        let mut command = portable_pty::CommandBuilder::new("unused");
        crate::process::overlay_process_environment(&mut command);
        assert_eq!(command.get_env("PATH"), Some(path.as_os_str()));
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
        std::fs::write(dir.path().join(".gitignore"), "vendor/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("vendor/generated")).unwrap();
        std::fs::write(dir.path().join("vendor/generated/decoy.rs"), "").unwrap();

        let entries = exec.read_dir(dir.path()).await.unwrap();
        assert!(entries.iter().any(|e| e.name == "src" && e.is_dir));
        assert!(entries.iter().any(|e| e.name == "top.rs" && !e.is_dir));

        let files: Vec<_> = exec
            .walk(dir.path())
            .await
            .unwrap()
            .into_iter()
            .map(|w| w.path)
            .collect();
        assert!(files
            .iter()
            .any(|file| file.ends_with(Path::new("src").join("main.rs"))));
        assert!(files.iter().any(|file| file.ends_with("top.rs")));
        assert!(!files
            .iter()
            .any(|file| file.to_string_lossy().contains("node_modules")));
        assert!(!files
            .iter()
            .any(|file| file.to_string_lossy().contains("vendor")));
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
