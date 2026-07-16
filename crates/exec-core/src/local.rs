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
        configure_noninteractive(&mut cmd);
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
        for (name, value) in NONINTERACTIVE_ENV {
            cmd.env(name, value);
        }

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
