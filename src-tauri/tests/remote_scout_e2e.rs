//! End-to-end proof that **Scout + read-only delegation work on a remote SSH
//! target** — the feature just enabled. Orchestration used to be force-disabled
//! for remote sessions; now the scout toolchain registers, and a delegated
//! read-only child reconnects through the same exec-server instead of reading
//! the local disk.
//!
//! This is the full user path: `ssh.rs` brings up the remote exec-server, the
//! `LocalAgentProvider` connects its `RemoteExecutor` through the tunnel, and a
//! real model is asked to run `scout_capabilities` and to read a remote-only
//! file through `delegate_read_only`. It needs a live host + a real `ck_live_`
//! key, so it's `#[ignore]`d. Run with:
//!
//! ```sh
//! CLARK_SSH_TEST_HOST=cpu \
//! CLARK_SSH_TEST_ROOT=/home/ubuntu/clark-scout-e2e \
//! CLARK_API_KEY=ck_live_… \
//! CLARK_CODE_BASE_URL=https://api.clarkslabs.com/v1 \
//! CLARK_CODE_MODEL=clark-code \
//! CLARK_REMOTE_LIVE=1 \
//! cargo test -p clark-desktop --test remote_scout_e2e -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The remote exec-server binary is reused (it is protocol-identical to this
//! tree — no exec-server/exec-protocol code changed); the test asserts its
//! version matches `v{CARGO_PKG_VERSION}` so a stale remote can't fake a pass.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use agent_core::domain::{AgentEvent, RunOutcome, RunStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use clark_desktop_lib::ssh::{self, RemoteSpec};
use futures::StreamExt;
use provider_local::{Executor, LocalExecutor, RemoteExecutor};
use serde_json::json;

/// The scout + read-only-delegation tools that must register on a remote target.
/// (`delegate_coding_workstreams` intentionally stays local-only for now — see
/// orchestration_tool.rs.)
const SCOUT_TOOLS: &[&str] = &[
    "scout_capabilities",
    "scout_adapter",
    "scout_ledger",
    "scout_enterprise",
    "scout_enterprise_query",
    "scout_probe",
    "scout_measure",
    "delegate_read_only",
    "resolve_delegation",
];

struct LiveEnv {
    host: String,
    root: String,
    key: String,
    base_url: String,
    model: String,
    binary: Option<PathBuf>,
}

/// SSH-only (no paid model): the transport + session-setup half of the e2e.
struct SshEnv {
    host: String,
    root: String,
    binary: Option<PathBuf>,
}

fn ssh_env() -> Option<SshEnv> {
    let host = std::env::var("CLARK_SSH_TEST_HOST")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let root = std::env::var("CLARK_SSH_TEST_ROOT")
        .ok()
        .filter(|v| !v.trim().is_empty());
    match (host, root) {
        (Some(host), Some(root)) => Some(SshEnv {
            host,
            root,
            binary: std::env::var("CLARK_SSH_TEST_BIN").ok().map(PathBuf::from),
        }),
        _ => {
            eprintln!(
                "skipping: set CLARK_SSH_TEST_HOST + CLARK_SSH_TEST_ROOT to a reachable host"
            );
            None
        }
    }
}

fn live_env() -> Option<LiveEnv> {
    if std::env::var("CLARK_REMOTE_LIVE").as_deref() != Ok("1") {
        eprintln!("skipping: set CLARK_REMOTE_LIVE=1 to authorize a paid live run");
        return None;
    }
    let required = |name: &str| -> String {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| panic!("{name} is required when CLARK_REMOTE_LIVE=1"))
    };
    Some(LiveEnv {
        host: required("CLARK_SSH_TEST_HOST"),
        root: required("CLARK_SSH_TEST_ROOT"),
        key: required("CLARK_API_KEY"),
        base_url: required("CLARK_CODE_BASE_URL"),
        model: required("CLARK_CODE_MODEL"),
        // Optional dev override; when unset the deployed matching-version binary
        // is reused (verified below).
        binary: std::env::var("CLARK_SSH_TEST_BIN").ok().map(PathBuf::from),
    })
}

fn ssh(host: &str, script: &str) -> std::process::Output {
    Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            host,
            script,
        ])
        .output()
        .expect("ssh")
}

fn assert_done(outcome: Option<RunOutcome>, tools: &[String], text: &str) {
    let outcome = outcome.expect("the run stream ended without a terminal outcome");
    eprintln!("usage: {:?}", outcome.usage);
    assert_eq!(
        outcome.status,
        RunStatus::Done,
        "tools: {tools:?}; text: {text}"
    );
}

async fn run_to_completion(
    provider: &mut provider_local::LocalAgentProvider,
    session_id: &agent_core::ids::SessionId,
    mut stream: agent_core::provider::EventStream,
    tools: &mut Vec<String>,
    text: &mut String,
) -> Option<RunOutcome> {
    tokio::time::timeout(Duration::from_secs(240), async {
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::ToolCall { call, .. } => {
                    tools.push(call.tool_name.clone().unwrap_or_else(|| call.title.clone()));
                }
                AgentEvent::MessageChunk {
                    delta: agent_core::domain::ContentBlock::Text { text: t },
                    ..
                } => text.push_str(&t),
                AgentEvent::PermissionRequest { request } => {
                    let _ = provider
                        .respond(
                            session_id,
                            ClientResponse::Permission {
                                request: request.id.clone(),
                                option: "allow_once".into(),
                                feedback: None,
                            },
                        )
                        .await;
                }
                AgentEvent::RunFinished { outcome, .. } => return Some(outcome),
                _ => {}
            }
        }
        None
    })
    .await
    .expect("agent run timed out")
}

#[tokio::test]
#[ignore = "needs a live SSH host + a real ck_live_ key; see file header"]
async fn scout_and_read_only_delegation_run_on_the_remote() {
    let Some(e) = live_env() else { return };

    // If no dev binary is given, the deployed server must match this tree.
    if e.binary.is_none() {
        let want = format!(
            "$HOME/.clark/bin/clark-exec-server-v{}-linux-x86_64",
            env!("CARGO_PKG_VERSION")
        );
        assert!(
            ssh(&e.host, &format!("test -x {want}")).status.success(),
            "remote exec-server v{} not deployed on {} — connect the app's remote project once to deploy it",
            env!("CARGO_PKG_VERSION"),
            e.host
        );
    }

    // Clean slate on the remote.
    let reset = format!(
        "mkdir -p {r} && rm -rf {r}/* {r}/.[!.]* 2>/dev/null; mkdir -p {r}",
        r = e.root
    );
    assert!(ssh(&e.host, &reset).status.success());

    // 1) Bring up the remote server + tunnel.
    let conn = ssh::connect(&RemoteSpec {
        host: e.host.clone(),
        remote_root: e.root.clone(),
        local_binary: e.binary.clone(),
    })
    .await
    .expect("ssh::connect");
    eprintln!("remote up: {} ({})", conn.ws_url, conn.arch.slug());

    // A direct client to the same server: seed the remote-only file the child
    // must read, and confirm it does NOT exist locally.
    let seed = RemoteExecutor::connect(&conn.ws_url, &conn.token)
        .await
        .expect("seed RemoteExecutor");

    // Two independent, sizeable areas so the delegation admission gate accepts
    // the fan-out (explore requires ≥2 non-overlapping workstreams and ≥40k
    // total estimated context tokens ≈ 160KB of source). Each area carries a
    // distinct marker that exists ONLY on the remote host.
    let filler = "filler line for context size\n".repeat(4_500); // ~126KB per area
    for (area, marker) in [
        ("area-a", "REMOTE_AREA_A_MARKER_1c2d3e"),
        ("area-b", "REMOTE_AREA_B_MARKER_4f5a6b"),
    ] {
        let blob = format!("{filler}{marker}\n");
        seed.write(
            std::path::Path::new(&format!("{}/{area}/blob.txt", e.root)),
            blob.as_bytes(),
        )
        .await
        .expect("seed area blob");
        assert!(
            !std::path::Path::new(&format!("{}/{area}/blob.txt", e.root)).exists(),
            "{area}/blob.txt exists locally — the remote-only proof would be invalid"
        );
    }

    // 2) Connect the real provider against the remote.
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(e.key.clone()),
            extra: json!({
                "model": e.model.clone(),
                "base_url": e.base_url.clone(),
                "research": false,
                "remote": { "ws_url": conn.ws_url, "token": conn.token, "cwd": e.root.clone() },
            }),
            ..Default::default()
        })
        .await
        .expect("provider.connect");
    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("new_session");

    // 3) Turn one: run scout_capabilities against the remote target.
    let mut tools = Vec::new();
    let mut text = String::new();
    let stream = provider
        .prompt(
            &session.id,
            PromptInput::text(
                "Call the `scout_capabilities` tool exactly once with no arguments, \
                 then in one short sentence report the target platform and architecture it returned. \
                 Do not call any other tool.",
            ),
        )
        .await
        .expect("prompt scout_capabilities");
    let outcome =
        run_to_completion(&mut provider, &session.id, stream, &mut tools, &mut text).await;
    eprintln!("turn 1 tools: {tools:?}\nturn 1 text: {text}");
    assert_done(outcome, &tools, &text);
    assert!(
        tools.iter().any(|t| t == "scout_capabilities"),
        "expected the model to call scout_capabilities on the remote: {tools:?}"
    );

    // 4) Turn two: fan out a read-only exploration over the two remote-only
    //    areas. The delegated children run as nested providers that must
    //    reconnect through the same exec-server; if they fell back to a local
    //    executor the area files would not exist for them and the markers would
    //    be absent from their reports.
    let mut tools2 = Vec::new();
    let mut text2 = String::new();
    let prompt2 = "Use the `delegate_read_only` tool with purpose `explore` and exactly two \
         workstreams: one with scope `area-a` whose objective is to report the exact \
         `REMOTE_AREA_A_MARKER_…` line at the end of `area-a/blob.txt`, and one with \
         scope `area-b` whose objective is to report the exact `REMOTE_AREA_B_MARKER_…` \
         line at the end of `area-b/blob.txt`. Then call `resolve_delegation` for each \
         reported task to accept sound evidence, and finish by telling me both markers."
        .to_string();
    let stream2 = provider
        .prompt(&session.id, PromptInput::text(prompt2))
        .await
        .expect("prompt delegate_read_only");
    let outcome2 =
        run_to_completion(&mut provider, &session.id, stream2, &mut tools2, &mut text2).await;
    eprintln!("turn 2 tools: {tools2:?}\nturn 2 text: {text2}");
    assert_done(outcome2, &tools2, &text2);
    assert!(
        tools2.iter().any(|t| t == "delegate_read_only"),
        "expected the model to call delegate_read_only on the remote: {tools2:?}"
    );
    // resolve_delegation only succeeds when a fan-out was admitted and reported —
    // this is the proof the delegated children actually ran (on the remote).
    assert!(
        tools2.iter().any(|t| t == "resolve_delegation"),
        "expected the fan-out to be admitted and resolved (not refused): {tools2:?}"
    );

    // 5) The decisive proof: both remote-only markers surfaced via the children.
    for marker in ["REMOTE_AREA_A_MARKER_1c2d3e", "REMOTE_AREA_B_MARKER_4f5a6b"] {
        assert!(
            text2.contains(marker),
            "delegated child missed the remote-only marker {marker}: {text2}"
        );
    }

    // Cleanup.
    let _ = ssh(&e.host, &format!("rm -rf {r}", r = e.root));
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote scout e2e: OK");
}

/// The free half of the e2e (no model, no cost): bring up the remote over real
/// SSH, connect a provider session through `extra.remote`, and confirm the scout
/// toolchain registers against the remote executor — and that its read path
/// resolves to the remote host (a file that exists only remotely is readable,
/// and is absent locally).
#[tokio::test]
#[ignore = "needs a live SSH host; set CLARK_SSH_TEST_HOST/ROOT — see header"]
async fn scout_toolchain_registers_and_reads_on_the_remote() {
    let Some(e) = ssh_env() else { return };

    if e.binary.is_none() {
        let want = format!(
            "$HOME/.clark/bin/clark-exec-server-v{}-linux-x86_64",
            env!("CARGO_PKG_VERSION")
        );
        assert!(
            ssh(&e.host, &format!("test -x {want}")).status.success(),
            "remote exec-server v{} not deployed on {}",
            env!("CARGO_PKG_VERSION"),
            e.host
        );
    }

    let reset = format!(
        "mkdir -p {r} && rm -rf {r}/* 2>/dev/null; mkdir -p {r}",
        r = e.root
    );
    assert!(ssh(&e.host, &reset).status.success());

    let conn = ssh::connect(&RemoteSpec {
        host: e.host.clone(),
        remote_root: e.root.clone(),
        local_binary: e.binary.clone(),
    })
    .await
    .expect("ssh::connect");
    eprintln!("remote up: {} ({})", conn.ws_url, conn.arch.slug());

    // A file that exists only on the remote host.
    let only_remote = format!("{}/only-remote.txt", e.root);
    let direct = RemoteExecutor::connect(&conn.ws_url, &conn.token)
        .await
        .expect("direct RemoteExecutor");
    direct
        .write(std::path::Path::new(&only_remote), b"remote-bytes\n")
        .await
        .expect("seed only-remote.txt");
    assert!(
        LocalExecutor
            .metadata(std::path::Path::new(&only_remote))
            .await
            .is_err(),
        "only-remote.txt exists locally — remote-only proof invalid"
    );

    // Connect a real provider session through extra.remote. No prompt is sent, so
    // no model call happens — this only exercises session setup on the remote.
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            extra: json!({
                "research": false,
                "memories": false,
                "remote": { "ws_url": conn.ws_url, "token": conn.token, "cwd": e.root.clone() },
            }),
            ..Default::default()
        })
        .await
        .expect("provider.connect");
    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("new_session");

    // The scout toolchain + read-only delegation must be registered for a remote
    // session (they were previously gated off entirely).
    let names = provider.tool_names();
    eprintln!("registered remote tools: {names:?}");
    for tool in SCOUT_TOOLS {
        assert!(
            names.iter().any(|n| n == tool),
            "missing `{tool}` on remote session"
        );
    }
    // The coding writer stays local-only until it propagates `remote` itself.
    assert!(
        !names.iter().any(|n| n == "delegate_coding_workstreams"),
        "delegate_coding_workstreams must stay local-only on a remote session"
    );

    // Read the remote-only file through the session's executor path to prove the
    // session's I/O is bound to the remote host, not the local disk.
    let bytes = provider
        .session_executor()
        .read(std::path::Path::new(&only_remote))
        .await
        .expect("read only-remote.txt through the session executor");
    assert_eq!(bytes, b"remote-bytes\n");

    let _ = session; // session id unused beyond proving setup succeeded
    let _ = ssh(&e.host, &format!("rm -rf {r}", r = e.root));
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote scout registration e2e: OK");
}

/// The **security** specialist on a remote host, with no model: connect a real
/// remote session, then drive `security_scan_contract` (inventory) and
/// `security_poc_execute` directly. The PoC must run through the target-native
/// `security-poc-v1` service on the remote host and return a sealed
/// `managed_disposable` receipt — previously this tool refused remote outright.
#[tokio::test]
#[ignore = "needs a live SSH host with the new exec-server; set CLARK_SSH_TEST_HOST/ROOT"]
async fn security_scan_and_poc_run_on_the_remote() {
    let Some(e) = ssh_env() else { return };

    let reset = format!(
        "mkdir -p {r} && rm -rf {r}/* 2>/dev/null; mkdir -p {r}",
        r = e.root
    );
    assert!(ssh(&e.host, &reset).status.success());

    let conn = ssh::connect(&RemoteSpec {
        host: e.host.clone(),
        remote_root: e.root.clone(),
        local_binary: e.binary.clone(),
    })
    .await
    .expect("ssh::connect");
    eprintln!("remote up: {} ({})", conn.ws_url, conn.arch.slug());

    // Seed a remotely-only source file the PoC will scan.
    let direct = RemoteExecutor::connect(&conn.ws_url, &conn.token)
        .await
        .expect("direct RemoteExecutor");
    direct
        .write(
            std::path::Path::new(&format!("{}/app.py", e.root)),
            b"# SECURITY_MARKER token = 'hardcoded-secret'\n",
        )
        .await
        .expect("seed app.py");

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            extra: json!({
                "research": false,
                "memories": false,
                "remote": { "ws_url": conn.ws_url, "token": conn.token, "cwd": e.root.clone() },
            }),
            ..Default::default()
        })
        .await
        .expect("provider.connect");
    let _session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("new_session");

    // Both security tools must be exposed for a remote session now.
    let names = provider.tool_names();
    assert!(names.iter().any(|n| n == "security_scan_contract"));
    assert!(names.iter().any(|n| n == "security_poc_execute"));

    let ctx = provider.tool_ctx().expect("tool ctx");

    // 1) Inventory the remote checkout.
    let inventory = provider
        .tool("security_scan_contract")
        .expect("security_scan_contract")
        .invoke(json!({ "action": "inventory", "scope": "." }), &ctx)
        .await;
    assert!(
        !inventory.is_error,
        "inventory failed: {}",
        inventory.content
    );
    let inventory_id = inventory.details["inventoryId"]
        .as_str()
        .expect("inventoryId")
        .to_string();
    eprintln!("remote inventory id: {inventory_id}");

    // 2) Run a PoC control against the remote checkout via the target service.
    let poc = provider
        .tool("security_poc_execute")
        .expect("security_poc_execute")
        .invoke(
            json!({
                "scan_id": "scan-remote-e2e",
                "candidate_id": "cand-remote-e2e",
                "inventory_id": inventory_id,
                "scope": ".",
                "control": "positive",
                "language": "shell",
                "expected_observation": "the hardcoded marker is present in app.py",
                "script": "grep -q SECURITY_MARKER app.py",
                "expected_exit_code": 0,
                "timeout_seconds": 15
            }),
            &ctx,
        )
        .await;
    assert!(!poc.is_error, "remote PoC failed: {}", poc.content);
    let receipt = &poc.details["receipt"];
    assert_eq!(receipt["containment"], json!("managed_disposable"));
    assert_eq!(receipt["passed"], json!(true));
    let artifact = receipt["artifactPath"].as_str().unwrap().to_string();
    eprintln!("remote PoC receipt: {receipt}");

    // 3) The receipt + workspace artifacts must exist ON THE REMOTE host.
    let listing = ssh(&e.host, &format!("cat {}/{artifact}", e.root));
    let body = String::from_utf8_lossy(&listing.stdout);
    assert!(
        body.contains("managed_disposable"),
        "remote receipt.json missing managed_disposable: {body}"
    );

    let _ = ssh(&e.host, &format!("rm -rf {r}", r = e.root));
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote security e2e: OK");
}
