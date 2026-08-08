use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use thiserror::Error;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

const SSH_TIMEOUT: &str = "10";
const MASTER_START_TIMEOUT: Duration = Duration::from_secs(12);
const MASTER_STOP_TIMEOUT: Duration = Duration::from_secs(3);
const MASTER_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// One lifecycle-owned SSH connection for one remote worker boundary.
///
/// Bootstrap commands, uploads, and the worker channel all multiplex over the
/// same private control socket. The master never persists beyond this object.
pub(crate) struct SshTransport {
    host: String,
    socket: PathBuf,
    _directory: TempDir,
    master: Arc<Mutex<Option<Child>>>,
}

impl SshTransport {
    pub(crate) async fn connect(host: &str) -> Result<Arc<Self>, SshTransportError> {
        let directory = private_socket_directory()?;
        let socket = directory.path().join("master");
        let mut command = Command::new("ssh");
        command
            .args(["-M", "-N"])
            .args(master_options(&socket))
            .arg(host)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let master = command.spawn()?;
        let transport = Arc::new(Self {
            host: host.into(),
            socket,
            _directory: directory,
            master: Arc::new(Mutex::new(Some(master))),
        });
        if let Err(error) = transport.wait_until_ready().await {
            transport.force_stop().await;
            return Err(error);
        }
        Ok(transport)
    }

    pub(crate) fn ssh_command(&self) -> Command {
        let mut command = Command::new("ssh");
        command.args(session_options(&self.socket)).arg(&self.host);
        command
    }

    pub(crate) fn worker_command(&self) -> Command {
        let mut command = Command::new("ssh");
        command
            .args(session_options(&self.socket))
            .arg("-T")
            .arg(&self.host);
        command
    }

    pub(crate) fn scp_command(&self) -> Command {
        let mut command = Command::new("scp");
        command.args(session_options(&self.socket));
        command
    }

    pub(crate) fn destination(&self, remote: &str) -> String {
        format!("{}:{remote}", self.host)
    }

    pub(crate) async fn shutdown(&self) -> Result<(), SshTransportError> {
        let mut exit = control_command(&self.socket, &self.host, "exit");
        let _ = tokio::time::timeout(MASTER_STOP_TIMEOUT, exit.status()).await;

        let mut master = self.master.lock().await;
        let Some(child) = master.as_mut() else {
            return Ok(());
        };
        match tokio::time::timeout(MASTER_STOP_TIMEOUT, child.wait()).await {
            Ok(status) => {
                status?;
                master.take();
                Ok(())
            }
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                master.take();
                Err(SshTransportError::ShutdownTimeout)
            }
        }
    }

    async fn wait_until_ready(&self) -> Result<(), SshTransportError> {
        let deadline = tokio::time::Instant::now() + MASTER_START_TIMEOUT;
        loop {
            {
                let mut master = self.master.lock().await;
                if let Some(status) = master
                    .as_mut()
                    .expect("master exists while starting")
                    .try_wait()?
                {
                    master.take();
                    return Err(SshTransportError::StartupExit(status.code()));
                }
            }
            let mut check = control_command(&self.socket, &self.host, "check");
            if check
                .status()
                .await
                .map(|status| status.success())
                .unwrap_or(false)
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(SshTransportError::StartupTimeout);
            }
            tokio::time::sleep(MASTER_POLL_INTERVAL).await;
        }
    }

    async fn force_stop(&self) {
        let mut master = self.master.lock().await;
        if let Some(child) = master.as_mut() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        master.take();
    }
}

impl Drop for SshTransport {
    fn drop(&mut self) {
        if Arc::strong_count(&self.master) == 1 {
            if let Ok(mut master) = self.master.try_lock() {
                if let Some(child) = master.as_mut() {
                    let _ = child.start_kill();
                }
            }
        }
    }
}

fn private_socket_directory() -> Result<TempDir, std::io::Error> {
    #[cfg(unix)]
    let root = Path::new("/tmp");
    #[cfg(not(unix))]
    let root = std::env::temp_dir();
    let directory = tempfile::Builder::new()
        .prefix("agent-ssh-")
        .tempdir_in(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(directory)
}

fn master_options(socket: &Path) -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={SSH_TIMEOUT}"),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=12".into(),
        "-o".into(),
        "ControlMaster=yes".into(),
        "-o".into(),
        "ControlPersist=no".into(),
        "-o".into(),
        format!("ControlPath={}", socket.display()),
    ]
}

fn session_options(socket: &Path) -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={SSH_TIMEOUT}"),
        "-o".into(),
        "ControlMaster=no".into(),
        "-o".into(),
        format!("ControlPath={}", socket.display()),
    ]
}

fn control_command(socket: &Path, host: &str, operation: &str) -> Command {
    let mut command = Command::new("ssh");
    command
        .arg("-S")
        .arg(socket)
        .args(["-O", operation, "-o", "BatchMode=yes"])
        .arg(host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[derive(Debug, Error)]
pub(crate) enum SshTransportError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("SSH master exited during startup with status {0:?}")]
    StartupExit(Option<i32>),
    #[error("SSH master did not become ready within the connection deadline")]
    StartupTimeout,
    #[error("SSH master did not exit within the shutdown deadline")]
    ShutdownTimeout,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_control_socket_is_short_and_owner_only() {
        let directory = private_socket_directory().unwrap();
        let socket = directory.path().join("master");
        assert!(socket.as_os_str().len() < 100);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                directory.path().metadata().unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn sessions_require_the_owned_control_socket_without_creating_aliases() {
        let socket = Path::new("/tmp/agent-ssh-test/master");
        let options = session_options(socket);
        assert!(options.contains(&"ControlMaster=no".into()));
        assert!(options.contains(&"ControlPath=/tmp/agent-ssh-test/master".into()));
        assert!(!options
            .iter()
            .any(|option| option.starts_with("ControlPersist=")));
    }

    #[test]
    fn master_is_process_owned_and_never_persists() {
        let options = master_options(Path::new("/tmp/agent-ssh-test/master"));
        assert!(options.contains(&"ControlMaster=yes".into()));
        assert!(options.contains(&"ControlPersist=no".into()));
        assert!(options.contains(&"ServerAliveInterval=15".into()));
        assert!(options.contains(&"ServerAliveCountMax=12".into()));
    }
}
