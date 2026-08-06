use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::{stream, StreamExt};

mod model;
mod scenario;
mod turn;

use model::{sanitized_base_url, summarize, Receipt};
use scenario::run_trajectory;

const MODEL: &str = "clark-code:free";
const DEFAULT_BASE_URL: &str = "https://api.clarkslabs.com/v1";

#[derive(Clone)]
struct Args {
    repetitions: usize,
    concurrency: usize,
    max_provider_cost_usd: f64,
    output: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut repetitions = 12;
    let mut concurrency = 4;
    let mut max_provider_cost_usd: f64 = 1.0;
    let mut output = None;
    let mut values = std::env::args().skip(1);
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--repetitions" => {
                repetitions = next_value(&mut values, "--repetitions")?
                    .parse()
                    .map_err(|_| "--repetitions must be a positive integer".to_string())?;
            }
            "--concurrency" => {
                concurrency = next_value(&mut values, "--concurrency")?
                    .parse()
                    .map_err(|_| "--concurrency must be a positive integer".to_string())?;
            }
            "--max-provider-cost-usd" => {
                max_provider_cost_usd = next_value(&mut values, "--max-provider-cost-usd")?
                    .parse()
                    .map_err(|_| "--max-provider-cost-usd must be positive".to_string())?;
            }
            "--out" => output = Some(PathBuf::from(next_value(&mut values, "--out")?)),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if repetitions == 0 || concurrency == 0 {
        return Err("--repetitions and --concurrency must be positive".to_string());
    }
    if !max_provider_cost_usd.is_finite() || max_provider_cost_usd <= 0.0 {
        return Err("--max-provider-cost-usd must be positive".to_string());
    }
    let output = output.ok_or_else(|| "--out is required".to_string())?;
    Ok(Args {
        repetitions,
        concurrency,
        max_provider_cost_usd,
        output,
    })
}

fn next_value(values: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    values
        .next()
        .ok_or_else(|| format!("{name} requires a value"))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn source_state() -> (Option<String>, bool) {
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let commit = run(&["rev-parse", "HEAD"]).filter(|value| !value.is_empty());
    let dirty = run(&["status", "--porcelain"]).is_none_or(|value| !value.is_empty());
    (commit, dirty)
}

fn write_receipt(path: &Path, receipt: &Receipt) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| format!("write {}: {error}", path.display()))
}

#[tokio::main]
async fn main() {
    let args = parse_args().unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        std::env::var("CLARK_FREE_STRESS_LIVE").ok().as_deref(),
        Some("1"),
        "set CLARK_FREE_STRESS_LIVE=1 to authorize this exact live Free-route run"
    );
    let api_key = std::env::var("CLARK_CODE_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .expect("CLARK_CODE_API_KEY is required");
    let base_url =
        std::env::var("CLARK_CODE_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let receipt_base_url = sanitized_base_url(&base_url);
    assert!(
        !args.output.exists(),
        "output already exists: {}",
        args.output.display()
    );
    std::fs::create_dir_all(args.output.join("workspaces")).expect("create output directory");
    let (source_commit, source_dirty) = source_state();
    let started_at_ms = now_ms();
    let mut trajectories = Vec::new();

    for batch_start in (0..args.repetitions).step_by(args.concurrency) {
        let batch_end = (batch_start + args.concurrency).min(args.repetitions);
        let batch = stream::iter(batch_start..batch_end)
            .map(|repetition| {
                run_trajectory(
                    repetition,
                    args.output.clone(),
                    base_url.clone(),
                    api_key.clone(),
                )
            })
            .buffer_unordered(args.concurrency)
            .collect::<Vec<_>>()
            .await;
        trajectories.extend(batch);
        trajectories.sort_by_key(|trajectory| trajectory.repetition);
        let summary = summarize(&trajectories);
        let partial = Receipt {
            schema_version: 1,
            evidence_class: "live_provider_host",
            requested_model: MODEL,
            base_url: receipt_base_url.clone(),
            started_at_ms,
            finished_at_ms: now_ms(),
            source_commit: source_commit.clone(),
            source_dirty,
            repetitions_requested: args.repetitions,
            repetitions_completed: trajectories.len(),
            concurrency: args.concurrency,
            max_provider_cost_usd: args.max_provider_cost_usd,
            trajectories,
            summary: summary.clone(),
        };
        write_receipt(&args.output.join("receipt.json"), &partial).expect("write receipt");
        trajectories = partial.trajectories;
        eprintln!(
            "free-tier-stress: {}/{} trajectories, {}/{} cases passed, provider_cost=${:.6}",
            trajectories.len(),
            args.repetitions,
            summary.passed,
            summary.cases,
            summary.provider_cost_usd
        );
        if summary.provider_cost_usd >= args.max_provider_cost_usd {
            eprintln!("provider cost ceiling reached; refusing another batch");
            break;
        }
    }

    let summary = summarize(&trajectories);
    let receipt = Receipt {
        schema_version: 1,
        evidence_class: "live_provider_host",
        requested_model: MODEL,
        base_url: receipt_base_url,
        started_at_ms,
        finished_at_ms: now_ms(),
        source_commit,
        source_dirty,
        repetitions_requested: args.repetitions,
        repetitions_completed: trajectories.len(),
        concurrency: args.concurrency,
        max_provider_cost_usd: args.max_provider_cost_usd,
        trajectories,
        summary: summary.clone(),
    };
    let receipt_path = args.output.join("receipt.json");
    write_receipt(&receipt_path, &receipt).expect("write final receipt");
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).expect("serialize summary")
    );
    println!("RECEIPT={}", receipt_path.display());
    if !summary.gate_passed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
