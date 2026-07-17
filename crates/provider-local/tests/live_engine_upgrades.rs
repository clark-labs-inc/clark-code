//! Live end-to-end exercise of the Codex-inspired engine upgrades — mid-run
//! steering, usage-driven/forced compaction, parallel read batches, reasoning
//! replay on the wire, and tool-output truncation — against the REAL
//! clark-code model in a throwaway sandbox git repo.
//!
//! Costs real credits and is model-behavior dependent, so it only runs when
//! live env is explicit:
//!
//! CLARK_CODE_LIVE=1 CLARK_CODE_API_KEY=ck_live_... \
//!   cargo test -p provider-local --test live_engine_upgrades -- --ignored --nocapture --test-threads=1

use agent_core::domain::{AgentEvent, Role, RunStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_local::LocalAgentProvider;
use serde_json::json;
use std::path::Path;
use std::process::Command;

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
    let base_url = std::env::var("CLARK_CODE_BASE_URL")
        .unwrap_or_else(|_| "https://api.clarkslabs.com/v1".to_string());
    let model = std::env::var("CLARK_CODE_MODEL").unwrap_or_else(|_| "clark-code".to_string());
    let api_key = match std::env::var("CLARK_CODE_API_KEY") {
        Ok(key) if !key.trim().is_empty() => key,
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

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

/// Seed a real git repo the agent will work in.
fn sandbox_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "e2e@clark.test"]);
    git(dir.path(), &["config", "user.name", "clark e2e"]);
    std::fs::create_dir_all(dir.path().join("notes")).unwrap();
    std::fs::write(
        dir.path().join("notes/a.md"),
        "Project fact A: the launch date is March 3.\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("notes/b.md"),
        "Project fact B: the venue is Pier 48.\n",
    )
    .unwrap();
    // Large enough that reading it must trip the middle-out truncation
    // (~40k-char cap) and, with the tiny compaction threshold below, force a
    // REAL mid-run compaction pass.
    let filler: String = (0..6_000)
        .map(|i| format!("log line {i}: nothing important here\n"))
        .collect();
    std::fs::write(dir.path().join("big.txt"), filler).unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "seed"]);
    dir
}

#[tokio::test]
#[ignore = "live network + credits; run explicitly with CLARK_CODE_LIVE=1"]
async fn live_steering_compaction_and_parallel_reads_in_sandbox_repo() {
    let Some(cfg) = live_config() else { return };
    let dir = sandbox_repo();

    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(cfg.api_key.clone()),
            extra: json!({
                "base_url": cfg.base_url,
                "model": cfg.model,
                "memories": false,
                // Tiny threshold: the big.txt read (~10k tokens even after
                // truncation) must push the transcript over it, so a real
                // compaction pass runs inside this turn.
                "auto_compact_token_limit": 3000,
                "compact_request_token_limit": 50_000,
            }),
            ..Default::default()
        })
        .await
        .expect("connect");
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            resume: None,
        })
        .await
        .expect("session");

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text(
                "Read notes/a.md and notes/b.md (you may read them in parallel), skim big.txt, \
                 then create SUMMARY.md stating the launch date and the venue in one sentence \
                 each.",
            ),
        )
        .await
        .expect("prompt");

    let mut steered = false;
    let mut saw_compaction_note = false;
    let mut finished_status = None;
    let mut context_limit = None;
    while let Some(ev) = stream.next().await {
        match &ev {
            AgentEvent::ToolCall { call, .. } => {
                eprintln!("tool: {}", call.title);
                if !steered {
                    steered = true;
                    provider
                        .steer(
                            &session.id,
                            PromptInput::text(
                                "Important mid-task change: SUMMARY.md must END with the exact \
                                 word STEERWORKS on its own line.",
                            ),
                        )
                        .await
                        .expect("steer active run");
                    eprintln!("steered the active run");
                }
            }
            AgentEvent::MessageChunk {
                role: Role::System, ..
            } => {
                saw_compaction_note = true;
                eprintln!("compaction note surfaced");
            }
            AgentEvent::PermissionRequest { request } => {
                provider
                    .respond(
                        &session.id,
                        ClientResponse::Permission {
                            request: request.id.clone(),
                            option: "allow_once".into(),
                            feedback: None,
                        },
                    )
                    .await
                    .expect("allow permission");
            }
            AgentEvent::Error { code, message, .. } => {
                eprintln!("error event: {code}: {message}");
            }
            AgentEvent::RunFinished { outcome, .. } => {
                finished_status = Some(outcome.status);
                context_limit = outcome.usage.as_ref().and_then(|u| u.context_limit);
                break;
            }
            _ => {}
        }
    }

    assert_eq!(
        finished_status,
        Some(RunStatus::Done),
        "live run must finish cleanly"
    );
    assert!(steered, "the run must have been steered mid-flight");
    assert_eq!(
        context_limit,
        Some(3000),
        "the run usage must carry the engine's real compaction threshold"
    );
    assert!(
        saw_compaction_note,
        "the forced-low threshold must produce a visible compaction note"
    );

    let summary = std::fs::read_to_string(dir.path().join("SUMMARY.md"))
        .expect("the model must have created SUMMARY.md");
    eprintln!("SUMMARY.md:\n{summary}");
    let lower = summary.to_lowercase();
    assert!(
        lower.contains("march 3") && (lower.contains("pier 48")),
        "facts from the parallel reads must land in the summary: {summary}"
    );
    assert!(
        summary.contains("STEERWORKS"),
        "the steered instruction must be honored: {summary}"
    );
}
