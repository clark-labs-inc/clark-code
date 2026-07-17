//! Memory-lifecycle evaluation runner.
//!
//! Runs the scenario catalog against a live clark-code model and grades
//! memory behavior across six dimensions (stale, correction, hallucination,
//! proactivity, recall, churn). Results stream to a JSONL file; a summary
//! table prints at the end.
//!
//! ```sh
//! CLARK_CODE_BASE_URL=https://api.clarkslabs.com/v1 CLARK_CODE_API_KEY=ck_... \
//!   cargo run -p provider-local --example memory_eval -- \
//!     --model clark-code --out results.jsonl --concurrency 8 [--filter stale] [--limit 12]
//! ```
//!
//! The process's HOME is redirected to a scratch dir before any session is
//! created, so agent writes to the "global" memory scope never touch the real
//! `~/.clark`.

mod grading;
mod scenarios;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_core::domain::{AgentEvent, ContentBlock, RunStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use grading::{Check, CheckResult, JudgeClient, RunRecord};
use provider_local::LocalAgentProvider;
use scenarios::{CommitStep, Scenario, SeedMemory};
use serde::Serialize;

const TURN_TIMEOUT: Duration = Duration::from_secs(300);

struct Args {
    model: String,
    out: PathBuf,
    concurrency: usize,
    filter: Option<String>,
    limit: Option<usize>,
}

fn parse_args() -> Args {
    let mut args = Args {
        model: "clark-code".into(),
        out: PathBuf::from("memory-eval-results.jsonl"),
        concurrency: 6,
        filter: None,
        limit: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--model" => args.model = it.next().expect("--model value"),
            "--out" => args.out = PathBuf::from(it.next().expect("--out value")),
            "--concurrency" => {
                args.concurrency = it.next().expect("--concurrency value").parse().unwrap()
            }
            "--filter" => args.filter = it.next(),
            "--limit" => args.limit = it.next().map(|v| v.parse().unwrap()),
            other => panic!("unknown arg {other}"),
        }
    }
    args
}

#[derive(Serialize)]
struct ScenarioOutcome {
    id: String,
    dimension: &'static str,
    model: String,
    score: f64,
    checks: Vec<CheckResult>,
    cost_usd: f64,
    duration_s: f64,
    error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    replies: Vec<String>,
}

fn git(repo: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn build_repo(repo: &Path, scenario: &Scenario) {
    for (path, content) in &scenario.initial_files {
        let p = repo.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    git(repo, &["init", "--quiet"]);
    git(repo, &["config", "user.name", "Eval"]);
    git(repo, &["config", "user.email", "eval@example.com"]);
    git(repo, &["add", "."]);
    git(repo, &["commit", "--quiet", "-m", "initial"]);
    for step in &scenario.commits {
        apply_commit(repo, step);
    }
}

fn apply_commit(repo: &Path, step: &CommitStep) {
    for (from, to) in &step.renames {
        let dest = repo.join(to);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        git(repo, &["mv", from, to]);
    }
    for path in &step.deletes {
        git(repo, &["rm", "--quiet", path]);
    }
    for (path, content) in &step.writes {
        let p = repo.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
        git(repo, &["add", path]);
    }
    git(repo, &["commit", "--quiet", "-am", &step.message]);
}

/// Write seeded notes in the same on-disk format `save_memory` produces.
fn seed_memories(repo: &Path, memories: &[SeedMemory]) {
    if memories.is_empty() {
        return;
    }
    let dir = repo.join(".clark/memory");
    std::fs::create_dir_all(&dir).unwrap();
    let mut index = String::from("# Memory index\n");
    for m in memories {
        let slug: String = m
            .title
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let desc = m.body.lines().next().unwrap_or(&m.title);
        std::fs::write(
            dir.join(format!("{slug}.md")),
            format!(
                "---\nname: {}\ndescription: {}\ntype: {}\n---\n\n{}\n",
                m.title, desc, m.mtype, m.body
            ),
        )
        .unwrap();
        index.push_str(&format!("- [{}]({slug}.md) — {desc}\n", m.title));
    }
    std::fs::write(dir.join("MEMORY.md"), index).unwrap();
}

fn store_text(repo: &Path) -> String {
    let dir = repo.join(".clark/memory");
    let mut out = String::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_file() {
                out.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
                out.push('\n');
            }
        }
    }
    out
}

async fn run_scenario(
    scenario: &Scenario,
    model: &str,
    base_url: &str,
    api_key: &str,
    judge: &JudgeClient,
) -> ScenarioOutcome {
    let started = std::time::Instant::now();
    let dir = tempfile::tempdir().expect("tempdir");
    build_repo(dir.path(), scenario);
    seed_memories(dir.path(), &scenario.memories);

    let mut record = RunRecord {
        store_before: store_text(dir.path()),
        ..Default::default()
    };
    let mut error = None;

    let mut provider = LocalAgentProvider::new();
    let connected = provider
        .connect(ProviderConfig {
            auth_token: Some(api_key.to_string()),
            extra: serde_json::json!({
                "base_url": base_url,
                "model": model,
                "cwd": dir.path().to_string_lossy(),
                "research": false,
                "memories": true,
                "permissions": {"bash": "ask", "write_file": "allow", "edit_file": "allow"}
            }),
            ..Default::default()
        })
        .await;
    if let Err(e) = connected {
        return outcome(scenario, model, &record, dir.path(), judge, Some(e.to_string()), started)
            .await;
    }
    let session = match provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            resume: None,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return outcome(
                scenario,
                model,
                &record,
                dir.path(),
                judge,
                Some(e.to_string()),
                started,
            )
            .await
        }
    };

    'turns: for turn in &scenario.turns {
        let mut stream = match provider
            .prompt(&session.id, PromptInput::text(turn))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                error = Some(e.to_string());
                break 'turns;
            }
        };
        let mut reply = String::new();
        let drive = async {
            while let Some(ev) = stream.next().await {
                match ev {
                    AgentEvent::MessageChunk {
                        delta: ContentBlock::Text { text },
                        ..
                    } => reply.push_str(&text),
                    AgentEvent::ToolCall { call, .. } => {
                        let name = call
                            .title
                            .split(':')
                            .next()
                            .unwrap_or(&call.title)
                            .trim()
                            .to_string();
                        let args = call
                            .raw_input
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| call.title.clone());
                        record.tool_calls.push((name, args));
                    }
                    AgentEvent::PermissionRequest { request } => {
                        let _ = provider
                            .respond(
                                &session.id,
                                ClientResponse::Permission {
                                    request: request.id,
                                    option: "allow_once".into(),
                                    feedback: None,
                                },
                            )
                            .await;
                    }
                    AgentEvent::RunFinished { outcome, .. } => {
                        if let Some(u) = outcome.usage {
                            record.cost_usd += u.cost_usd.unwrap_or(0.0);
                        }
                        return match outcome.status {
                            RunStatus::Done => Ok(()),
                            other => Err(format!("run ended {other:?}: {:?}", outcome.error)),
                        };
                    }
                    _ => {}
                }
            }
            Err("stream ended without RunFinished".to_string())
        };
        let result = tokio::time::timeout(TURN_TIMEOUT, drive).await;
        record.replies.push(reply);
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error = Some(e);
                break 'turns;
            }
            Err(_) => {
                error = Some("turn timed out".into());
                break 'turns;
            }
        }
    }

    // Post-turn memory extraction (when present in the build under test) runs
    // detached after RunFinished; give it time to land before grading store
    // state on the dimensions that measure writes.
    if matches!(
        scenario.dimension,
        scenarios::Dimension::Proactivity
            | scenarios::Dimension::Correction
            | scenarios::Dimension::Hallucination
    ) {
        tokio::time::sleep(Duration::from_secs(15)).await;
    }

    outcome(scenario, model, &record, dir.path(), judge, error, started).await
}

#[allow(clippy::too_many_arguments)]
async fn outcome(
    scenario: &Scenario,
    model: &str,
    record: &RunRecord,
    repo: &Path,
    judge: &JudgeClient,
    error: Option<String>,
    started: std::time::Instant,
) -> ScenarioOutcome {
    let mut record_final = RunRecord {
        tool_calls: record.tool_calls.clone(),
        replies: record.replies.clone(),
        store_before: record.store_before.clone(),
        store_after: store_text(repo),
        cost_usd: record.cost_usd,
    };
    // An errored run grades what it has; checks over missing state just fail.
    let checks: &[Check] = &scenario.checks;
    let results = grading::grade(checks, &record_final, repo, judge).await;
    let score = if results.is_empty() {
        0.0
    } else {
        results.iter().filter(|r| r.pass).count() as f64 / results.len() as f64
    };
    if std::env::var("MEMORY_EVAL_KEEP_REPLIES").is_err() {
        record_final.replies.clear();
    }
    ScenarioOutcome {
        id: scenario.id.clone(),
        dimension: scenario.dimension.label(),
        model: model.to_string(),
        score,
        checks: results,
        cost_usd: record_final.cost_usd,
        duration_s: started.elapsed().as_secs_f64(),
        error,
        replies: record_final.replies,
    }
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let base_url =
        std::env::var("CLARK_CODE_BASE_URL").expect("set CLARK_CODE_BASE_URL");
    let api_key = std::env::var("CLARK_CODE_API_KEY").expect("set CLARK_CODE_API_KEY");

    // Sandbox the global memory scope for the whole process, before any
    // session exists. Scenario grading reads the project store + remember
    // args, so cross-scenario mixing in this shared dir is harmless.
    let eval_home = tempfile::tempdir().expect("eval home");
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", eval_home.path());
    }

    let mut catalog = scenarios::all();
    if let Some(filter) = &args.filter {
        catalog.retain(|s| s.id.starts_with(filter.as_str()) || s.dimension.label() == filter);
    }
    if let Some(limit) = args.limit {
        catalog.truncate(limit);
    }
    eprintln!(
        "memory-eval: {} scenarios, model={}, concurrency={}",
        catalog.len(),
        args.model,
        args.concurrency
    );

    let judge = Arc::new(JudgeClient::new(&base_url, &api_key));
    let model = args.model.clone();
    let base = base_url.clone();
    let key = api_key.clone();

    let outcomes: Vec<ScenarioOutcome> = futures::stream::iter(catalog.iter())
        .map(|scenario| {
            let judge = judge.clone();
            let (model, base, key) = (model.clone(), base.clone(), key.clone());
            async move {
                let o = run_scenario(scenario, &model, &base, &key, &judge).await;
                eprintln!(
                    "  [{}] {} score={:.2} cost=${:.4}{}",
                    o.dimension,
                    o.id,
                    o.score,
                    o.cost_usd,
                    o.error
                        .as_deref()
                        .map(|e| format!(" ERROR: {e}"))
                        .unwrap_or_default()
                );
                o
            }
        })
        .buffer_unordered(args.concurrency)
        .collect()
        .await;

    let mut file = std::fs::File::create(&args.out).expect("open out file");
    for o in &outcomes {
        writeln!(file, "{}", serde_json::to_string(o).unwrap()).unwrap();
    }

    // Summary: per-dimension mean score + check-level pass rate.
    let mut by_dim: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut total_cost = 0.0;
    let mut errors = 0;
    for o in &outcomes {
        by_dim.entry(o.dimension).or_default().push(o.score);
        total_cost += o.cost_usd;
        if o.error.is_some() {
            errors += 1;
        }
    }
    println!("\n== memory-eval summary (model={}) ==", args.model);
    let mut overall = Vec::new();
    for (dim, scores) in &by_dim {
        let mean = scores.iter().sum::<f64>() / scores.len() as f64;
        overall.extend_from_slice(scores);
        println!("  {dim:<16} {:>5.1}%  (n={})", mean * 100.0, scores.len());
    }
    let mean = overall.iter().sum::<f64>() / overall.len().max(1) as f64;
    println!(
        "  {:<16} {:>5.1}%  (n={}, errors={}, cost=${:.2})",
        "OVERALL",
        mean * 100.0,
        overall.len(),
        errors,
        total_cost
    );
}
