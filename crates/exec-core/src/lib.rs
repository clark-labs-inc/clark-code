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
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn shell: {e}"))?;

        let wait = child.wait_with_output();
        let output = tokio::select! {
            _ = cancel.cancelled() => return Err("command cancelled".into()),
            res = tokio::time::timeout(timeout, wait) => res,
        };
        match output {
            Ok(Ok(out)) => Ok(ExecOutput {
                stdout: out.stdout,
                stderr: out.stderr,
                code: out.status.code(),
            }),
            Ok(Err(e)) => Err(format!("command failed: {e}")),
            Err(_) => Err(format!(
                "command timed out after {} ms",
                timeout.as_millis()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
