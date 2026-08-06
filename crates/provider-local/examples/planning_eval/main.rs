//! Executable Plan Mode and context-ablation benchmark.
//!
//! Offline mode validates fixtures, grading, receipts, and reporting without a
//! model. Live mode is fail-closed to the Clark Free product route resolving to
//! DeepSeek V4 Flash Latest with explicit zero-cost evidence.

mod context;
mod fixture_support;
mod fixtures;
mod gateway;
mod judge;
mod judge_verdict;
mod lifecycle;
mod model;
mod plan_bank;
mod retrieval;
mod retry;
mod route;
mod runner;
mod scenario_families;
mod scenario_families_extra_a;
mod scenario_families_extra_b;
mod stats;
mod typed_handoff;

use context::lanes;
use fixtures::scenarios;
use model::{CaseRecord, RouteReceipt, Summary};
use plan_bank::{BankSourceSet, PlanBank};
use route::{verify_free_route, LiveConfig};
use runner::{run_live_case, run_offline_case};
use std::collections::{BTreeMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Offline,
    Live,
}

struct Args {
    mode: Mode,
    scenarios: Vec<String>,
    lanes: Vec<String>,
    repetitions: usize,
    output: Option<PathBuf>,
    profile: String,
    max_live_cases: usize,
    judge_input: Option<PathBuf>,
    judgments: Option<PathBuf>,
}

struct OutputLock {
    path: PathBuf,
}

impl OutputLock {
    fn acquire(output: &Path) -> Result<Self, String> {
        let path = output.join(".planning-eval-active");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    format!(
                        "{} is already owned by another evaluator process; use a new output \
                         directory, or remove the lock only after proving no writer is alive",
                        output.display()
                    )
                } else {
                    format!("failed to lock {}: {error}", output.display())
                }
            })?;
        writeln!(file, "pid={}", std::process::id()).map_err(|error| error.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for OutputLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut mode = Mode::Offline;
        let mut selected_scenarios = Vec::new();
        let mut selected_lanes = Vec::new();
        let mut repetitions = 1;
        let mut output = None;
        let mut profile = "concise".to_string();
        let mut max_live_cases = 6;
        let mut judge_input = None;
        let mut judgments = None;
        let mut values = std::env::args().skip(1);
        while let Some(argument) = values.next() {
            match argument.as_str() {
                "--offline" => mode = Mode::Offline,
                "--live" => mode = Mode::Live,
                "--scenarios" => {
                    selected_scenarios = csv(&required_value(&mut values, "--scenarios")?)
                }
                "--lanes" => selected_lanes = csv(&required_value(&mut values, "--lanes")?),
                "--repetitions" => {
                    repetitions = positive_usize(
                        &required_value(&mut values, "--repetitions")?,
                        "--repetitions",
                    )?
                }
                "--output" => {
                    output = Some(PathBuf::from(required_value(&mut values, "--output")?))
                }
                "--profile" => profile = required_value(&mut values, "--profile")?,
                "--max-live-cases" => {
                    max_live_cases = positive_usize(
                        &required_value(&mut values, "--max-live-cases")?,
                        "--max-live-cases",
                    )?
                }
                "--judge-input" => {
                    judge_input = Some(PathBuf::from(required_value(&mut values, "--judge-input")?))
                }
                "--judgments" => {
                    judgments = Some(PathBuf::from(required_value(&mut values, "--judgments")?))
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                unknown => return Err(format!("unknown argument: {unknown}")),
            }
        }
        if !matches!(profile.as_str(), "legacy" | "decision_complete" | "concise") {
            return Err(format!("unsupported planning profile: {profile}"));
        }
        Ok(Self {
            mode,
            scenarios: selected_scenarios,
            lanes: selected_lanes,
            repetitions,
            output,
            profile,
            max_live_cases,
            judge_input,
            judgments,
        })
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("planning_eval: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args = Args::parse()?;
    if let Some(input) = args.judge_input.as_deref() {
        let output = args
            .output
            .clone()
            .unwrap_or_else(|| input.join("llm-judge-v1"));
        return judge::run(input, &output, args.judgments.as_deref());
    }
    if args.judgments.is_some() {
        return Err("--judgments requires --judge-input".into());
    }
    let mut run_id = format!(
        "planning-eval-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs()
    );
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/planning-eval").join(&run_id));
    std::fs::create_dir_all(&output).map_err(|error| error.to_string())?;
    let _output_lock = OutputLock::acquire(&output)?;
    let isolated_global_memory = output.join("isolated-global-memory");
    std::fs::create_dir_all(&isolated_global_memory).map_err(|error| error.to_string())?;
    let isolated_global_memory = isolated_global_memory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    std::env::set_var("CLARK_EVAL_GLOBAL_MEMORY_DIR", isolated_global_memory);
    let plan_bank_path = output.join("plan-bank.jsonl");
    let mut plan_bank = PlanBank::open(&plan_bank_path)?;
    for (source, destination) in [
        (
            "crates/provider-local/examples/planning_eval/PREREGISTRATION_V3.md",
            "PREREGISTRATION.md",
        ),
        (
            "crates/provider-local/examples/planning_eval/WEAK_POINTS_V3.md",
            "WEAK_POINTS.md",
        ),
        (
            "crates/provider-local/examples/planning_eval/COMPLETION_AUDIT_V3.md",
            "COMPLETION_AUDIT.md",
        ),
    ] {
        let body = std::fs::read_to_string(source)
            .map_err(|error| format!("failed to read {source}: {error}"))?;
        std::fs::write(output.join(destination), body).map_err(|error| error.to_string())?;
    }

    let all_scenarios = scenarios();
    let all_lanes = lanes();
    validate_filters(
        &args.scenarios,
        all_scenarios.iter().map(|scenario| scenario.id),
        "scenario",
    )?;
    validate_filters(
        &args.lanes,
        all_lanes.iter().map(|lane| lane.id.as_str()),
        "lane",
    )?;
    let selected_scenarios = all_scenarios
        .iter()
        .filter(|scenario| {
            args.scenarios.is_empty() || args.scenarios.contains(&scenario.id.into())
        })
        .collect::<Vec<_>>();
    let selected_lanes = all_lanes
        .iter()
        .filter(|lane| args.lanes.is_empty() || args.lanes.contains(&lane.id))
        .collect::<Vec<_>>();
    let requested_cases = selected_scenarios.len() * selected_lanes.len() * args.repetitions;
    let required_bank_treatments = selected_lanes
        .iter()
        .filter_map(|lane| match lane.plan_origin {
            model::PlanOrigin::BankNone => Some((BankSourceSet::None, lane.knowledge_delivery())),
            model::PlanOrigin::BankAll => Some((BankSourceSet::All, lane.knowledge_delivery())),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let requested_bank_entries =
        selected_scenarios.len() * required_bank_treatments.len() * args.repetitions;
    if args.mode == Mode::Offline {
        for repetition in 1..=args.repetitions {
            for scenario in &selected_scenarios {
                for (source_set, knowledge_delivery) in &required_bank_treatments {
                    plan_bank.ensure_offline_reference(
                        scenario,
                        repetition,
                        *source_set,
                        *knowledge_delivery,
                        &args.profile,
                    )?;
                }
            }
        }
    }

    let live = if args.mode == Mode::Live {
        if std::env::var("CLARK_CODE_LIVE").as_deref() != Ok("1") {
            return Err("set CLARK_CODE_LIVE=1 to authorize the free-route model calls".into());
        }
        let requested_workflows = requested_cases + requested_bank_entries;
        if requested_workflows > args.max_live_cases {
            return Err(format!(
                "requested {requested_cases} live cases plus {requested_bank_entries} frozen-plan generations ({requested_workflows} model workflows) exceeds --max-live-cases {}; narrow the lanes/scenarios or raise the explicit cap",
                args.max_live_cases
            ));
        }
        let mut config = LiveConfig {
            api_key: env_or_dotenv("CLARK_CODE_API_KEY")
                .ok_or("CLARK_CODE_API_KEY is required (environment or ignored .env)")?,
            base_url: env_or_dotenv("CLARK_CODE_BASE_URL")
                .unwrap_or_else(|| "https://api.clarkslabs.com/v1".into()),
            model: env_or_dotenv("CLARK_CODE_MODEL")
                .ok_or("CLARK_CODE_MODEL must be explicitly set to clark-code:free")?,
            profile: args.profile.clone(),
            reasoning_effort: std::env::var("PLANNING_EVAL_REASONING_EFFORT")
                .unwrap_or_else(|_| "low".into()),
        };
        eprintln!("verifying free product route before benchmark cases");
        let route = verify_free_route(&mut config).await?;
        eprintln!(
            "verified route {} -> {} with explicit free-tier evidence",
            route.product_route, route.effective_model
        );
        Some((config, route))
    } else {
        None
    };
    if let Some((config, route)) = &live {
        for repetition in 1..=args.repetitions {
            for scenario in &selected_scenarios {
                for (source_set, knowledge_delivery) in &required_bank_treatments {
                    match plan_bank.find(
                        scenario.id,
                        repetition,
                        *source_set,
                        *knowledge_delivery,
                        &args.profile,
                        &route.effective_model,
                    ) {
                        Ok(_) => continue,
                        Err(error) if error.starts_with("missing plan-bank entry") => {}
                        Err(error) => return Err(error),
                    }
                    eprintln!(
                        "generating frozen plan bank entry: {} {:?} {:?} repetition {}",
                        scenario.id, source_set, knowledge_delivery, repetition
                    );
                    let entry = PlanBank::generate_live_entry(
                        scenario,
                        repetition,
                        *source_set,
                        *knowledge_delivery,
                        route,
                        config,
                    )
                    .await?;
                    plan_bank.insert(entry)?;
                }
            }
        }
    }

    let jsonl_path = output.join("results.jsonl");
    let mut records = load_records(&jsonl_path)?;
    if let Some(existing) = records.first() {
        run_id = existing.run_id.clone();
        eprintln!(
            "resuming run {run_id} with {} completed cases",
            records.len()
        );
    }
    let mut completed = records
        .iter()
        .map(|record| {
            (
                record.scenario.clone(),
                record.lane.clone(),
                record.repetition,
            )
        })
        .collect::<HashSet<_>>();
    let mut jsonl = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&jsonl_path)
        .map_err(|error| error.to_string())?;
    for repetition in 1..=args.repetitions {
        for scenario in &selected_scenarios {
            let mut ordered_lanes = selected_lanes.clone();
            ordered_lanes.sort_by_key(|lane| {
                context::sha256(&format!(
                    "planning-eval-v3:{repetition}:{}:{}",
                    scenario.id, lane.id
                ))
            });
            for lane in ordered_lanes {
                let key = (scenario.id.to_string(), lane.id.clone(), repetition);
                if completed.contains(&key) {
                    continue;
                }
                eprintln!(
                    "[{}/{}] {} {} repetition {}",
                    records.len() + 1,
                    requested_cases,
                    scenario.id,
                    lane.id,
                    repetition
                );
                let bank_source = match lane.plan_origin {
                    model::PlanOrigin::BankNone => Some(BankSourceSet::None),
                    model::PlanOrigin::BankAll => Some(BankSourceSet::All),
                    _ => None,
                };
                let bank_entry = bank_source
                    .map(|source_set| {
                        let route_model = live
                            .as_ref()
                            .map(|(_, route)| route.effective_model.as_str())
                            .unwrap_or("deterministic-reference");
                        plan_bank.find(
                            scenario.id,
                            repetition,
                            source_set,
                            lane.knowledge_delivery(),
                            &args.profile,
                            route_model,
                        )
                    })
                    .transpose()?;
                let record = match &live {
                    Some((config, route)) => {
                        run_live_case(
                            &run_id, scenario, lane, repetition, route, config, bank_entry,
                        )
                        .await?
                    }
                    None => run_offline_case(
                        &run_id,
                        scenario,
                        lane,
                        repetition,
                        &args.profile,
                        bank_entry,
                    )?,
                };
                serde_json::to_writer(&mut jsonl, &record).map_err(|error| error.to_string())?;
                writeln!(jsonl).map_err(|error| error.to_string())?;
                jsonl.flush().map_err(|error| error.to_string())?;
                completed.insert(key);
                records.push(record);
            }
        }
    }
    if records.is_empty() {
        return Err("no benchmark cases selected".into());
    }
    let route = live
        .as_ref()
        .map(|(_, route)| route.clone())
        .unwrap_or_else(route::offline_route);
    let summary = summarize(
        &run_id,
        args.mode,
        &args.profile,
        args.repetitions,
        route,
        &records,
        &plan_bank,
    );
    write_json(&output.join("summary.json"), &summary)?;
    write_json(
        &output.join("lifecycle-findings.json"),
        &serde_json::json!({
            "schema_version": 1,
            "evidence": "deterministic provider boundary tests in lifecycle.rs",
            "findings": lifecycle::findings(),
        }),
    )?;
    write_json(
        &output.join("manifest.json"),
        &serde_json::json!({
            "schema_version": 5,
            "run_id": run_id,
            "scenarios": selected_scenarios.iter().map(|scenario| scenario.id).collect::<Vec<_>>(),
            "scenario_domains": selected_scenarios.iter().map(|scenario| (
                scenario.id,
                scenario.domain()
            )).collect::<BTreeMap<_, _>>(),
            "lanes": selected_lanes.iter().map(|lane| lane.id.as_str()).collect::<Vec<_>>(),
            "lane_treatments": selected_lanes.iter().map(|lane| (
                lane.id.as_str(),
                serde_json::json!({
                    "knowledge_delivery": lane.knowledge_delivery(),
                    "planner_sources": lane.planner_sources,
                    "executor_sources": lane.executor_sources,
                    "plan_origin": lane.plan_origin,
                    "handoff": lane.handoff,
                })
            )).collect::<BTreeMap<_, _>>(),
            "repetitions": args.repetitions,
            "ordering": "sha256(planning-eval-v3:repetition:scenario:lane)",
            "fixture_hashes": records.iter().map(|record| (
                record.scenario.clone(),
                record.fixture_sha256.clone()
            )).collect::<BTreeMap<_, _>>(),
            "planning_prompt_sha256": records.first().map(|record| record.planning_prompt_sha256.clone()),
            "route": &summary.route,
            "plan_bank": {
                "path": "plan-bank.jsonl",
                "entries": plan_bank.len()
            },
            "lifecycle_findings": {
                "path": "lifecycle-findings.json",
                "count": lifecycle::findings().len()
            },
        }),
    )?;
    std::fs::write(output.join("report.md"), report(&summary, &records))
        .map_err(|error| error.to_string())?;
    println!("{}", output.canonicalize().unwrap_or(output).display());
    Ok(())
}

fn load_records(path: &Path) -> Result<Vec<CaseRecord>, String> {
    let Some(body) = std::fs::read_to_string(path)
        .ok()
        .filter(|body| !body.trim().is_empty())
    else {
        return Ok(Vec::new());
    };
    body.lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line)
                .map_err(|error| format!("{} line {}: {error}", path.display(), index + 1))
        })
        .collect()
}

fn summarize(
    run_id: &str,
    mode: Mode,
    profile: &str,
    repetitions: usize,
    route: RouteReceipt,
    records: &[CaseRecord],
    plan_bank: &PlanBank,
) -> Summary {
    let comparisons = [
        ("no_plan", "planner_none"),
        ("no_plan", "plan_discarded"),
        ("planner_none", "planner_all"),
        ("context_executor_only", "context_both"),
        ("planner_all", "oracle_planner"),
        ("planner_all", "noisy_planner"),
        ("planner_all", "stale_planner"),
        ("planner_all", "conflict_planner"),
        ("oracle_discarded", "oracle_markdown_fresh"),
        ("oracle_markdown_fresh", "oracle_real_fresh"),
        ("real_plan_fresh", "real_plan_fresh_all"),
        ("bank_none_discarded", "bank_none_markdown"),
        ("bank_none_markdown", "bank_none_typed_replay"),
        ("bank_all_discarded", "bank_all_markdown"),
        ("bank_all_markdown", "bank_all_typed_replay"),
        ("bank_none_markdown", "bank_all_markdown"),
        ("bank_none_typed_replay", "bank_all_typed_replay"),
        (
            "bank_all_typed_replay",
            "bank_all_preactivated_typed_replay",
        ),
        ("bank_all_typed_replay", "bank_all_prefetched_typed_replay"),
        (
            "bank_all_preactivated_discarded",
            "bank_all_preactivated_markdown",
        ),
        (
            "bank_all_preactivated_markdown",
            "bank_all_preactivated_typed_replay",
        ),
        (
            "bank_all_prefetched_discarded",
            "bank_all_prefetched_markdown",
        ),
        (
            "bank_all_prefetched_markdown",
            "bank_all_prefetched_typed_replay",
        ),
    ];
    let paired_effects = comparisons
        .iter()
        .filter_map(|(control, candidate)| stats::paired_effect(records, control, candidate))
        .collect();
    let first_failures = records
        .iter()
        .filter_map(|record| {
            record.verification.first_failure().map(|failure| {
                (
                    format!("{}:{}:r{}", record.scenario, record.lane, record.repetition),
                    format!("{}: {}", failure.id, failure.detail),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let plan_bank_cost = plan_bank.total_planner_cost_usd();
    let total_provider_reported_upstream_cost_usd = route.probe_upstream_cost_usd
        + plan_bank_cost
        + records
            .iter()
            .map(|record| record.planner_usage.cost_usd + record.executor_usage.cost_usd)
            .sum::<f64>();
    Summary {
        schema_version: 5,
        run_id: run_id.into(),
        mode: match mode {
            Mode::Offline => "offline-reference",
            Mode::Live => "live",
        }
        .into(),
        route,
        prompt_profile: profile.into(),
        repetitions,
        lane_summaries: stats::lane_summaries(records),
        paired_effects,
        first_failures,
        plan_bank_entries: plan_bank.len(),
        plan_bank_planner_tokens: plan_bank.total_planner_tokens(),
        plan_bank_provider_reported_upstream_cost_usd: plan_bank_cost,
        total_provider_reported_upstream_cost_usd,
    }
}

fn report(summary: &Summary, records: &[CaseRecord]) -> String {
    let mut text = format!(
        "# Planning and context evaluation\n\n\
         - Run: `{}`\n\
         - Mode: `{}`\n\
         - Route: `{}` -> `{}`\n\
         - Free tier verified: `{}`\n\
         - Planning profile: `{}`\n\
         - Cases: `{}`\n\
         - Frozen plan-bank entries: `{}` ({} planner tokens, `${:.8}` provider-reported upstream cost)\n\
         - Provider-reported upstream cost: `${:.8}`\n\n\
         - Judgment status: `pending LLM trajectory review`\n\n\
         ## Factual lane receipts\n\n\
         These hidden checks and operational receipts are evidence for the judge, not plan-quality or adherence verdicts.\n\n\
         | Lane | n | Hidden checks | Retrieval | All hidden checks | Tokens | Latency ms | Cost |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|\n",
        summary.run_id,
        summary.mode,
        summary.route.product_route,
        summary.route.effective_model,
        summary.route.free_tier_verified,
        summary.prompt_profile,
        records.len(),
        summary.plan_bank_entries,
        summary.plan_bank_planner_tokens,
        summary.plan_bank_provider_reported_upstream_cost_usd,
        summary.total_provider_reported_upstream_cost_usd,
    );
    for lane in &summary.lane_summaries {
        let retrieval = lane
            .retrieval_compliance_rate
            .map(|value| format!("{:.1}%", value * 100.0))
            .unwrap_or_else(|| "—".into());
        text.push_str(&format!(
            "| {} | {} | {:.3} | {} | {:.1}% | {:.0} | {:.0} | ${:.6} |\n",
            lane.lane,
            lane.cases,
            lane.mean_hidden_check_score,
            retrieval,
            lane.hidden_check_full_success_rate * 100.0,
            lane.mean_total_tokens,
            lane.mean_latency_ms,
            lane.total_cost_usd
        ));
    }
    text.push_str("\n## Paired hidden-check effects\n\n");
    for effect in &summary.paired_effects {
        text.push_str(&format!(
            "- `{}` -> `{}`: delta {:.3}, 95% paired bootstrap CI [{:.3}, {:.3}], n={}\n",
            effect.control,
            effect.candidate,
            effect.mean_executor_delta,
            effect.ci95_low,
            effect.ci95_high,
            effect.pairs
        ));
    }
    text.push_str(
        "\nPlan quality, knowledge influence, adherence, completion honesty, and causal attribution \
         require `--judge-input` packet export followed by validated LLM verdict ingestion.\n",
    );
    text.push_str("\n## First contract failures\n\n");
    if summary.first_failures.is_empty() {
        text.push_str("None.\n");
    } else {
        for (case, failure) in &summary.first_failures {
            text.push_str(&format!("- `{case}`: {failure}\n"));
        }
    }
    text
}

fn validate_filters<'a>(
    selected: &[String],
    available: impl Iterator<Item = &'a str>,
    kind: &str,
) -> Result<(), String> {
    let available = available.collect::<Vec<_>>();
    for value in selected {
        if !available.contains(&value.as_str()) {
            return Err(format!(
                "unknown {kind} {value}; available: {}",
                available.join(",")
            ));
        }
    }
    Ok(())
}

fn env_or_dotenv(name: &str) -> Option<String> {
    if let Ok(value) = std::env::var(name) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    let contents = std::fs::read_to_string(".env").ok()?;
    contents.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        (key.trim() == name).then(|| {
            value
                .trim()
                .trim_matches(|character| character == '\'' || character == '"')
                .to_string()
        })
    })
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, format!("{body}\n")).map_err(|error| error.to_string())
}

fn csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn required_value(values: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    values.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn positive_usize(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("{flag} must be a positive integer"))
}

fn print_help() {
    println!(
        "planning_eval [--offline|--live] [--scenarios CSV] [--lanes CSV] \
         [--repetitions N] [--profile concise] [--output PATH] [--max-live-cases N]\n\
         planning_eval --judge-input RUN_OR_RESULTS [--judgments JSONL] [--output PATH]"
    );
}
