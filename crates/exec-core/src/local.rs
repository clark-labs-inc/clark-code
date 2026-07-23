use super::*;

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

    async fn remove_file(&self, path: &Path) -> ExecResult<()> {
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn remove_dir_all(&self, path: &Path) -> ExecResult<()> {
        match tokio::fs::remove_dir_all(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn rename(&self, from: &Path, to: &Path) -> ExecResult<()> {
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        #[cfg(windows)]
        {
            atomic_rename(from, to)
        }
        #[cfg(not(windows))]
        {
            tokio::fs::rename(from, to).await.map_err(|e| e.to_string())
        }
    }

    async fn read_dir(&self, path: &Path) -> ExecResult<Vec<DirEntry>> {
        let mut rd = tokio::fs::read_dir(path).await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(entry) = rd.next_entry().await.map_err(|e| e.to_string())? {
            let file_type = entry.file_type().await.map_err(|e| e.to_string())?;
            out.push(DirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: file_type.is_dir(),
                is_symlink: file_type.is_symlink(),
            });
        }
        Ok(out)
    }

    async fn metadata(&self, path: &Path) -> ExecResult<FileMeta> {
        let m = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|e| e.to_string())?;
        Ok(FileMeta {
            modified: m.modified().ok(),
            len: m.len(),
            is_dir: m.is_dir(),
            is_symlink: m.file_type().is_symlink(),
        })
    }

    async fn canonicalize(&self, path: &Path) -> ExecResult<PathBuf> {
        tokio::fs::canonicalize(path)
            .await
            .map_err(|e| e.to_string())
    }

    async fn home_dir(&self, _cwd: &Path) -> ExecResult<PathBuf> {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| "target environment has no HOME directory".to_string())
    }

    async fn walk(&self, root: &Path) -> ExecResult<Vec<WalkEntry>> {
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            let mut builder = ignore::WalkBuilder::new(&root);
            builder
                .follow_links(false)
                .hidden(false)
                .require_git(false)
                .filter_entry(|entry| !is_ignored(entry.path()));
            for entry in builder.build() {
                let Ok(entry) = entry else { continue };
                if !entry.file_type().is_some_and(|kind| kind.is_file()) {
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
        run_process_streaming(
            &ProcessSpec::shell(command, cwd),
            timeout,
            cancel,
            on_output,
        )
        .await
    }

    async fn exec_streaming_pty(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
        on_output: OnOutput<'_>,
    ) -> ExecResult<ExecOutput> {
        run_process_streaming_pty(
            &ProcessSpec::shell(command, cwd),
            timeout,
            cancel,
            on_output,
        )
        .await
    }
}

#[cfg(windows)]
fn atomic_rename(from: &Path, to: &Path) -> ExecResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let from = from
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod shell_tests {
    use super::{windows_shell_args, Executor, LocalExecutor, ShellKind};

    #[test]
    fn cmd_script_disables_autorun_and_command_echo() {
        assert_eq!(
            windows_shell_args(ShellKind::Cmd, Some("where.exe ssh")),
            ["/D", "/Q", "/C", "where.exe ssh"]
        );
    }

    #[test]
    fn powershell_script_disables_profiles_and_interaction() {
        assert_eq!(
            windows_shell_args(ShellKind::PowerShell, Some("Get-ChildItem")),
            [
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-ChildItem"
            ]
        );
    }

    #[test]
    fn interactive_powershell_remains_profile_free_without_noninteractive_mode() {
        assert_eq!(
            windows_shell_args(ShellKind::PowerShell, None),
            ["-NoLogo", "-NoProfile"]
        );
    }

    #[tokio::test]
    async fn rename_atomically_replaces_an_existing_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let from = temp.path().join("next.json");
        let to = temp.path().join("current.json");
        tokio::fs::write(&from, b"next")
            .await
            .expect("write source");
        tokio::fs::write(&to, b"old").await.expect("write target");

        LocalExecutor.rename(&from, &to).await.expect("replace");

        assert_eq!(tokio::fs::read(&to).await.expect("read target"), b"next");
        assert!(!from.exists());
    }
}
