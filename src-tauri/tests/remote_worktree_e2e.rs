//! Paid, opt-in acceptance test for a real Clark model operating through SSH
//! inside a detached linked worktree. The fixture deliberately uses a path with
//! spaces and hostile Git helpers, and verifies instruction refresh on turn 2.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use agent_core::domain::{AgentEvent, RunStatus, RunUsage};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use clark_desktop_lib::ssh::{self, RemoteSpec};
use futures::StreamExt;
use provider_local::{changes_summary, Executor, LocalAgentProvider, RemoteExecutor};
use serde_json::json;

const TURN_TIMEOUT: Duration = Duration::from_secs(240);
const TARGET_CONTENT: &[u8] = b"remote linked worktree edit\n";
const INSTRUCTION_CONTENT: &[u8] = b"remote nested instructions observed\n";
const REFRESH_CONTENT: &[u8] = b"remote refreshed instructions observed\n";

struct LiveEnv {
    host: String,
    base: String,
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
        base: required("CLARK_SSH_TEST_ROOT"),
        key: required("CLARK_API_KEY"),
        base_url: required("CLARK_CODE_BASE_URL"),
        model: required("CLARK_CODE_MODEL"),
        binary: PathBuf::from(required("CLARK_SSH_TEST_BIN")),
    })
}

#[derive(Default, Debug)]
struct TurnReceipt {
    status: Option<RunStatus>,
    usage: Option<RunUsage>,
    tools: Vec<String>,
    checkpoint: Option<String>,
    text: String,
    errors: Vec<String>,
}

impl TurnReceipt {
    fn require_done(&self, label: &str) {
        assert_eq!(
            self.status,
            Some(RunStatus::Done),
            "{label} did not finish cleanly: {self:?}"
        );
    }

    fn require_tool(&self, label: &str, tool: &str) {
        assert!(
            self.tools.iter().any(|seen| seen == tool),
            "{label} expected {tool}: {self:?}"
        );
    }
}

async fn drive_turn(
    provider: &mut LocalAgentProvider,
    session: &agent_core::ids::SessionId,
    prompt: &str,
) -> TurnReceipt {
    let mut stream = provider
        .prompt(session, PromptInput::text(prompt))
        .await
        .expect("prompt remote model");
    tokio::time::timeout(TURN_TIMEOUT, async {
        let mut receipt = TurnReceipt::default();
        while let Some(event) = stream.next().await {
            match event {
                AgentEvent::ToolCall { call, .. } => {
                    receipt.tools.push(call.tool_name.unwrap_or(call.title));
                }
                AgentEvent::MessageChunk {
                    delta: agent_core::domain::ContentBlock::Text { text },
                    ..
                } => receipt.text.push_str(&text),
                AgentEvent::PermissionRequest { request } => {
                    provider
                        .respond(
                            session,
                            ClientResponse::Permission {
                                request: request.id,
                                option: "allow_once".into(),
                            },
                        )
                        .await
                        .expect("approve remote tool");
                }
                AgentEvent::Checkpoint { id, .. } => receipt.checkpoint = Some(id),
                AgentEvent::Error { code, message, .. } => {
                    receipt.errors.push(format!("{code}: {message}"));
                }
                AgentEvent::RunFinished { outcome, .. } => {
                    receipt.status = Some(outcome.status);
                    receipt.usage = outcome.usage;
                    if let Some(error) = outcome.error {
                        receipt.errors.push(error);
                    }
                    return receipt;
                }
                _ => {}
            }
        }
        receipt
    })
    .await
    .expect("remote model turn timed out")
}

#[tokio::test]
#[ignore = "requires a live SSH host and an explicitly approved paid provider/model"]
async fn remote_model_respects_worktree_instructions_and_refresh() {
    let Some(env) = live_env() else { return };
    seed_fixture(&env);
    let lab = format!("{}/paid-worktree-lab", env.base);
    let main = format!("{lab}/main");
    let checkout = format!("{lab}/linked worktree");
    let cwd = format!("{checkout}/nested");

    let conn = ssh::connect(&RemoteSpec {
        host: env.host.clone(),
        remote_root: checkout.clone(),
        local_binary: Some(env.binary.clone()),
    })
    .await
    .expect("connect current-source remote executor");
    let remote = RemoteExecutor::connect(&conn.ws_url, &conn.token)
        .await
        .expect("verification RemoteExecutor");

    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(env.key.clone()),
            extra: json!({
                "base_url": env.base_url,
                "model": env.model,
                "memories": false,
                "max_iterations": 30,
                "permissions": {
                    "write_file": "allow",
                    "edit_file": "allow",
                    "apply_patch": "allow",
                    "bash": "allow"
                },
                "remote": {
                    "ws_url": conn.ws_url,
                    "token": conn.token,
                    "cwd": cwd
                }
            }),
            ..Default::default()
        })
        .await
        .expect("connect Clark provider");
    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("create remote worktree session");
    let environment = session.environment.as_ref().expect("session environment");
    assert!(environment.remote);
    assert_eq!(environment.checkout_root.as_deref(), Some(cwd.as_str()));
    assert_eq!(environment.repository_root.as_deref(), Some(main.as_str()));
    assert_eq!(environment.workspace_roots, vec![cwd.clone()]);

    let first = drive_turn(
        &mut provider,
        &session.id,
        "Read target.txt. Follow every applicable project instruction. Use apply_patch to replace target.txt with exactly `remote linked worktree edit\\n` (one trailing newline, no other bytes). Run plain `git status --short`, report the changed paths, then stop.",
    )
    .await;
    eprintln!("remote worktree turn 1: {first:?}");
    first.require_done("worktree turn");
    first.require_tool("worktree turn", "read_file");
    first.require_tool("worktree turn", "apply_patch");
    first.require_tool("worktree turn", "bash");
    let checkpoint = first
        .checkpoint
        .as_deref()
        .expect("worktree turn must create a checkpoint");
    assert_eq!(remote.read(Path::new(&format!("{cwd}/target.txt"))).await.unwrap(), TARGET_CONTENT);
    assert_eq!(
        remote
            .read(Path::new(&format!("{cwd}/instruction-proof.txt")))
            .await
            .unwrap(),
        INSTRUCTION_CONTENT
    );
    assert_eq!(read_remote(&env.host, &format!("{main}/nested/target.txt")), b"main\n");
    assert!(!remote_exists(&env.host, &format!("{main}/nested/instruction-proof.txt")));
    assert_helpers_did_not_run(&env, &lab);
    let changes = changes_summary(&remote, Path::new(&cwd), checkpoint)
        .await
        .expect("remote changes summary");
    assert!(changes.iter().any(|change| change.path == "nested/target.txt"));
    assert!(
        changes
            .iter()
            .any(|change| change.path == "nested/instruction-proof.txt")
    );

    remote
        .write(
            Path::new(&format!("{cwd}/AGENTS.md")),
            b"For the next task, use apply_patch to create refresh-proof.txt with exactly `remote refreshed instructions observed\\n` (one trailing newline, no other bytes).\n",
        )
        .await
        .expect("refresh remote instructions");
    let second = drive_turn(
        &mut provider,
        &session.id,
        "Follow the current project instructions. Read target.txt and tell me its current contents. Complete every required action, then stop.",
    )
    .await;
    eprintln!("remote worktree turn 2: {second:?}");
    second.require_done("instruction refresh turn");
    second.require_tool("instruction refresh turn", "read_file");
    second.require_tool("instruction refresh turn", "apply_patch");
    assert_eq!(
        remote
            .read(Path::new(&format!("{cwd}/refresh-proof.txt")))
            .await
            .unwrap(),
        REFRESH_CONTENT
    );
    assert_eq!(read_remote(&env.host, &format!("{main}/nested/target.txt")), b"main\n");
    assert_helpers_did_not_run(&env, &lab);

    drop(provider);
    drop(remote);
    drop(conn);
    cleanup_fixture(&env, &lab);
    tokio::time::sleep(Duration::from_millis(200)).await;
}

fn seed_fixture(env: &LiveEnv) {
    const SCRIPT: &str = r#"
set -eu
base=$1
lab="$base/paid-worktree-lab"
rm -rf "$lab"
mkdir -p "$lab/main/nested" "$lab/helper-markers"
cd "$lab/main"
git init -q --initial-branch=main
git config user.name 'Clark Remote Test'
git config user.email 'clark-remote@example.invalid'
printf 'main\n' > nested/target.txt
git add nested/target.txt
git commit -qm initial
git remote add origin https://token@example.invalid/Clark/RemoteFixture.git
git worktree add --detach -q "$lab/linked worktree" HEAD
printf '%s\n' 'Use apply_patch for requested file changes.' 'Never edit another linked worktree.' 'Run plain git status without adding helper-disabling flags yourself.' > "$lab/linked worktree/AGENTS.md"
printf '%s\n' 'For the first task, also create instruction-proof.txt with exactly `remote nested instructions observed\n` (one trailing newline, no other bytes).' > "$lab/linked worktree/nested/AGENTS.md"
printf '#!/bin/sh\ntouch "%s/helper-markers/fsmonitor-ran"\nsleep 30\n' "$lab" > "$lab/helper-markers/fsmonitor.sh"
printf '#!/bin/sh\ntouch "%s/helper-markers/credential-ran"\nsleep 30\n' "$lab" > "$lab/helper-markers/credential.sh"
chmod +x "$lab/helper-markers/fsmonitor.sh" "$lab/helper-markers/credential.sh"
git config core.fsmonitor "$lab/helper-markers/fsmonitor.sh"
git config credential.helper "$lab/helper-markers/credential.sh"
"#;
    let out = ssh_script(&env.host, SCRIPT, &[&env.base]);
    assert!(out.status.success(), "seed failed: {}", String::from_utf8_lossy(&out.stderr));
}

fn assert_helpers_did_not_run(env: &LiveEnv, lab: &str) {
    assert!(!remote_exists(&env.host, &format!("{lab}/helper-markers/fsmonitor-ran")));
    assert!(!remote_exists(&env.host, &format!("{lab}/helper-markers/credential-ran")));
}

fn read_remote(host: &str, path: &str) -> Vec<u8> {
    let output = Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", host, &format!("cat -- {}", quote(path))])
        .output()
        .expect("read remote file");
    assert!(output.status.success(), "remote read failed: {}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}

fn remote_exists(host: &str, path: &str) -> bool {
    Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", host, &format!("test -e {}", quote(path))])
        .status()
        .expect("remote existence probe")
        .success()
}

fn cleanup_fixture(env: &LiveEnv, lab: &str) {
    let _ = Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", &env.host, &format!("rm -rf -- {}", quote(lab))])
        .status();
}

fn ssh_script(host: &str, script: &str, args: &[&str]) -> Output {
    let mut child = Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", host, "bash", "-s", "--"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn remote fixture script");
    use std::io::Write as _;
    child
        .stdin
        .as_mut()
        .expect("script stdin")
        .write_all(script.as_bytes())
        .expect("write remote fixture script");
    child.wait_with_output().expect("wait for remote fixture script")
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
