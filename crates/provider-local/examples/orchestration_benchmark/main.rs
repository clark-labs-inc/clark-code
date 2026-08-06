//! Reproducible benchmark for Clark's proposed multi-agent coding coordinator.
//!
//! Offline scripted runs are the default and validate benchmark mechanics,
//! safety gates, failure recovery, scenario solvability, and report generation.
//! They are not model-quality evidence. Paid live lanes require both
//! `ORCHESTRATION_BENCH_LIVE=1` and an API key; live execution is implemented
//! separately so an ordinary test command can never spend credits.

mod control;
mod coordinator;
mod lifecycle;
mod live_runner;
mod model;
mod report;
mod scenarios;
mod scripted_provider;

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use coordinator::{run_scripted, ScriptedRunOptions};
use live_runner::{run_live, LiveRunOptions};
use model::{BenchmarkRecord, LaneSpec};
use uuid::Uuid;

struct Args {
    out: PathBuf,
    repetitions: u32,
    scenario: Option<String>,
    lane: Option<String>,
    strong_model: String,
    cheap_model: String,
    acp_model: String,
    acp_command: Option<Vec<String>>,
    list: bool,
    live: bool,
    full_live_matrix: bool,
    attempt_timeout_secs: u64,
    live_token_budget: u64,
    max_live_runs: usize,
    max_live_cost_usd: f64,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Self {
            out: PathBuf::from("target/orchestration-benchmark")
                .join(&Uuid::new_v4().to_string()[..8]),
            repetitions: 1,
            scenario: None,
            lane: None,
            strong_model: "scripted-strong".into(),
            cheap_model: "scripted-cheap".into(),
            acp_model: "external-acp".into(),
            acp_command: None,
            list: false,
            live: false,
            full_live_matrix: false,
            attempt_timeout_secs: 600,
            live_token_budget: 400_000,
            max_live_runs: 12,
            max_live_cost_usd: 2.0,
        };
        let mut input = std::env::args().skip(1);
        while let Some(arg) = input.next() {
            match arg.as_str() {
                "--out" => args.out = PathBuf::from(input.next().ok_or("--out requires a path")?),
                "--repetitions" => {
                    args.repetitions = input
                        .next()
                        .ok_or("--repetitions requires a number")?
                        .parse()
                        .map_err(|_| "--repetitions must be a positive integer")?;
                    if args.repetitions == 0 {
                        return Err("--repetitions must be positive".into());
                    }
                }
                "--scenario" => args.scenario = input.next(),
                "--lane" => args.lane = input.next(),
                "--strong-model" => {
                    args.strong_model = input.next().ok_or("--strong-model requires a value")?
                }
                "--cheap-model" => {
                    args.cheap_model = input.next().ok_or("--cheap-model requires a value")?
                }
                "--acp-model" => {
                    args.acp_model = input.next().ok_or("--acp-model requires a value")?
                }
                "--acp-command-json" => {
                    let raw = input
                        .next()
                        .ok_or("--acp-command-json requires a JSON array")?;
                    let command: Vec<String> = serde_json::from_str(&raw)
                        .map_err(|error| format!("--acp-command-json: {error}"))?;
                    if command.is_empty() || command.iter().any(|part| part.trim().is_empty()) {
                        return Err("--acp-command-json requires non-empty command parts".into());
                    }
                    args.acp_command = Some(command);
                }
                "--list" => args.list = true,
                "--live" => args.live = true,
                "--full-live-matrix" => args.full_live_matrix = true,
                "--attempt-timeout-secs" => {
                    args.attempt_timeout_secs = input
                        .next()
                        .ok_or("--attempt-timeout-secs requires a number")?
                        .parse()
                        .map_err(|_| "--attempt-timeout-secs must be a positive integer")?;
                    if args.attempt_timeout_secs == 0 {
                        return Err("--attempt-timeout-secs must be positive".into());
                    }
                }
                "--live-token-budget" => {
                    args.live_token_budget = input
                        .next()
                        .ok_or("--live-token-budget requires a number")?
                        .parse()
                        .map_err(|_| "--live-token-budget must be a positive integer")?;
                    if args.live_token_budget == 0 {
                        return Err("--live-token-budget must be positive".into());
                    }
                }
                "--max-live-runs" => {
                    args.max_live_runs = input
                        .next()
                        .ok_or("--max-live-runs requires a number")?
                        .parse()
                        .map_err(|_| "--max-live-runs must be a positive integer")?;
                    if args.max_live_runs == 0 {
                        return Err("--max-live-runs must be positive".into());
                    }
                }
                "--max-live-cost-usd" => {
                    args.max_live_cost_usd = input
                        .next()
                        .ok_or("--max-live-cost-usd requires a number")?
                        .parse()
                        .map_err(|_| "--max-live-cost-usd must be a positive number")?;
                    if !args.max_live_cost_usd.is_finite() || args.max_live_cost_usd <= 0.0 {
                        return Err("--max-live-cost-usd must be positive and finite".into());
                    }
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        Ok(args)
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("orchestration benchmark failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    load_env_file(PathBuf::from(".env"))?;
    let mut args = Args::parse()?;
    if args.live {
        if args.strong_model == "scripted-strong" {
            args.strong_model = "clark-code".into();
        }
        if args.cheap_model == "scripted-cheap" {
            args.cheap_model = provider_local::DEFAULT_MODEL.into();
        }
    }
    let scenarios = scenarios::catalog();
    let mut lanes = LaneSpec::catalog(&args.strong_model, &args.cheap_model);
    if let Some(mixed) = lanes.iter_mut().find(|lane| lane.id == "mixed-harness") {
        mixed.subagent_model = Some(args.acp_model.clone());
    }
    if args.live {
        for lane in &mut lanes {
            lane.token_budget = args.live_token_budget;
        }
    }
    if args.list {
        println!("Scenarios:");
        for scenario in &scenarios {
            println!(
                "  {:<34} family={:<24} delegate={} cloud={}",
                scenario.id,
                scenario.family,
                scenario.expected_delegate,
                scenario.cloud_agent_eligible
            );
        }
        println!("\nLanes:");
        for lane in &lanes {
            println!(
                "  {:<20} kind={:?} root={} subagent={}",
                lane.id,
                lane.kind,
                lane.root_model,
                lane.subagent_model.as_deref().unwrap_or("-")
            );
        }
        return Ok(());
    }
    if args.live {
        live_preflight()?;
        if args.scenario.is_none() && args.lane.is_none() && !args.full_live_matrix {
            return Err(
                "live mode requires --scenario and/or --lane; pass --full-live-matrix to authorize the complete paid cross-product"
                    .into(),
            );
        }
    }

    let selected_scenarios: Vec<_> = scenarios
        .iter()
        .filter(|scenario| {
            args.scenario
                .as_deref()
                .is_none_or(|filter| scenario.id == filter || scenario.family == filter)
        })
        .collect();
    if selected_scenarios.is_empty() {
        return Err(format!("no scenarios matched {:?}", args.scenario));
    }
    let selected_lanes: Vec<_> = lanes
        .iter()
        .filter(|lane| {
            args.lane
                .as_deref()
                .is_none_or(|filter| filter.split(',').any(|id| lane.id == id.trim()))
        })
        .collect();
    if selected_lanes.is_empty() {
        return Err(format!("no lanes matched {:?}", args.lane));
    }
    if args.live
        && selected_lanes.iter().any(|lane| lane.id == "mixed-harness")
        && args.acp_command.is_none()
    {
        return Err(
            "live mixed-harness requires --acp-command-json; Clark wraps it in a macOS read-only sandbox"
                .into(),
        );
    }

    std::fs::create_dir_all(&args.out).map_err(|error| error.to_string())?;
    let result_path = args.out.join("results.jsonl");
    let mut result_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&result_path)
        .map_err(|error| format!("open {}: {error}", result_path.display()))?;
    let mut records: Vec<BenchmarkRecord> = Vec::new();
    let mut live_cost_usd = 0.0;
    let mut live_runs = 0;
    'matrix: for repetition in 1..=args.repetitions {
        for scenario in &selected_scenarios {
            for lane in &selected_lanes {
                if args.live
                    && (live_runs >= args.max_live_runs || live_cost_usd >= args.max_live_cost_usd)
                {
                    eprintln!(
                        "[live] stopping at safety cap: runs={live_runs}/{} cost=${live_cost_usd:.4}/${:.4}",
                        args.max_live_runs, args.max_live_cost_usd
                    );
                    break 'matrix;
                }
                eprintln!(
                    "[{}] scenario={} lane={} repetition={repetition}",
                    if args.live { "live" } else { "scripted" },
                    scenario.id,
                    lane.id
                );
                let record = if args.live {
                    run_live(
                        scenario,
                        lane,
                        &LiveRunOptions {
                            artifact_root: args.out.join("runs"),
                            repetition,
                            api_key: live_api_key()?,
                            base_url: std::env::var("ORCHESTRATION_BENCH_BASE_URL")
                                .unwrap_or_else(|_| provider_local::DEFAULT_BASE_URL.into()),
                            research_model: std::env::var("ORCHESTRATION_BENCH_RESEARCH_MODEL")
                                .unwrap_or_else(|_| provider_local::DEFAULT_RESEARCH_MODEL.into()),
                            acp_command: args.acp_command.clone(),
                            attempt_timeout: Duration::from_secs(args.attempt_timeout_secs),
                        },
                    )
                    .await?
                } else {
                    run_scripted(
                        scenario,
                        lane,
                        &ScriptedRunOptions {
                            artifact_root: args.out.join("runs"),
                            repetition,
                        },
                    )
                    .await?
                };
                writeln!(
                    result_file,
                    "{}",
                    serde_json::to_string(&record).map_err(|error| error.to_string())?
                )
                .map_err(|error| error.to_string())?;
                records.push(record);
                if args.live {
                    let record = records.last().expect("record was just pushed");
                    live_cost_usd += record.metrics.cost_usd;
                    live_runs += 1;
                }
            }
        }
    }
    let summary = report::summarize(&records);
    std::fs::write(
        args.out.join("summary.json"),
        serde_json::to_vec_pretty(&summary).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    std::fs::write(args.out.join("report.md"), report::markdown(&summary))
        .map_err(|error| error.to_string())?;
    println!("{}", report::markdown(&summary));
    println!("\nArtifacts: {}", args.out.display());
    Ok(())
}

fn live_preflight() -> Result<(), String> {
    if std::env::var("ORCHESTRATION_BENCH_LIVE").ok().as_deref() != Some("1") {
        return Err("set ORCHESTRATION_BENCH_LIVE=1 to authorize paid model calls".into());
    }
    live_api_key()?;
    Ok(())
}

fn live_api_key() -> Result<String, String> {
    ["CLARK_CODE_API_KEY", "CLARK_API_KEY"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .ok_or_else(|| {
            "set CLARK_CODE_API_KEY or CLARK_API_KEY; keys are never written to results".into()
        })
}

fn load_env_file(path: PathBuf) -> Result<(), String> {
    if !path.is_file() {
        return Ok(());
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || std::env::var_os(name).is_some() {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(value);
        std::env::set_var(name, value);
    }
    Ok(())
}

fn print_help() {
    println!(
        "Clark orchestration benchmark\n\n\
         Usage: cargo run -p provider-local --example orchestration_benchmark -- [options]\n\n\
         --out PATH             Artifact directory (must not already contain results.jsonl)\n\
         --repetitions N        Repetitions per scenario/lane (default 1)\n\
         --scenario ID|FAMILY   Filter synthetic scenarios\n\
         --lane ID[,ID...]      Filter one or more A/B lanes\n\
         --strong-model ID      Strong-model label/config\n\
         --cheap-model ID       Cheap-subagent label/config\n\
         --acp-model ID         Model label recorded for the external ACP harness\n\
         --acp-command-json JSON  ACP command as a JSON string array (live mixed-harness only)\n\
         --attempt-timeout-secs Per-agent live timeout (default 600)\n\
         --live-token-budget N  Tree token budget per live run (default 400000)\n\
         --max-live-runs N      Paid run cap across the invocation (default 12)\n\
         --max-live-cost-usd N  Paid cost cap between runs (default 2.0)\n\
         --list                 List scenarios and lanes\n\
         --live                 Paid mode; loads .env and requires ORCHESTRATION_BENCH_LIVE=1\n\
         --full-live-matrix     Authorize all live scenarios/lanes when no filter is supplied"
    );
}
