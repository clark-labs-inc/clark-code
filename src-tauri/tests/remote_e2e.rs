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
use provider_local::{discover_agent_setups, Executor, MigrationSource, RemoteExecutor};
use tokio_util::sync::CancellationToken;

struct Env {
    host: String,
    root: String,
    /// Optional dev override; unset means "fetch the prebuilt from the CDN".
    binary: Option<PathBuf>,
}

fn env() -> Option<Env> {
    let host = std::env::var("CLARK_SSH_TEST_HOST").ok()?;
    let root = std::env::var("CLARK_SSH_TEST_ROOT").ok()?;
    let binary = std::env::var("CLARK_SSH_TEST_BIN").ok().map(PathBuf::from);
    Some(Env { host, root, binary })
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
        local_binary: env.binary.clone(), // None → CDN fetch
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

/// Exercise the stateful process protocol on the real SSH transport: output
/// resumes after a client reconnect, stdin remains writable, retained output is
/// bounded, cancellation kills descendants, and the server root is enforced.
#[tokio::test]
#[ignore = "needs a live SSH host; set CLARK_SSH_TEST_{HOST,ROOT,BIN}"]
async fn remote_processes_survive_reconnect_and_remain_contained() {
    let Some(env) = env() else {
        eprintln!("skipping: set CLARK_SSH_TEST_HOST / _ROOT / _BIN to run this");
        return;
    };
    assert!(Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &env.host,
            &format!("mkdir -p {}", env.root),
        ])
        .status()
        .expect("spawn ssh mkdir")
        .success());

    let conn = ssh::connect(&RemoteSpec {
        host: env.host.clone(),
        remote_root: env.root.clone(),
        local_binary: env.binary.clone(),
    })
    .await
    .expect("ssh::connect");
    let first = RemoteExecutor::connect(&conn.ws_url, &conn.token)
        .await
        .expect("first RemoteExecutor");

    let process = first
        .background_start(
            "printf 'before\\n'; sleep 1; printf 'after\\n'; read value; printf 'input:%s\\n' \"$value\"",
            std::path::Path::new(&env.root),
        )
        .await
        .expect("start resumable process");
    let mut cursor = 0;
    let mut prefix = String::new();
    for _ in 0..50 {
        let status = first
            .background_status(&process, cursor)
            .await
            .expect("initial process status");
        cursor = status.cursor;
        for chunk in status.output {
            prefix.push_str(&String::from_utf8_lossy(&chunk.data));
        }
        if prefix.contains("before") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(prefix.contains("before"), "initial output: {prefix:?}");
    drop(first);

    let second = RemoteExecutor::connect(&conn.ws_url, &conn.token)
        .await
        .expect("reconnected RemoteExecutor");
    second
        .background_write(&process, b"hello-from-reconnect\n", true)
        .await
        .expect("write stdin after reconnect");
    let mut tail = String::new();
    let mut exit = None;
    for _ in 0..100 {
        let status = second
            .background_status(&process, cursor)
            .await
            .expect("resumed process status");
        cursor = status.cursor;
        for chunk in status.output {
            tail.push_str(&String::from_utf8_lossy(&chunk.data));
        }
        if status.exit_code.is_some() {
            exit = status.exit_code;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(exit, Some(Some(0)));
    assert!(tail.contains("after"), "resumed output: {tail:?}");
    assert!(
        tail.contains("input:hello-from-reconnect"),
        "resumed output: {tail:?}"
    );
    assert!(!tail.contains("before"), "output was replayed: {tail:?}");

    let noisy = second
        .background_start("yes x | head -c 1100000", std::path::Path::new(&env.root))
        .await
        .expect("start bounded-output process");
    let mut bounded = None;
    for _ in 0..100 {
        let status = second
            .background_status(&noisy, 0)
            .await
            .expect("bounded output status");
        if status.exit_code.is_some() {
            bounded = Some(status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let bounded = bounded.expect("noisy process did not finish");
    let retained = bounded
        .output
        .iter()
        .map(|chunk| chunk.data.len())
        .sum::<usize>();
    assert!(bounded.truncated);
    assert!(retained <= 1_048_576, "retained {retained} bytes");

    let child_pid = format!("{}/remote-child.pid", env.root);
    let tree = second
        .background_start(
            &format!("sleep 30 & echo $! > {child_pid}; wait"),
            std::path::Path::new(&env.root),
        )
        .await
        .expect("start process tree");
    for _ in 0..50 {
        if second
            .metadata(std::path::Path::new(&child_pid))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    second
        .background_kill(&tree)
        .await
        .expect("kill process tree");
    let killed = second
        .exec(
            &format!("pid=$(cat {child_pid}); ! kill -0 \"$pid\" 2>/dev/null"),
            std::path::Path::new(&env.root),
            Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .await
        .expect("check descendant");
    assert_eq!(killed.code, Some(0), "background descendant survived kill");

    let escape = std::path::Path::new(&env.root).join("../clark-escape.txt");
    let err = second.write(&escape, b"must not escape").await.unwrap_err();
    assert!(err.contains("escapes project root"), "{err}");

    let _ = second.remove_file(std::path::Path::new(&child_pid)).await;
    drop(second);
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote process/reconnect e2e: OK");
}

/// Agent migration discovery must read the *remote* setup over the tunnel.
#[tokio::test]
#[ignore = "needs a live SSH host; set CLARK_SSH_TEST_{HOST,ROOT}"]
async fn discovers_agent_config_on_the_remote() {
    let Some(env) = env() else {
        eprintln!("skipping: set CLARK_SSH_TEST_HOST / _ROOT");
        return;
    };

    // Seed a Claude setup on the remote: an .mcp.json + a project skill.
    let seed = format!(
        "mkdir -p {root}/.claude/skills/remote-skill && \
         printf '%s' '{{\"mcpServers\":{{\"remote-mcp\":{{\"command\":\"echo\",\"args\":[\"hi\"]}}}}}}' > {root}/.mcp.json && \
         printf '%s\\n' '---' 'name: remote-skill' 'description: A remote test skill.' '---' 'body' \
           > {root}/.claude/skills/remote-skill/SKILL.md",
        root = env.root
    );
    let ok = Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", &env.host, &seed])
        .status()
        .expect("seed")
        .success();
    assert!(ok, "failed to seed remote .claude");

    let conn = ssh::connect(&RemoteSpec {
        host: env.host.clone(),
        remote_root: env.root.clone(),
        local_binary: env.binary.clone(),
    })
    .await
    .expect("ssh::connect");
    let remote = RemoteExecutor::connect(&conn.ws_url, &conn.token)
        .await
        .expect("RemoteExecutor");

    let root = std::path::Path::new(&env.root);
    let discoveries = discover_agent_setups(&remote, root).await;
    let claude = discoveries
        .iter()
        .find(|discovery| discovery.source == MigrationSource::Claude)
        .expect("remote Claude setup");
    eprintln!(
        "remote mcp: {:?}",
        claude.mcp.iter().map(|m| &m.name).collect::<Vec<_>>()
    );
    assert!(claude
        .mcp
        .iter()
        .any(|m| m.name == "remote-mcp" && m.command == "echo"));

    eprintln!(
        "remote skills: {:?}",
        claude.skills.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    let s = claude
        .skills
        .iter()
        .find(|s| s.name == "remote-skill")
        .expect("remote skill");
    assert_eq!(s.description, "A remote test skill.");
    assert_eq!(s.scope, "project");

    let _ = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &env.host,
            &format!("rm -rf {}/.claude {}/.mcp.json", env.root, env.root),
        ])
        .status();
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote claude discovery: OK");
}
