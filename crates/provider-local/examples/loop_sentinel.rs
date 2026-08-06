//! Live qualification harness for a bounded, same-model loop sentinel.
//!
//! The sentinel is not an executor or an outcome-quality critic. It receives a
//! compact host-state packet, emits one forced typed decision, and disappears.

#[path = "loop_sentinel/model.rs"]
mod model;
#[path = "loop_sentinel/policy.rs"]
mod policy;
#[allow(dead_code)]
#[path = "planning_eval/retry.rs"]
mod retry;
#[allow(dead_code)]
#[path = "planning_eval/route.rs"]
mod route;
#[path = "loop_sentinel/turn.rs"]
mod turn;

#[cfg(test)]
#[path = "loop_sentinel/tests.rs"]
mod tests;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::{stream, StreamExt};
use model::{summarize, MatrixReceipt, TrialReceipt};
use policy::{host_disposition, scenarios};
use route::{verify_free_route, LiveConfig};
use turn::run_sentinel;

pub(crate) const MODEL: &str = "clark-code:free";
const DEFAULT_BASE_URL: &str = "https://api.clarkslabs.com/v1";

struct Args {
    output: PathBuf,
    repetitions: usize,
    concurrency: usize,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("loop_sentinel: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    if std::env::var("CLARK_LOOP_SENTINEL_LIVE").as_deref() != Ok("1") {
        return Err(
            "set CLARK_LOOP_SENTINEL_LIVE=1 to authorize this exact Free-route matrix".into(),
        );
    }
    if args.output.exists() {
        return Err(format!(
            "refusing to overwrite existing output {}",
            args.output.display()
        ));
    }
    let started_at_ms = now_ms();
    let mut config = LiveConfig {
        api_key: std::env::var("CLARK_CODE_API_KEY").map_err(|_| {
            "CLARK_CODE_API_KEY is required; .env fallback is intentionally disabled"
        })?,
        base_url: std::env::var("CLARK_CODE_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string()),
        model: std::env::var("CLARK_CODE_MODEL").unwrap_or_else(|_| MODEL.to_string()),
        profile: "concise".into(),
        reasoning_effort: "max".into(),
    };
    if config.model != MODEL {
        return Err(format!("CLARK_CODE_MODEL must be exactly {MODEL}"));
    }
    let route = verify_free_route(&mut config).await?;
    let config = Arc::new(config);
    let scenario_set = scenarios();

    for scenario in &scenario_set {
        let observed = host_disposition(&scenario.packet);
        if observed != scenario.expected_host_disposition {
            return Err(format!(
                "scenario {} expected host disposition {:?}, observed {:?}",
                scenario.id, scenario.expected_host_disposition, observed
            ));
        }
    }

    let mut trials = scenario_set
        .iter()
        .filter(|scenario| scenario.invocation.calls_model())
        .flat_map(|scenario| {
            (1..=args.repetitions).map(move |repetition| (scenario.clone(), repetition))
        })
        .collect::<Vec<_>>();
    let total_calls = trials.len();
    let expected_model = route.effective_model.clone();
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let live_trials = stream::iter(trials.drain(..))
        .map(|(scenario, repetition)| {
            let config = config.clone();
            let expected_model = expected_model.clone();
            let completed = &completed;
            async move {
                let call = run_sentinel(&config, &scenario.packet, &expected_model).await;
                let trial = TrialReceipt::from_call(scenario, repetition, call);
                let count = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                eprintln!(
                    "loop sentinel: completed {count}/{total_calls} ({} r{}, {})",
                    trial.scenario,
                    repetition,
                    if trial.passed { "pass" } else { "fail" }
                );
                trial
            }
        })
        .buffer_unordered(args.concurrency)
        .collect::<Vec<_>>()
        .await;

    let mut all_trials = live_trials;
    for scenario in scenario_set
        .into_iter()
        .filter(|scenario| !scenario.invocation.calls_model())
    {
        for repetition in 1..=args.repetitions {
            all_trials.push(TrialReceipt::host_only(scenario.clone(), repetition));
        }
    }
    all_trials.sort_by(|left, right| {
        left.repetition
            .cmp(&right.repetition)
            .then_with(|| left.scenario.cmp(right.scenario))
    });
    let summary = summarize(&all_trials);
    let (source_commit, source_dirty) = source_state();
    let receipt = MatrixReceipt {
        schema_version: 1,
        evidence_class: "live_same_model_loop_sentinel",
        design: "deterministic hard stops and stop validator; occasional one-shot sentinel; productive shadow controls; no executor or outcome-quality task",
        requested_model: MODEL,
        reasoning_effort: "max",
        route,
        repetitions_requested: args.repetitions,
        concurrency: args.concurrency,
        source_commit,
        source_dirty,
        trials: all_trials,
        summary,
        started_at_ms,
        finished_at_ms: now_ms(),
    };
    std::fs::create_dir_all(&args.output).map_err(|error| error.to_string())?;
    let receipt_path = args.output.join("receipt.json");
    std::fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt.summary).map_err(|error| error.to_string())?
    );
    println!("RECEIPT={}", receipt_path.display());
    if !receipt.summary.gate_passed {
        return Err("loop-sentinel qualification gate failed".into());
    }
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut output = None;
    let mut repetitions = 1usize;
    let mut concurrency = 2usize;
    let mut values = std::env::args().skip(1);
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--out" => output = Some(PathBuf::from(required(&mut values, "--out")?)),
            "--repetitions" => repetitions = positive(&required(&mut values, "--repetitions")?)?,
            "--concurrency" => concurrency = positive(&required(&mut values, "--concurrency")?)?,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Args {
        output: output.ok_or("--out is required")?,
        repetitions,
        concurrency,
    })
}

fn required(values: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    values
        .next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn positive(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("expected a positive integer, got {value:?}"))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn source_state() -> (Option<String>, bool) {
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map(|output| !output.stdout.is_empty())
        .unwrap_or(true);
    (commit, dirty)
}
