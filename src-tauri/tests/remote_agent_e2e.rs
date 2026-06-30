//! Full-stack live test: the **real agent loop + real Clark model** driving the
//! coding tools on a **remote host** over SSH. This is the end-to-end path a user
//! takes — `ssh.rs` brings up the exec-server (fetched from the CDN), the
//! `LocalAgentProvider` connects its `RemoteExecutor` through the tunnel, and the
//! model is asked to write a file + run a command **on the remote**.
//!
//! Needs a live host + a real `ck_live_` key, so it's `#[ignore]`d. Run with:
//!
//! ```sh
//! CLARK_SSH_TEST_HOST=scl \
//! CLARK_SSH_TEST_ROOT=/home/ubuntu/clark-remote-test \
//! CLARK_API_KEY=ck_live_… \
//! cargo test -p clark-desktop --test remote_agent_e2e -- --ignored --nocapture
//! ```

use std::process::Command;
use std::time::Duration;

use agent_core::domain::AgentEvent;
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use clark_desktop_lib::ssh::{self, RemoteSpec};
use futures::StreamExt;
use serde_json::json;

const MARKER: &str = "agent_remote_proof.txt";
const CONTENT: &str = "clark-code-remote-ok";

#[tokio::test]
#[ignore = "needs a live SSH host + a real ck_live_ key; see file header"]
async fn agent_writes_a_file_and_runs_a_command_on_the_remote() {
    let (Ok(host), Ok(root), Ok(key)) = (
        std::env::var("CLARK_SSH_TEST_HOST"),
        std::env::var("CLARK_SSH_TEST_ROOT"),
        std::env::var("CLARK_API_KEY"),
    ) else {
        eprintln!("skipping: set CLARK_SSH_TEST_HOST / _ROOT and CLARK_API_KEY");
        return;
    };

    // Clean slate on the remote.
    let _ = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &host,
            &format!("mkdir -p {root}; rm -f {root}/{MARKER}"),
        ])
        .status();

    // 1) Bring up the remote server + tunnel (binary fetched from the CDN).
    let conn = ssh::connect(&RemoteSpec {
        host: host.clone(),
        remote_root: root.clone(),
        local_binary: None,
    })
    .await
    .expect("ssh::connect");
    eprintln!("remote up: {} ({})", conn.ws_url, conn.arch.slug());

    // 2) Connect the real provider, pointed at the remote via extra.remote.
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(key),
            extra: json!({
                "research": false,
                // Auto-allow the mutating tools so the loop runs unattended.
                "permissions": { "write_file": "allow", "edit_file": "allow", "bash": "allow" },
                "remote": { "ws_url": conn.ws_url, "token": conn.token, "cwd": root },
            }),
            ..Default::default()
        })
        .await
        .expect("provider.connect");

    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("new_session");

    // 3) Ask the model to do real work on the remote.
    let prompt = format!(
        "You are working in a remote project. Do exactly these two things, then stop:\n\
         1. Create a file named `{MARKER}` in the project root whose entire contents are this one line: {CONTENT}\n\
         2. Run the shell command `hostname` and tell me its output.\n\
         Use your tools. Do not ask for confirmation."
    );
    let mut stream = provider
        .prompt(&session.id, PromptInput::text(prompt))
        .await
        .expect("prompt");

    // 4) Drive the loop to completion (auto-approve any gate, just in case).
    let mut text = String::new();
    let mut tool_titles = Vec::new();
    let finished = tokio::time::timeout(Duration::from_secs(180), async {
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::ToolCall { call, .. } => tool_titles.push(call.title),
                AgentEvent::MessageChunk {
                    delta: agent_core::domain::ContentBlock::Text { text: t },
                    ..
                } => text.push_str(&t),
                AgentEvent::PermissionRequest { request } => {
                    let _ = provider
                        .respond(
                            &session.id,
                            ClientResponse::Permission {
                                request: request.id.clone(),
                                option: "allow_once".into(),
                            },
                        )
                        .await;
                }
                AgentEvent::RunFinished { .. } => return true,
                _ => {}
            }
        }
        false
    })
    .await
    .expect("agent run timed out");

    eprintln!("tools: {tool_titles:?}");
    eprintln!("final text: {text}");
    assert!(finished, "the run did not finish cleanly");

    // 5) The real proof: the file the *model* asked to create exists **on the
    //    remote**, with the content it was told to write.
    let out = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &host,
            &format!("cat {root}/{MARKER}"),
        ])
        .output()
        .expect("ssh cat");
    let on_remote = String::from_utf8_lossy(&out.stdout);
    eprintln!("remote file contents: {on_remote:?}");
    assert!(
        on_remote.contains(CONTENT),
        "the agent's file did not land on the remote with the expected content; got {on_remote:?}"
    );

    // Cleanup.
    let _ = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &host,
            &format!("rm -f {root}/{MARKER}"),
        ])
        .status();
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote agent e2e: OK");
}
