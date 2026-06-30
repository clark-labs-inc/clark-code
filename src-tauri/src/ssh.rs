//! SSH orchestration for **remote projects**.
//!
//! Given an SSH destination + a remote project root, this brings up the
//! `clark-exec-server` on the remote and a loopback tunnel to it, then hands the
//! provider a `ws://127.0.0.1:<port>` URL + capability token. The provider's
//! `RemoteExecutor` connects to that URL, so the agent's file/shell tools run on
//! the remote while the loop + model stay local.
//!
//! Design (mirrors codex's exec-server transport):
//!   1. `ssh <host> uname -sm` — detect the remote arch.
//!   2. Ensure `~/.clark/bin/clark-exec-server-v<ver>-<arch>` exists on the
//!      remote; if not, scp the matching local build up (Phase 5 will swap this
//!      for a version-pinned CDN fetch — see [`ensure_binary`]).
//!   3. Run the server in the **foreground** of one ssh channel bound to
//!      `127.0.0.1:0`; read its printed URL to learn the ephemeral remote port.
//!      The capability token is delivered over that channel's **stdin**, so it
//!      never appears in any process's argv (local or remote).
//!   4. Open a second `ssh -N -L <localport>:127.0.0.1:<remoteport>` channel.
//!   5. Return `ws://127.0.0.1:<localport>` + token.
//!
//! Security: **system ssh only** — no stored secrets; auth/encryption is the SSH
//! tunnel, inheriting `~/.ssh/config`, the agent, `known_hosts`, and ProxyJump.
//! We deliberately do not weaken `StrictHostKeyChecking`. The exec-server binds
//! loopback on the remote and is reachable only through the forward; the token
//! is defense-in-depth against another local user on the remote.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

/// How long to wait on any single ssh control operation.
const SSH_CONNECT_TIMEOUT: &str = "10";

/// What the caller asks to connect to.
pub struct RemoteSpec {
    /// SSH destination passed verbatim to `ssh` — a `~/.ssh/config` alias or
    /// `user@host`. ProxyJump / port / identity come from the user's ssh config.
    pub host: String,
    /// Absolute project root **on the remote host**.
    pub remote_root: String,
    /// Local `clark-exec-server` build to upload, compiled for the remote arch.
    pub local_binary: PathBuf,
}

/// A live remote connection. Field order matters: `_server_stdin` is declared
/// first so it drops first — closing it sends EOF over the **still-open** ssh
/// channel, which makes the remote watchdog kill the server (see
/// [`start_server`]). This is why the server channel is *not* `kill_on_drop`: a
/// SIGKILLed ssh client doesn't cleanly close its channel without keepalives, so
/// the remote process would orphan. The same EOF fires if the whole app dies
/// (the OS closes the pipe), so the remote is reaped either way. The tunnel is a
/// plain forwarder with no remote process, so `kill_on_drop` is fine there.
pub struct RemoteConn {
    pub ws_url: String,
    pub token: String,
    pub remote_root: String,
    pub arch: RemoteArch,
    /// The server channel's stdin — the remote-shutdown signal. Drops first.
    _server_stdin: ChildStdin,
    _server: Child,
    _tunnel: Child,
}

/// Remote CPU/OS — selects the matching server binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteArch {
    LinuxX86_64,
    LinuxAarch64,
    DarwinArm64,
    DarwinX86_64,
}

impl RemoteArch {
    /// Parse `uname -sm` output (e.g. "Linux x86_64", "Darwin arm64").
    fn from_uname(out: &str) -> Result<Self, String> {
        let mut it = out.split_whitespace();
        let os = it.next().unwrap_or("");
        let machine = it.next().unwrap_or("");
        match (os, machine) {
            ("Linux", "x86_64") => Ok(Self::LinuxX86_64),
            ("Linux", "aarch64" | "arm64") => Ok(Self::LinuxAarch64),
            ("Darwin", "arm64") => Ok(Self::DarwinArm64),
            ("Darwin", "x86_64") => Ok(Self::DarwinX86_64),
            _ => Err(format!("unsupported remote platform: {:?}", out.trim())),
        }
    }

    /// Stable slug for the binary filename and (Phase 5) the CDN asset path.
    pub fn slug(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "linux-x86_64",
            Self::LinuxAarch64 => "linux-aarch64",
            Self::DarwinArm64 => "darwin-aarch64",
            Self::DarwinX86_64 => "darwin-x86_64",
        }
    }
}

/// Bring up the remote server + tunnel and return a ready connection.
pub async fn connect(spec: &RemoteSpec) -> Result<RemoteConn, String> {
    if !spec.local_binary.is_file() {
        return Err(format!(
            "local clark-exec-server binary not found at {} — build it for the remote's architecture first",
            spec.local_binary.display()
        ));
    }

    let arch = detect_arch(&spec.host).await?;
    let home = remote_home(&spec.host).await?;
    let remote_bin = format!(
        "{home}/.clark/bin/clark-exec-server-v{}-{}",
        env!("CARGO_PKG_VERSION"),
        arch.slug()
    );
    ensure_binary(&spec.host, &remote_bin, &spec.local_binary).await?;

    let token = new_token();
    let (server, server_stdin, remote_port) =
        start_server(&spec.host, &remote_bin, &spec.remote_root, &token).await?;
    let local_port = free_local_port()?;
    let tunnel = open_tunnel(&spec.host, local_port, remote_port).await?;
    wait_for_port(local_port, Duration::from_secs(10)).await?;

    Ok(RemoteConn {
        ws_url: format!("ws://127.0.0.1:{local_port}"),
        token,
        remote_root: spec.remote_root.clone(),
        arch,
        _server_stdin: server_stdin,
        _server: server,
        _tunnel: tunnel,
    })
}

/// Result of a read-only "test connection" against a host.
#[derive(serde::Serialize)]
pub struct Probe {
    /// Detected architecture slug (e.g. `linux-x86_64`).
    pub arch: String,
    /// The remote `$HOME`, so the UI can show where the server will live.
    pub home: String,
}

/// Reach a host and report its architecture + home — no deploy, no tunnel. Backs
/// the settings "Test connection" button; surfaces the exact failures that bite
/// at connect time (unreachable host, unsupported arch).
pub async fn probe(host: &str) -> Result<Probe, String> {
    let arch = detect_arch(host).await?;
    let home = remote_home(host).await?;
    Ok(Probe {
        arch: arch.slug().to_string(),
        home,
    })
}

async fn detect_arch(host: &str) -> Result<RemoteArch, String> {
    RemoteArch::from_uname(&ssh_capture(host, "uname -sm").await?)
}

async fn remote_home(host: &str) -> Result<String, String> {
    let home = ssh_capture(host, "printf %s \"$HOME\"").await?;
    let home = home.trim().to_string();
    if home.is_empty() {
        return Err("could not resolve remote $HOME".into());
    }
    Ok(home)
}

/// Upload the server binary if the remote doesn't already have this exact
/// version+arch. The version is in the filename, so a desktop upgrade naturally
/// re-uploads. (Phase 5: fetch the version-pinned prebuilt from the CDN instead
/// of uploading, falling back to upload for dev.)
async fn ensure_binary(host: &str, remote_bin: &str, local_binary: &Path) -> Result<(), String> {
    if ssh_ok(host, &format!("test -x {}", shq(remote_bin))).await {
        return Ok(());
    }
    let dir = remote_bin.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
    if !ssh_ok(host, &format!("mkdir -p {}", shq(dir))).await {
        return Err(format!("could not create {dir} on {host}"));
    }
    scp(local_binary, &format!("{host}:{remote_bin}")).await?;
    if !ssh_ok(host, &format!("chmod +x {}", shq(remote_bin))).await {
        return Err(format!("could not chmod the uploaded server on {host}"));
    }
    Ok(())
}

/// Run the server on an ssh channel and read its bound port. Two jobs are done
/// over the channel's **stdin**, so nothing sensitive lands in argv:
///   1. the first line carries the capability token;
///   2. the channel then blocks on `cat`, so stdin EOF (the channel closing on
///      our end) is the signal to `kill` the server — a no-PTY `ssh host cmd`
///      otherwise leaves the remote process orphaned when the client goes away.
///
/// Returns the child, its (kept-open) stdin, and the remote port. The caller
/// must hold the stdin for the connection's lifetime; dropping it shuts the
/// server down.
async fn start_server(
    host: &str,
    remote_bin: &str,
    root: &str,
    token: &str,
) -> Result<(Child, ChildStdin, u16), String> {
    let remote_cmd = format!(
        "read CLARK_EXEC_TOKEN; export CLARK_EXEC_TOKEN; \
         {} --root {} --listen 127.0.0.1:0 & SRV=$!; \
         cat >/dev/null; kill \"$SRV\" 2>/dev/null",
        shq(remote_bin),
        shq(root)
    );
    // Deliberately NOT kill_on_drop: shutdown is via stdin-EOF → the remote
    // watchdog (a SIGKILLed ssh client wouldn't cleanly close its channel). The
    // local ssh process exits on its own once the remote command finishes.
    let mut child = Command::new("ssh")
        .args(["-o", &connect_timeout(), host, &remote_cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawning ssh (server): {e}"))?;

    let mut stdin = child.stdin.take().ok_or("ssh produced no stdin")?;
    stdin
        .write_all(format!("{token}\n").as_bytes())
        .await
        .map_err(|e| format!("sending token: {e}"))?;
    let _ = stdin.flush().await;
    // Keep `stdin` open — the remote `cat` blocks on it; closing it kills the
    // server. (Returned to the caller, who holds it in RemoteConn.)

    let stdout = child.stdout.take().ok_or("ssh produced no stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let port = tokio::time::timeout(Duration::from_secs(15), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(p) = parse_server_port(&line) {
                return Some(p);
            }
        }
        None
    })
    .await
    .map_err(|_| "timed out waiting for the exec-server to announce its URL".to_string())?
    .ok_or_else(|| "exec-server exited before printing its URL".to_string())?;

    // Keep draining stdout so the channel never back-pressures the server.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

    Ok((child, stdin, port))
}

async fn open_tunnel(host: &str, local_port: u16, remote_port: u16) -> Result<Child, String> {
    let forward = format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}");
    Command::new("ssh")
        .args([
            "-N",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            &connect_timeout(),
            "-L",
            &forward,
            host,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawning ssh (tunnel): {e}"))
}

async fn wait_for_port(port: u16, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err("the SSH tunnel did not come up in time".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ---- low-level ssh/scp helpers ---------------------------------------------

fn connect_timeout() -> String {
    format!("ConnectTimeout={SSH_CONNECT_TIMEOUT}")
}

/// Run a remote command and return its stdout, erroring on non-zero exit.
async fn ssh_capture(host: &str, remote_cmd: &str) -> Result<String, String> {
    let out = Command::new("ssh")
        .args(["-o", &connect_timeout(), host, remote_cmd])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("spawning ssh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ssh {host} `{remote_cmd}` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// True iff a remote command exits 0 (used for existence/`test` checks).
async fn ssh_ok(host: &str, remote_cmd: &str) -> bool {
    Command::new("ssh")
        .args(["-o", &connect_timeout(), host, remote_cmd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn scp(local: &Path, remote_dest: &str) -> Result<(), String> {
    let status = Command::new("scp")
        .args(["-o", &connect_timeout()])
        .arg(local)
        .arg(remote_dest)
        .stdin(Stdio::null())
        .status()
        .await
        .map_err(|e| format!("spawning scp: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("scp to {remote_dest} failed"))
    }
}

/// Allocate a free local loopback port by binding `:0` and releasing it. Small
/// TOCTOU window before ssh grabs it; acceptable for a per-session forward.
fn free_local_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("allocating a local port: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    Ok(port)
}

/// Single-quote a string for safe embedding in a remote `/bin/sh` command.
fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A 244-bit random capability token (two v4 UUIDs).
fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Parse `CLARK_EXEC_SERVER_URL=ws://127.0.0.1:<port>` → `<port>`.
fn parse_server_port(line: &str) -> Option<u16> {
    let url = line.trim().strip_prefix("CLARK_EXEC_SERVER_URL=")?;
    url.rsplit(':').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uname_to_arch() {
        assert_eq!(
            RemoteArch::from_uname("Linux x86_64\n").unwrap(),
            RemoteArch::LinuxX86_64
        );
        assert_eq!(
            RemoteArch::from_uname("Linux aarch64").unwrap(),
            RemoteArch::LinuxAarch64
        );
        assert_eq!(
            RemoteArch::from_uname("Darwin arm64").unwrap(),
            RemoteArch::DarwinArm64
        );
        assert!(RemoteArch::from_uname("Plan9 mips").is_err());
    }

    #[test]
    fn arch_slugs_are_stable() {
        assert_eq!(RemoteArch::LinuxX86_64.slug(), "linux-x86_64");
        assert_eq!(RemoteArch::LinuxAarch64.slug(), "linux-aarch64");
        assert_eq!(RemoteArch::DarwinArm64.slug(), "darwin-aarch64");
    }

    #[test]
    fn parses_server_url_line() {
        assert_eq!(
            parse_server_port("CLARK_EXEC_SERVER_URL=ws://127.0.0.1:54321"),
            Some(54321)
        );
        assert_eq!(
            parse_server_port("CLARK_EXEC_SERVER_URL=ws://127.0.0.1:54321\n"),
            Some(54321)
        );
        assert_eq!(parse_server_port("some other log line"), None);
        assert_eq!(
            parse_server_port("CLARK_EXEC_SERVER_URL=ws://127.0.0.1:notaport"),
            None
        );
    }

    #[test]
    fn shell_quoting_escapes_single_quotes() {
        assert_eq!(shq("/home/me/proj"), "'/home/me/proj'");
        assert_eq!(shq("a'b"), "'a'\\''b'");
    }

    #[test]
    fn tokens_are_long_and_unique() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64); // two 32-hex-char simple UUIDs
    }
}
