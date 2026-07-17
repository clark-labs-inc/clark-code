//! goal_eval — the autonomous-goal quality benchmark for clark-code.
//!
//! Each scenario hands the REAL local provider a "build the whole thing"
//! request in a throwaway git repo, lets goal mode drive it autonomously
//! (create_goal → engine continuation turns → update_goal), then scores the
//! produced artifacts against a programmatic rubric. It measures OUTCOMES —
//! files that exist and contain what a finished deliverable must contain —
//! plus the run's efficiency (tokens, cost, wall time, continuation turns).
//!
//! Live-gated (real model, real credits):
//!
//! CLARK_CODE_LIVE=1 CLARK_CODE_API_KEY=ck_live_... \
//!   cargo run -p provider-local --example goal_eval
//!
//! Optional: CLARK_CODE_MODEL (default clark-code = GLM 5.2),
//! GOAL_EVAL_SCENARIOS=snake,website (default: both).

use agent_core::domain::{AgentEvent, Role, RunStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_local::LocalAgentProvider;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Hard wall per scenario — a stuck run is itself a finding, not a hang.
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(35 * 60);
/// Token budget the prompt asks the model to set on its goal — bounds spend
/// and exercises the budget machinery.
const GOAL_TOKEN_BUDGET: u64 = 300_000;

struct Check {
    label: &'static str,
    weight: u32,
    passed: bool,
}

struct Scenario {
    id: &'static str,
    prompt: String,
    grade: fn(&Path) -> Vec<Check>,
}

fn check(label: &'static str, weight: u32, passed: bool) -> Check {
    Check {
        label,
        weight,
        passed,
    }
}

fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            id: "snake",
            prompt: format!(
                "Build a complete, playable Snake game as a single self-contained index.html \
                 (inline CSS + JS, no external dependencies). It must have: canvas rendering, \
                 arrow-key controls, food that grows the snake, a visible score, collision \
                 game-over, and a restart control. Pursue this autonomously until it is \
                 genuinely complete: create a goal with create_goal (token_budget \
                 {GOAL_TOKEN_BUDGET}) and keep working until the game is fully done and \
                 verified."
            ),
            grade: |dir| {
                let html = read(dir, "index.html").unwrap_or_default();
                let lower = html.to_lowercase();
                vec![
                    check("index.html exists", 15, !html.is_empty()),
                    check("has <canvas>", 10, lower.contains("<canvas")),
                    check("has inline <script>", 5, lower.contains("<script")),
                    check("substantial (≥4KB)", 10, html.len() >= 4_000),
                    check(
                        "keyboard controls",
                        10,
                        lower.contains("arrowup") || lower.contains("keydown"),
                    ),
                    check("score", 10, lower.contains("score")),
                    check(
                        "game over",
                        10,
                        lower.contains("game over") || lower.contains("gameover"),
                    ),
                    check("restart", 10, lower.contains("restart")),
                    check(
                        "game loop",
                        10,
                        lower.contains("requestanimationframe") || lower.contains("setinterval"),
                    ),
                    check(
                        "no external deps",
                        10,
                        !lower.contains("src=\"http") && !lower.contains("href=\"http"),
                    ),
                ]
            },
        },
        Scenario {
            id: "website",
            prompt: format!(
                "Build a complete small portfolio website for a fictional product designer \
                 named Riley Chen: index.html, about.html, projects.html, and a shared \
                 styles.css. Every page needs the same working nav linking all three pages, \
                 the stylesheet linked, a responsive viewport meta tag, and real content (no \
                 lorem ipsum). Pursue this autonomously until it is genuinely complete: \
                 create a goal with create_goal (token_budget {GOAL_TOKEN_BUDGET}) and keep \
                 working until the whole site is done and verified."
            ),
            grade: |dir| {
                let pages = ["index.html", "about.html", "projects.html"];
                let bodies: Vec<Option<String>> = pages.iter().map(|p| read(dir, p)).collect();
                let css = read(dir, "styles.css").unwrap_or_default();
                let all_lower: Vec<String> = bodies
                    .iter()
                    .map(|b| b.clone().unwrap_or_default().to_lowercase())
                    .collect();
                let total_html: usize = bodies
                    .iter()
                    .map(|b| b.as_deref().map(str::len).unwrap_or(0))
                    .sum();
                let mut checks = vec![
                    check("index.html exists", 10, bodies[0].is_some()),
                    check("about.html exists", 10, bodies[1].is_some()),
                    check("projects.html exists", 10, bodies[2].is_some()),
                    check("styles.css exists (≥300B)", 10, css.len() >= 300),
                    check(
                        "all pages link styles.css",
                        10,
                        all_lower.iter().all(|b| b.contains("styles.css")),
                    ),
                    check(
                        "nav on all pages",
                        10,
                        all_lower.iter().all(|b| b.contains("<nav")),
                    ),
                    check(
                        "viewport meta on all pages",
                        10,
                        all_lower.iter().all(|b| b.contains("viewport")),
                    ),
                    check("substantial content (≥3KB html)", 15, total_html >= 3_000),
                ];
                let cross_linked = all_lower.iter().all(|b| {
                    pages
                        .iter()
                        .filter(|p| b.contains(&p.to_lowercase()))
                        .count()
                        >= 2
                });
                checks.push(check("pages cross-link each other", 10, cross_linked));
                let no_lorem = all_lower.iter().all(|b| !b.contains("lorem ipsum"));
                checks.push(check("no lorem ipsum", 5, no_lorem));
                checks
            },
        },
    ]
}

struct ScenarioResult {
    id: &'static str,
    score: u32,
    max_score: u32,
    checks: Vec<Check>,
    status: String,
    goal_completed: bool,
    goal_turns: u32,
    tool_calls: u32,
    tokens_in: u64,
    tokens_out: u64,
    cost_usd: f64,
    wall: Duration,
    timed_out: bool,
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git {args:?} failed");
}

fn sandbox_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "eval@clark.test"]);
    git(dir.path(), &["config", "user.name", "goal eval"]);
    std::fs::write(dir.path().join("README.md"), "# goal-eval sandbox\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "seed"]);
    dir
}

async fn run_scenario(
    scenario: &Scenario,
    base_url: &str,
    model: &str,
    api_key: &str,
) -> (ScenarioResult, PathBuf) {
    let repo = sandbox_repo();
    let started = Instant::now();

    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(api_key.to_string()),
            extra: json!({
                "base_url": base_url,
                "model": model,
                "memories": false,
            }),
            ..Default::default()
        })
        .await
        .expect("connect");
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(repo.path().to_string_lossy().to_string()),
            mode: None,
            resume: None,
        })
        .await
        .expect("session");

    let mut stream = provider
        .prompt(&session.id, PromptInput::text(scenario.prompt.clone()))
        .await
        .expect("prompt");

    let mut goal_turns = 0u32;
    let mut tool_calls = 0u32;
    let mut goal_completed = false;
    let mut run_id = None;
    let mut status = "unfinished".to_string();
    let mut usage = None;
    let mut timed_out = false;

    loop {
        let remaining = SCENARIO_TIMEOUT.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            timed_out = true;
        }
        let next = if timed_out {
            None
        } else {
            tokio::time::timeout(remaining, stream.next())
                .await
                .unwrap_or_else(|_| {
                    timed_out = true;
                    None
                })
        };
        let Some(ev) = next else {
            if timed_out {
                eprintln!(
                    "[{}] TIMEOUT after {:?} — cancelling",
                    scenario.id,
                    started.elapsed()
                );
                if let Some(run) = &run_id {
                    let _ = provider.cancel(&session.id, run).await;
                }
                // Give the cancel a moment to settle, then stop consuming.
                tokio::time::sleep(Duration::from_secs(3)).await;
                status = "timeout".into();
            }
            break;
        };
        match &ev {
            AgentEvent::RunStarted { run } => run_id = Some(run.clone()),
            AgentEvent::ToolCall { call, .. } => {
                tool_calls += 1;
                eprintln!("[{}] tool: {}", scenario.id, call.title);
                if call.tool_name.as_deref() == Some("update_goal") {
                    let complete = call
                        .raw_input
                        .as_ref()
                        .and_then(|v| v.get("status"))
                        .and_then(|v| v.as_str())
                        == Some("complete");
                    if complete {
                        goal_completed = true;
                    }
                }
            }
            AgentEvent::MessageChunk {
                role: Role::System, ..
            } => {
                goal_turns += 1;
                eprintln!("[{}] goal continuation #{goal_turns}", scenario.id);
            }
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
            AgentEvent::Error { code, message, .. } => {
                eprintln!("[{}] error: {code}: {message}", scenario.id);
            }
            AgentEvent::RunFinished { outcome, .. } => {
                status = format!("{:?}", outcome.status).to_lowercase();
                if outcome.status != RunStatus::Done {
                    if let Some(err) = &outcome.error {
                        eprintln!("[{}] run ended: {err}", scenario.id);
                    }
                }
                usage = outcome.usage;
                break;
            }
            _ => {}
        }
    }

    let checks = (scenario.grade)(repo.path());
    let score: u32 = checks.iter().filter(|c| c.passed).map(|c| c.weight).sum();
    let max_score: u32 = checks.iter().map(|c| c.weight).sum();
    let result = ScenarioResult {
        id: scenario.id,
        score,
        max_score,
        checks,
        status,
        goal_completed,
        goal_turns,
        tool_calls,
        tokens_in: usage.map(|u| u.input_tokens).unwrap_or(0),
        tokens_out: usage.map(|u| u.output_tokens).unwrap_or(0),
        cost_usd: usage.and_then(|u| u.cost_usd).unwrap_or(0.0),
        wall: started.elapsed(),
        timed_out,
    };
    // Keep the artifacts around for inspection.
    let keep = repo.keep();
    (result, keep)
}

#[tokio::main]
async fn main() {
    if std::env::var("CLARK_CODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("goal_eval is live-only: set CLARK_CODE_LIVE=1 and CLARK_CODE_API_KEY");
        return;
    }
    let api_key = match std::env::var("CLARK_CODE_API_KEY") {
        Ok(key) if !key.trim().is_empty() => key,
        _ => {
            eprintln!("set CLARK_CODE_API_KEY");
            return;
        }
    };
    let base_url = std::env::var("CLARK_CODE_BASE_URL")
        .unwrap_or_else(|_| "https://api.clarkslabs.com/v1".to_string());
    let model = std::env::var("CLARK_CODE_MODEL").unwrap_or_else(|_| "clark-code".to_string());
    let filter = std::env::var("GOAL_EVAL_SCENARIOS").ok();

    let selected: Vec<Scenario> = scenarios()
        .into_iter()
        .filter(|s| {
            filter
                .as_deref()
                .map(|f| f.split(',').any(|id| id.trim() == s.id))
                .unwrap_or(true)
        })
        .collect();

    eprintln!(
        "goal_eval: model={model}, scenarios={:?}",
        selected.iter().map(|s| s.id).collect::<Vec<_>>()
    );

    let mut results = Vec::new();
    for scenario in &selected {
        eprintln!("\n=== scenario: {} ===", scenario.id);
        let (result, artifacts) = run_scenario(scenario, &base_url, &model, &api_key).await;
        eprintln!(
            "[{}] artifacts kept at {}",
            scenario.id,
            artifacts.display()
        );
        results.push((result, artifacts));
    }

    println!("\n# goal_eval results — model: {model}\n");
    println!("| scenario | score | run | goal complete | goal turns | tools | tokens in/out | cost | time |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for (r, _) in &results {
        println!(
            "| {} | **{}/{}** | {}{} | {} | {} | {} | {}/{} | ${:.2} | {}s |",
            r.id,
            r.score,
            r.max_score,
            r.status,
            if r.timed_out { " (timeout)" } else { "" },
            if r.goal_completed { "yes" } else { "no" },
            r.goal_turns,
            r.tool_calls,
            r.tokens_in,
            r.tokens_out,
            r.cost_usd,
            r.wall.as_secs(),
        );
    }
    for (r, artifacts) in &results {
        println!(
            "\n## {} — {}/{} ({})",
            r.id,
            r.score,
            r.max_score,
            artifacts.display()
        );
        for c in &r.checks {
            println!(
                "- [{}] {} ({} pts)",
                if c.passed { "x" } else { " " },
                c.label,
                c.weight
            );
        }
    }
}
