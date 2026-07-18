//! Full-stack live test: the **real agent loop + real Clark model** driving the
//! coding tools on a **remote host** over SSH. This is the end-to-end path a user
//! takes — `ssh.rs` uploads the requested development exec-server, the
//! `LocalAgentProvider` connects its `RemoteExecutor` through the tunnel, and the
//! model is asked to write a file + run a command **on the remote**.
//!
//! Needs a live host + a real `ck_live_` key, so it's `#[ignore]`d. Run with:
//!
//! ```sh
//! CLARK_SSH_TEST_HOST=scl \
//! CLARK_SSH_TEST_ROOT=/home/ubuntu/clark-remote-test \
//! CLARK_API_KEY=ck_live_… \
//! CLARK_CODE_BASE_URL=https://api.clarkslabs.com/v1 \
//! CLARK_CODE_MODEL=clark-code \
//! CLARK_SSH_TEST_BIN=target/x86_64-unknown-linux-musl/release/clark-exec-server \
//! CLARK_REMOTE_LIVE=1 \
//! cargo test -p clark-desktop --test remote_agent_e2e -- --ignored --nocapture --test-threads=1
//! ```

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use agent_core::domain::{AgentEvent, RunOutcome, RunStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use clark_desktop_lib::ssh::{self, RemoteSpec};
use futures::StreamExt;
use serde_json::json;

const MARKER: &str = "agent_remote_proof.txt";
const CONTENT: &str = "clark-code-remote-ok";

struct LiveEnv {
    host: String,
    root: String,
    key: String,
    base_url: String,
    model: String,
    binary: PathBuf,
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
        binary: PathBuf::from(required("CLARK_SSH_TEST_BIN")),
    })
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

#[tokio::test]
#[ignore = "needs a live SSH host + a real ck_live_ key; see file header"]
async fn agent_writes_a_file_and_runs_a_command_on_the_remote() {
    let Some(env) = live_env() else { return };

    // Clean slate on the remote.
    let _ = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &env.host,
            &format!("mkdir -p {root}; rm -f {root}/{MARKER}", root = env.root),
        ])
        .status();

    // 1) Bring up the remote server + tunnel using this working tree's binary.
    let conn = ssh::connect(&RemoteSpec {
        host: env.host.clone(),
        remote_root: env.root.clone(),
        local_binary: Some(env.binary.clone()),
    })
    .await
    .expect("ssh::connect");
    eprintln!("remote up: {} ({})", conn.ws_url, conn.arch.slug());

    // 2) Connect the real provider, pointed at the remote via extra.remote.
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(env.key.clone()),
            extra: json!({
                "model": env.model.clone(),
                "base_url": env.base_url.clone(),
                "research": false,
                // Auto-allow the mutating tools so the loop runs unattended.
                "permissions": { "write_file": "allow", "edit_file": "allow", "apply_patch": "allow", "bash": "allow" },
                "remote": { "ws_url": conn.ws_url, "token": conn.token, "cwd": env.root.clone() },
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
         1. Create a file named `{MARKER}` in the project root with exactly `{CONTENT}` followed by one newline and no other bytes.\n\
         2. Run the shell command `hostname` and tell me its output.\n\
         Use your tools. Do not ask for confirmation."
    );
    let mut stream = provider
        .prompt(&session.id, PromptInput::text(prompt))
        .await
        .expect("prompt");

    // 4) Drive the loop to completion (auto-approve any gate, just in case).
    let mut text = String::new();
    let mut tools = Vec::new();
    let status = tokio::time::timeout(Duration::from_secs(180), async {
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::ToolCall { call, .. } => {
                    tools.push(call.tool_name.unwrap_or(call.title));
                }
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
    .expect("agent run timed out");

    eprintln!("tools: {tools:?}");
    eprintln!("final text: {text}");
    assert_done(status, &tools, &text);
    assert!(
        tools
            .iter()
            .any(|tool| tool == "write_file" || tool == "apply_patch"),
        "expected a file mutation tool: {tools:?}"
    );
    assert!(
        tools.iter().any(|tool| tool == "bash"),
        "expected bash: {tools:?}"
    );

    // 5) The real proof: the file the *model* asked to create exists **on the
    //    remote**, with the content it was told to write.
    let out = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &env.host,
            &format!("cat {root}/{MARKER}", root = env.root),
        ])
        .output()
        .expect("ssh cat");
    let on_remote = String::from_utf8_lossy(&out.stdout);
    eprintln!("remote file contents: {on_remote:?}");
    assert_eq!(
        on_remote,
        format!("{CONTENT}\n"),
        "the agent's file did not land on the remote with exact contents"
    );

    // Cleanup.
    let _ = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &env.host,
            &format!("rm -f {root}/{MARKER}", root = env.root),
        ])
        .status();
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote agent e2e: OK");
}

/// End-to-end with SKILLS: seed several Claude skills on the remote, then run a
/// real conversation that must pick the right one, read its SKILL.md over the
/// tunnel, and produce exactly what it specifies — on the remote.
#[tokio::test]
#[ignore = "needs a live SSH host + a real ck_live_ key; see file header"]
async fn remote_agent_selects_and_follows_a_claude_skill() {
    let Some(env) = live_env() else { return };

    // Seed three skills on the remote; only `release-notes` is asked for. The
    // marker lines exist ONLY inside the SKILL.md, so producing them proves the
    // agent read + followed the skill.
    let seed = format!(
         "rm -rf {root}/.claude {root}/RELEASE_NOTES.md {root}/CHANGELOG.md {root}/GREETING.txt; \
         mkdir -p {root}/.claude/skills/release-notes {root}/.claude/skills/changelog {root}/.claude/skills/greeter && \
         printf '%s\\n' '---' 'name: release-notes' 'description: Generate release notes for this project when the user asks for release notes.' '---' \
           'When asked for release notes, create RELEASE_NOTES.md in the project root. Its FIRST line must be exactly:' \
           '# Clark Code Release Notes' 'and its LAST line must be exactly:' '-- generated by the release-notes skill' \
           > {root}/.claude/skills/release-notes/SKILL.md && \
         printf '%s\\n' '---' 'name: changelog' 'description: Maintain a CHANGELOG.md.' '---' 'Create CHANGELOG.md.' \
           > {root}/.claude/skills/changelog/SKILL.md && \
         printf '%s\\n' '---' 'name: greeter' 'description: Say hello in GREETING.txt.' '---' 'Create GREETING.txt.' \
           > {root}/.claude/skills/greeter/SKILL.md",
        root = env.root
    );
    assert!(
        Command::new("ssh")
            .args(["-o", "ConnectTimeout=10", &env.host, &seed])
            .status()
            .expect("seed")
            .success(),
        "failed to seed remote skills"
    );

    let conn = ssh::connect(&RemoteSpec {
        host: env.host.clone(),
        remote_root: env.root.clone(),
        local_binary: Some(env.binary.clone()),
    })
    .await
    .expect("ssh::connect");
    eprintln!("remote up: {} ({})", conn.ws_url, conn.arch.slug());

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(env.key.clone()),
            extra: json!({
                "model": env.model.clone(),
                "base_url": env.base_url.clone(),
                "research": false,
                "permissions": { "write_file": "allow", "edit_file": "allow", "apply_patch": "allow", "bash": "allow" },
                "remote": { "ws_url": conn.ws_url, "token": conn.token, "cwd": env.root.clone() },
            }),
            ..Default::default()
        })
        .await
        .expect("provider.connect");
    // new_session is where skills are discovered from the remote .claude and
    // injected into the system prompt.
    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("new_session");

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Generate release notes for this project using your release-notes skill. Then stop."),
        )
        .await
        .expect("prompt");

    let mut tools = Vec::new();
    let mut text = String::new();
    let status = tokio::time::timeout(Duration::from_secs(180), async {
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::ToolCall { call, .. } => tools.push(call.title),
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
    .expect("timed out");
    eprintln!("tools: {tools:?}");
    eprintln!("final: {text}");
    assert_done(status, &tools, &text);

    // The agent must have consulted the skill file.
    assert!(
        tools
            .iter()
            .any(|t| t.contains("SKILL.md") || t.contains("release-notes")),
        "expected the agent to read the release-notes SKILL.md; tools = {tools:?}"
    );

    // The real proof: RELEASE_NOTES.md on the remote has BOTH marker lines the
    // skill dictated, and the decoy skills' files were NOT created.
    let notes = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            &env.host,
            &format!(
                "cat {root}/RELEASE_NOTES.md 2>/dev/null; echo '<<<'; ls {root}",
                root = env.root
            ),
        ])
        .output()
        .expect("cat");
    let out = String::from_utf8_lossy(&notes.stdout);
    eprintln!("remote RELEASE_NOTES.md + listing:\n{out}");
    assert!(
        out.contains("# Clark Code Release Notes"),
        "missing first marker line: {out}"
    );
    assert!(
        out.contains("-- generated by the release-notes skill"),
        "missing last marker line: {out}"
    );
    assert!(
        !out.contains("GREETING.txt"),
        "the greeter skill should not have run: {out}"
    );

    let cleanup = format!(
        "rm -rf {root}/.claude {root}/RELEASE_NOTES.md {root}/CHANGELOG.md {root}/GREETING.txt",
        root = env.root
    );
    let _ = Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", &env.host, &cleanup])
        .status();
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("remote skill e2e: OK");
}
