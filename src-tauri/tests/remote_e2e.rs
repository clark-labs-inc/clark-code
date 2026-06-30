//! Live end-to-end test for **remote projects** — Phase 3 verification.
//!
//! It drives the *real* orchestration (`ssh.rs`) and the *real* client
//! (`RemoteExecutor`) against an actual host, proving the agent's tool I/O lands
//! on the remote machine. It needs a reachable SSH host + a `clark-exec-server`
//! built for that host's architecture, so it's `#[ignore]`d by default and only
//! runs when explicitly opted in:
//!
//! ```sh
//! CLARK_SSH_TEST_HOST=gpu \
//! CLARK_SSH_TEST_ROOT=/home/me/clark-remote-test \
//! CLARK_SSH_TEST_BIN=target/x86_64-unknown-linux-musl/release/clark-exec-server \
//! cargo test -p clark-desktop --test remote_e2e -- --ignored --nocapture
//! ```
//!
//! With the env unset it prints a skip note and passes, so it's inert in CI.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use clark_desktop_lib::ssh::{self, RemoteSpec};
use provider_local::{Executor, RemoteExecutor};
use tokio_util::sync::CancellationToken;

struct Env {
    host: String,
    root: String,
    binary: PathBuf,
}

fn env() -> Option<Env> {
    let host = std::env::var("CLARK_SSH_TEST_HOST").ok()?;
    let root = std::env::var("CLARK_SSH_TEST_ROOT").ok()?;
    let binary = std::env::var("CLARK_SSH_TEST_BIN").ok()?;
    Some(Env {
        host,
        root,
        binary: PathBuf::from(binary),
    })
}

#[tokio::test]
#[ignore = "needs a live SSH host; set CLARK_SSH_TEST_{HOST,ROOT,BIN}"]
async fn remote_project_round_trips_against_a_live_host() {
    let Some(env) = env() else {
        eprintln!("skipping: set CLARK_SSH_TEST_HOST / _ROOT / _BIN to run this");
        return;
    };

    // Ensure the project dir exists on the remote (test setup, not under test).
    let mkdir = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &env.host,
            &format!("mkdir -p {}", env.root),
        ])
        .status()
        .expect("spawn ssh mkdir");
    assert!(
        mkdir.success(),
        "could not create {} on {}",
        env.root,
        env.host
    );

    // 1. Bring up the server + tunnel via the real orchestrator.
    let spec = RemoteSpec {
        host: env.host.clone(),
        remote_root: env.root.clone(),
        local_binary: env.binary.clone(),
    };
    let conn = ssh::connect(&spec).await.expect("ssh::connect failed");
    eprintln!("connected: {} (arch {})", conn.ws_url, conn.arch.slug());

    // 2. Connect the same RemoteExecutor the provider would.
    let remote = RemoteExecutor::connect(&conn.ws_url, &conn.token)
        .await
        .expect("RemoteExecutor::connect failed");
    let cancel = CancellationToken::new();

    // 3. Write a file on the remote and read it back.
    let marker = format!("{}/clark-e2e-marker.txt", env.root);
    let payload = b"hello from clark-desktop e2e";
    remote
        .write(std::path::Path::new(&marker), payload)
        .await
        .expect("remote write");
    let read_back = remote
        .read(std::path::Path::new(&marker))
        .await
        .expect("remote read");
    assert_eq!(
        read_back, payload,
        "file content round-trips through the remote"
    );

    // 4. Run a command and prove it executed on the remote (hostname + the file
    //    we just wrote is visible to the remote shell).
    let out = remote
        .exec(
            &format!("hostname; pwd; ls {}", env.root),
            std::path::Path::new(&env.root),
            Duration::from_secs(30),
            &cancel,
        )
        .await
        .expect("remote exec");
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("remote exec stdout:\n{stdout}");
    assert_eq!(out.code, Some(0), "exec exited cleanly");
    assert!(
        stdout.contains("clark-e2e-marker.txt"),
        "the remote shell sees the file we wrote: {stdout:?}"
    );

    // 5. Walk finds the marker too.
    let walked = remote
        .walk(std::path::Path::new(&env.root))
        .await
        .expect("remote walk");
    assert!(
        walked
            .iter()
            .any(|w| w.path.ends_with("clark-e2e-marker.txt")),
        "walk surfaces the written file"
    );

    // 6. Clean up the marker, then drop the connection (kills server + tunnel).
    let _ = remote
        .exec(
            &format!("rm -f {}", marker),
            std::path::Path::new(&env.root),
            Duration::from_secs(10),
            &cancel,
        )
        .await;
    drop(remote);
    drop(conn);
    // Give the kill-on-drop a beat to propagate before the test process exits.
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote e2e: OK");
}
