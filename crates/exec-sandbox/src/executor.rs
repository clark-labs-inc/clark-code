use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use exec_core::{
    run_process_streaming, run_process_streaming_pty, DirEntry, ExecOutput, ExecResult,
    ExecutionContainment, Executor, FileMeta, LocalExecutor, OnOutput, ProcessSpec, WalkEntry,
};
use tokio_util::sync::CancellationToken;

use crate::{SandboxManager, SandboxPolicy, SandboxStatus};

/// Session-scoped local executor. It applies the same resolved policy to direct
/// filesystem primitives and every process prepared through the Executor seam.
pub struct SandboxedExecutor {
    manager: SandboxManager,
    local: LocalExecutor,
}

impl SandboxedExecutor {
    pub fn new(policy: SandboxPolicy) -> Result<Self, String> {
        let manager = SandboxManager::current(policy)?;
        if !matches!(manager.status(), SandboxStatus::Enforced { .. }) {
            return Err(match manager.status() {
                SandboxStatus::Unavailable { reason, .. } => {
                    format!("local sandbox unavailable: {reason}")
                }
                SandboxStatus::SetupRequired { reason, .. } => {
                    format!("local sandbox setup required: {reason}")
                }
                SandboxStatus::Enforced { .. } => unreachable!(),
            });
        }
        Ok(Self {
            manager,
            local: LocalExecutor,
        })
    }

    pub fn with_manager(manager: SandboxManager) -> Result<Self, String> {
        if !matches!(manager.status(), SandboxStatus::Enforced { .. }) {
            return Err("sandbox manager is not ready".to_string());
        }
        Ok(Self {
            manager,
            local: LocalExecutor,
        })
    }

    pub fn manager(&self) -> &SandboxManager {
        &self.manager
    }

    fn read_path(&self, path: &Path) -> ExecResult<PathBuf> {
        self.manager.policy().check_read(path)
    }

    fn write_path(&self, path: &Path) -> ExecResult<PathBuf> {
        self.manager.policy().check_write(path)
    }
}

#[async_trait]
impl Executor for SandboxedExecutor {
    async fn read(&self, path: &Path) -> ExecResult<Vec<u8>> {
        self.local.read(&self.read_path(path)?).await
    }

    async fn write(&self, path: &Path, data: &[u8]) -> ExecResult<()> {
        self.local.write(&self.write_path(path)?, data).await
    }

    async fn create_dir_all(&self, path: &Path) -> ExecResult<()> {
        self.local.create_dir_all(&self.write_path(path)?).await
    }

    async fn remove_file(&self, path: &Path) -> ExecResult<()> {
        self.local.remove_file(&self.write_path(path)?).await
    }

    async fn remove_dir_all(&self, path: &Path) -> ExecResult<()> {
        self.local.remove_dir_all(&self.write_path(path)?).await
    }

    async fn rename(&self, from: &Path, to: &Path) -> ExecResult<()> {
        self.local
            .rename(&self.write_path(from)?, &self.write_path(to)?)
            .await
    }

    async fn read_dir(&self, path: &Path) -> ExecResult<Vec<DirEntry>> {
        self.local.read_dir(&self.read_path(path)?).await
    }

    async fn metadata(&self, path: &Path) -> ExecResult<FileMeta> {
        self.local.metadata(&self.read_path(path)?).await
    }

    async fn canonicalize(&self, path: &Path) -> ExecResult<PathBuf> {
        let requested = self.read_path(path)?;
        let canonical = self.local.canonicalize(&requested).await?;
        self.manager.policy().check_read(&canonical)
    }

    async fn home_dir(&self, cwd: &Path) -> ExecResult<PathBuf> {
        self.local.home_dir(cwd).await
    }

    async fn mtime(&self, path: &Path) -> Option<SystemTime> {
        self.metadata(path)
            .await
            .ok()
            .and_then(|metadata| metadata.modified)
    }

    async fn walk(&self, root: &Path) -> ExecResult<Vec<WalkEntry>> {
        self.local.walk(&self.read_path(root)?).await
    }

    fn prepare_process(&self, process: ProcessSpec) -> ExecResult<ProcessSpec> {
        self.manager.prepare_process(process)
    }

    fn containment(&self) -> ExecutionContainment {
        ExecutionContainment::Managed
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
        let process = self.prepare_process(ProcessSpec::shell(command, cwd))?;
        run_process_streaming(&process, timeout, cancel, on_output).await
    }

    async fn exec_streaming_pty(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
        on_output: OnOutput<'_>,
    ) -> ExecResult<ExecOutput> {
        let process = self.prepare_process(ProcessSpec::shell(command, cwd))?;
        run_process_streaming_pty(&process, timeout, cancel, on_output).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BackendKind;

    #[tokio::test]
    async fn direct_filesystem_operations_use_the_policy() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let policy = SandboxPolicy::workspace_write(workspace.path().to_path_buf(), Vec::new());
        let manager = SandboxManager::simulate(
            policy,
            BackendKind::MacosSeatbelt,
            PathBuf::from("/usr/bin/sandbox-exec"),
        );
        let executor = SandboxedExecutor::with_manager(manager).unwrap();
        executor
            .write(&workspace.path().join("inside.txt"), b"ok")
            .await
            .unwrap();
        assert!(executor
            .write(&outside.path().join("outside.txt"), b"bad")
            .await
            .is_err());
    }
}
