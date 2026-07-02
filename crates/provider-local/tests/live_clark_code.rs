//! Live `clark-code` conversation matrix for the local coding provider.
//!
//! Ignored by default. This makes real Clark Platform model calls and mutates a
//! temporary project directory only when live env is explicit:
//!
//! ```sh
//! CLARK_CODE_LIVE=1 \
//! CLARK_CODE_BASE_URL=https://api.clarkslabs.com/v1 \
//! CLARK_CODE_MODEL=clark-code \
//! CLARK_CODE_API_KEY=ck_live_... \
//!   cargo test -p provider-local --test live_clark_code -- --ignored --nocapture --test-threads=1
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_core::domain::{AgentEvent, ContentBlock, RunStatus, ToolStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_local::LocalAgentProvider;
use serde_json::json;

const TURN_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug)]
struct LiveConfig {
    base_url: String,
    model: String,
    api_key: String,
}

fn live_config() -> Option<LiveConfig> {
    if std::env::var("CLARK_CODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping: set CLARK_CODE_LIVE=1 to permit live clark-code calls");
        return None;
    }
    let base_url = match std::env::var("CLARK_CODE_BASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            eprintln!("skipping: set CLARK_CODE_BASE_URL to the Clark Platform provider URL");
            return None;
        }
    };
    let model = match std::env::var("CLARK_CODE_MODEL") {
        Ok(value) if value.trim() == "clark-code" => value,
        Ok(value) => {
            eprintln!("skipping: CLARK_CODE_MODEL must be clark-code, got {value:?}");
            return None;
        }
        Err(_) => {
            eprintln!("skipping: set CLARK_CODE_MODEL=clark-code");
            return None;
        }
    };
    let api_key = match std::env::var("CLARK_CODE_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            eprintln!("skipping: set CLARK_CODE_API_KEY");
            return None;
        }
    };
    Some(LiveConfig {
        base_url,
        model,
        api_key,
    })
}

#[derive(Default, Debug)]
struct TurnSummary {
    finished: bool,
    status: Option<RunStatus>,
    run_error: Option<String>,
    usage: Option<agent_core::domain::RunUsage>,
    text: String,
    tools: Vec<String>,
    errors: Vec<String>,
    tool_statuses: BTreeMap<String, Vec<ToolStatus>>,
    permission_requests: usize,
    event_counts: BTreeMap<&'static str, usize>,
}

impl TurnSummary {
    fn require_done(&self, label: &str) {
        assert!(self.finished, "{label}: run did not finish: {self:?}");
        assert_eq!(
            self.status,
            Some(RunStatus::Done),
            "{label}: run did not finish cleanly: {self:?}"
        );
    }

    fn require_tool(&self, label: &str, tool: &str) {
        assert!(
            self.tools.iter().any(|seen| seen == tool),
            "{label}: expected tool {tool}, got {:?}",
            self.tools
        );
    }
}

async fn new_live_provider(
    cfg: &LiveConfig,
    cwd: &Path,
    extra: serde_json::Value,
) -> (LocalAgentProvider, agent_core::provider::Session) {
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(cfg.api_key.clone()),
            extra: merge_extra(
                json!({
                    "base_url": cfg.base_url,
                    "model": cfg.model,
                    "cwd": cwd.to_string_lossy(),
                }),
                extra,
            ),
            ..Default::default()
        })
        .await
        .expect("connect local clark-code provider");
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(cwd.to_string_lossy().to_string()),
            mode: None,
        })
        .await
        .expect("new local session");
    (provider, session)
}

fn merge_extra(mut base: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let Some(base_obj) = base.as_object_mut() else {
        return extra;
    };
    if let Some(extra_obj) = extra.as_object() {
        for (key, value) in extra_obj {
            base_obj.insert(key.clone(), value.clone());
        }
    }
    base
}

async fn drive_turn(
    provider: &mut LocalAgentProvider,
    session_id: &agent_core::ids::SessionId,
    prompt: &str,
) -> TurnSummary {
    let mut stream = provider
        .prompt(session_id, PromptInput::text(prompt))
        .await
        .expect("prompt");
    let mut summary = TurnSummary::default();
    let collect = async {
        while let Some(ev) = stream.next().await {
            *summary.event_counts.entry(event_name(&ev)).or_default() += 1;
            match ev {
                AgentEvent::MessageChunk {
                    delta: ContentBlock::Text { text },
                    ..
                } => summary.text.push_str(&text),
                AgentEvent::ToolCall { call, .. } => {
                    let tool = tool_name_from_title(&call.title);
                    summary.tools.push(tool);
                }
                AgentEvent::ToolCallUpdate { id, patch, .. } => {
                    if let Some(status) = patch.status {
                        summary
                            .tool_statuses
                            .entry(id.as_str().to_string())
                            .or_default()
                            .push(status);
                    }
                }
                AgentEvent::PermissionRequest { request } => {
                    summary.permission_requests += 1;
                    provider
                        .respond(
                            session_id,
                            ClientResponse::Permission {
                                request: request.id,
                                option: "allow_once".into(),
                            },
                        )
                        .await
                        .expect("allow permission once");
                }
                AgentEvent::RunFinished { outcome, .. } => {
                    summary.finished = true;
                    summary.status = Some(outcome.status);
                    summary.run_error = outcome.error;
                    summary.usage = outcome.usage;
                    break;
                }
                AgentEvent::Error { code, message, .. } => {
                    summary.errors.push(format!("{code}: {message}"));
                }
                _ => {}
            }
        }
        summary
    };
    tokio::time::timeout(TURN_TIMEOUT, collect)
        .await
        .expect("live clark-code turn timed out")
}

fn event_name(ev: &AgentEvent) -> &'static str {
    match ev {
        AgentEvent::RunStarted { .. } => "RunStarted",
        AgentEvent::Checkpoint { .. } => "Checkpoint",
        AgentEvent::MessageChunk { .. } => "MessageChunk",
        AgentEvent::ToolCall { .. } => "ToolCall",
        AgentEvent::ToolCallUpdate { .. } => "ToolCallUpdate",
        AgentEvent::Plan { .. } => "Plan",
        AgentEvent::PermissionRequest { .. } => "PermissionRequest",
        AgentEvent::Artifact { .. } => "Artifact",
        AgentEvent::Surface { .. } => "Surface",
        AgentEvent::ModeChanged { .. } => "ModeChanged",
        AgentEvent::RunFinished { .. } => "RunFinished",
        AgentEvent::Error { .. } => "Error",
    }
}

fn tool_name_from_title(title: &str) -> String {
    title.split(':').next().unwrap_or(title).trim().to_string()
}

fn write_project_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    println!(\"CLARK_LIVE_READ_SENTINEL_7391\");\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("notes/alpha.md"),
        "alpha note with CLARK_LIVE_GREP_SENTINEL_5142\n",
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env; makes real model calls"]
async fn live_clark_code_feature_matrix() {
    let Some(cfg) = live_config() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_project_fixture(dir.path());

    let (mut provider, session) = new_live_provider(
        &cfg,
        dir.path(),
        json!({
            "research": false,
            "memories": true,
            "permissions": {
                "bash": "ask",
                "write_file": "ask",
                "edit_file": "ask"
            }
        }),
    )
    .await;

    let pong = drive_turn(
        &mut provider,
        &session.id,
        "Reply with exactly this token and no extra words: CLARK_LIVE_PONG_2001",
    )
    .await;
    println!("[pong] {pong:?}");
    pong.require_done("pong");
    assert!(
        pong.text.contains("CLARK_LIVE_PONG_2001"),
        "pong: expected sentinel in assistant text: {:?}",
        pong.text
    );

    let read_search = drive_turn(
        &mut provider,
        &session.id,
        "Use list_dir, glob, grep, and read_file. List the project root, glob for **/*.rs, grep for CLARK_LIVE_GREP_SENTINEL_5142, read src/main.rs, then answer with both CLARK_LIVE_READ_SENTINEL_7391 and CLARK_LIVE_GREP_SENTINEL_5142.",
    )
    .await;
    println!("[read_search] {read_search:?}");
    read_search.require_done("read_search");
    for tool in ["list_dir", "glob", "grep", "read_file"] {
        read_search.require_tool("read_search", tool);
    }
    assert!(read_search.text.contains("CLARK_LIVE_READ_SENTINEL_7391"));
    assert!(read_search.text.contains("CLARK_LIVE_GREP_SENTINEL_5142"));

    let mutate = drive_turn(
        &mut provider,
        &session.id,
        "Use write_file to create live.txt with content `alpha`. Read live.txt. Use edit_file to replace alpha with beta. Then use bash to run `cat live.txt > bash-copy.txt`. Read bash-copy.txt and answer with CLARK_LIVE_MUTATE_DONE.",
    )
    .await;
    println!("[mutate] {mutate:?}");
    mutate.require_done("mutate");
    for tool in ["write_file", "read_file", "edit_file", "bash"] {
        mutate.require_tool("mutate", tool);
    }
    assert!(
        mutate.permission_requests >= 3,
        "mutate: expected write/edit/bash permission prompts, got {mutate:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("live.txt")).unwrap(),
        "beta"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("bash-copy.txt")).unwrap(),
        "beta"
    );

    let memory = drive_turn(
        &mut provider,
        &session.id,
        "Use the memory tool to remember a project fact titled `live e2e sentinel` with content `CLARK_MEMORY_SENTINEL_8402`. Then use memory recall and answer with CLARK_MEMORY_SENTINEL_8402.",
    )
    .await;
    println!("[memory] {memory:?}");
    memory.require_done("memory");
    memory.require_tool("memory", "memory");
    assert!(memory.text.contains("CLARK_MEMORY_SENTINEL_8402"));
    assert!(
        find_file_containing(
            &dir.path().join(".clark/memory"),
            "CLARK_MEMORY_SENTINEL_8402"
        )
        .is_some(),
        "memory: expected project memory file to contain sentinel"
    );
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env; makes real model calls"]
async fn live_clark_code_compacts_and_continues() {
    let Some(cfg) = live_config() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_project_fixture(dir.path());

    let (mut provider, session) = new_live_provider(
        &cfg,
        dir.path(),
        json!({
            "research": false,
            "memories": false,
            "auto_compact_token_limit": 2_000,
            "compact_request_token_limit": 1_600,
            "compact_recent_user_token_budget": 400
        }),
    )
    .await;
    let prompt = format!(
        "This turn intentionally exceeds the live compaction threshold. {}\nNow answer with exactly CLARK_LIVE_COMPACTION_DONE_3003.",
        "Important project context. ".repeat(900)
    );
    let summary = drive_turn(&mut provider, &session.id, &prompt).await;
    println!("[compaction] {summary:?}");
    summary.require_done("compaction");
    assert!(
        summary.text.contains("CLARK_LIVE_COMPACTION_DONE_3003"),
        "compaction: expected sentinel after compaction: {:?}",
        summary.text
    );
}

#[tokio::test]
#[ignore = "requires explicit live clark-code env plus CLARK_CODE_LIVE_RESEARCH=1"]
async fn live_clark_code_research_tool() {
    if std::env::var("CLARK_CODE_LIVE_RESEARCH").ok().as_deref() != Some("1") {
        eprintln!("skipping: set CLARK_CODE_LIVE_RESEARCH=1 to permit research tool spend");
        return;
    }
    let Some(cfg) = live_config() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_project_fixture(dir.path());
    let (mut provider, session) = new_live_provider(
        &cfg,
        dir.path(),
        json!({
            "research": true,
            "research_model": cfg.model,
            "memories": false
        }),
    )
    .await;
    let research = drive_turn(
        &mut provider,
        &session.id,
        "Use clark_research to check the current official Rust programming language website domain. Then answer with CLARK_RESEARCH_SENTINEL and the domain.",
    )
    .await;
    println!("[research] {research:?}");
    research.require_done("research");
    research.require_tool("research", "clark_research");
    assert!(research.text.contains("CLARK_RESEARCH_SENTINEL"));
    assert!(
        research.text.contains("rust-lang.org"),
        "research: expected rust-lang.org in answer: {:?}",
        research.text
    );
}

fn find_file_containing(root: &Path, needle: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_containing(&path, needle) {
                return Some(found);
            }
        } else if std::fs::read_to_string(&path)
            .ok()
            .is_some_and(|text| text.contains(needle))
        {
            return Some(path);
        }
    }
    None
}
