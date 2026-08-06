#[path = "multi_repo_benchmark/candidates.rs"]
mod candidates;
#[path = "multi_repo_benchmark/fixtures.rs"]
mod fixtures;
#[path = "multi_repo_benchmark/grader.rs"]
mod grader;
#[path = "multi_repo_benchmark/model.rs"]
mod model;
#[path = "multi_repo_benchmark/production.rs"]
mod production;
#[path = "multi_repo_benchmark/report.rs"]
mod report;
#[path = "multi_repo_benchmark/verification.rs"]
mod verification;
#[path = "multi_repo_benchmark/workspace.rs"]
mod workspace;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use candidates::{run_current, run_external, run_reference};
use model::{CandidateKind, CandidateResult, EvidenceLevel, LaneSpec, SafetyReceipt, UsageReceipt};
use uuid::Uuid;
use workspace::{DynError, SeededWorkspace};

#[derive(Debug)]
struct Options {
    output: PathBuf,
    candidate: CandidateKind,
    external_command: Vec<String>,
    scenario_ids: Vec<String>,
    lane_ids: Vec<String>,
    repetitions: u32,
    strong_model: String,
    cheap_model: String,
    reviewer_model: String,
    candidate_timeout_seconds: u64,
    sandbox_external: bool,
    allow_red: bool,
    list: bool,
}

fn main() -> Result<(), DynError> {
    let options = Options::parse(std::env::args().skip(1))?;
    let scenarios = fixtures::catalog();
    let lanes = LaneSpec::catalog(
        &options.strong_model,
        &options.cheap_model,
        &options.reviewer_model,
    );
    if options.list {
        println!("Scenarios:");
        for scenario in &scenarios {
            println!("  {:32} {}", scenario.id, scenario.title);
        }
        println!("Lanes:");
        for lane in &lanes {
            println!("  {:32} {} tokens", lane.id, lane.token_budget);
        }
        return Ok(());
    }
    let scenarios = select_scenarios(scenarios, &options.scenario_ids)?;
    let lanes = select_lanes(lanes, &options.lane_ids)?;
    if options.candidate == CandidateKind::External && options.external_command.is_empty() {
        return Err(
            "--candidate external requires --candidate-command-json '[\"program\", ...]'".into(),
        );
    }

    fs::create_dir_all(&options.output)?;
    let result_log_path = options.output.join("results.jsonl");
    let mut result_log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&result_log_path)
        .map_err(|error| {
            format!(
                "refusing to overwrite {}: {error}",
                result_log_path.display()
            )
        })?;
    let mut records = Vec::new();
    for repetition in 0..options.repetitions {
        for scenario in &scenarios {
            for lane in &lanes {
                let run_id = Uuid::new_v4().to_string();
                let run_root = options.output.join("runs").join(&run_id);
                fs::create_dir_all(&run_root)?;
                let workspace = SeededWorkspace::seed(&run_root, scenario)?;
                let manifest_path = run_root.join("task-manifest.json");
                let candidate_result_path = run_root.join("candidate-result.json");
                let request =
                    workspace.request(scenario, lane, &manifest_path, &candidate_result_path);
                fs::write(&manifest_path, serde_json::to_vec_pretty(&request.task)?)?;
                let evidence_level = if options.candidate == CandidateKind::External {
                    EvidenceLevel::External
                } else {
                    EvidenceLevel::Scripted
                };
                let result = match options.candidate {
                    CandidateKind::ClarkCurrent => run_current(scenario, lane, &workspace),
                    CandidateKind::Reference => {
                        run_reference(scenario, lane, &workspace, &run_root)
                    }
                    CandidateKind::External => run_external(
                        &options.external_command,
                        &request,
                        Duration::from_secs(options.candidate_timeout_seconds),
                        &run_root,
                        options.sandbox_external,
                    ),
                }
                .unwrap_or_else(|error| failed_result(options.candidate, scenario, lane, error));
                fs::write(&candidate_result_path, serde_json::to_vec_pretty(&result)?)?;
                let record = grader::grade(
                    run_id,
                    evidence_level,
                    options.candidate,
                    scenario,
                    repetition,
                    lane,
                    &workspace,
                    &run_root,
                    result,
                )?;
                serde_json::to_writer(&mut result_log, &record)?;
                result_log.write_all(b"\n")?;
                println!(
                    "{:32} {:24} behavior={:.0}% replay={:.0}% conformance={:.0}% {}",
                    scenario.id,
                    lane.id,
                    record.behavioral_correctness * 100.0,
                    record.replay_correctness * 100.0,
                    record.conformance_score * 100.0,
                    if record.passed() { "PASS" } else { "FAIL" }
                );
                records.push(record);
            }
        }
    }
    result_log.flush()?;
    let summary = report::write_report(&options.output, &records)?;
    println!("\nReport: {}", options.output.join("report.md").display());
    println!("Results: {}", result_log_path.display());
    println!(
        "Multi-agent conformance: {:.1}%",
        summary.multi_conformance_pass_rate * 100.0
    );

    let red_multi = records
        .iter()
        .filter(|record| record.lane.is_multi())
        .any(|record| !record.passed());
    if red_multi && !options.allow_red {
        return Err(format!(
            "candidate failed one or more multi-agent gates; inspect {} (use --allow-red only when capturing an expected-red baseline)",
            options.output.join("report.md").display()
        )
        .into());
    }
    Ok(())
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, DynError> {
        let mut values = args.peekable();
        let mut output = None;
        let mut candidate = CandidateKind::ClarkCurrent;
        let mut external_command = Vec::new();
        let mut scenario_ids = Vec::new();
        let mut lane_ids = Vec::new();
        let mut repetitions = 1;
        let mut strong_model = "strong-model".to_string();
        let mut cheap_model = "cheap-model".to_string();
        let mut reviewer_model = "independent-reviewer".to_string();
        let mut candidate_timeout_seconds = 900;
        let mut sandbox_external = true;
        let mut allow_red = false;
        let mut list = false;
        while let Some(argument) = values.next() {
            match argument.as_str() {
                "--out" => output = Some(PathBuf::from(next(&mut values, "--out")?)),
                "--candidate" => {
                    candidate = match next(&mut values, "--candidate")?.as_str() {
                        "clark-current" => CandidateKind::ClarkCurrent,
                        "reference" => CandidateKind::Reference,
                        "external" => CandidateKind::External,
                        value => return Err(format!("unknown candidate: {value}").into()),
                    }
                }
                "--candidate-command-json" => {
                    external_command =
                        serde_json::from_str(&next(&mut values, "--candidate-command-json")?)?;
                }
                "--scenario" => scenario_ids = comma_list(&next(&mut values, "--scenario")?),
                "--lane" => lane_ids = comma_list(&next(&mut values, "--lane")?),
                "--repetitions" => repetitions = next(&mut values, "--repetitions")?.parse()?,
                "--strong-model" => strong_model = next(&mut values, "--strong-model")?,
                "--cheap-model" => cheap_model = next(&mut values, "--cheap-model")?,
                "--reviewer-model" => reviewer_model = next(&mut values, "--reviewer-model")?,
                "--candidate-timeout-seconds" => {
                    candidate_timeout_seconds =
                        next(&mut values, "--candidate-timeout-seconds")?.parse()?;
                }
                "--allow-red" => allow_red = true,
                "--unsafe-external" => sandbox_external = false,
                "--list" => list = true,
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                value => return Err(format!("unknown argument: {value}").into()),
            }
        }
        if repetitions == 0 {
            return Err("--repetitions must be at least 1".into());
        }
        if candidate_timeout_seconds == 0 {
            return Err("--candidate-timeout-seconds must be at least 1".into());
        }
        let output = output.unwrap_or_else(|| {
            PathBuf::from("target/multi-repo-orchestration-benchmark")
                .join(Uuid::new_v4().to_string())
        });
        let output = if output.is_absolute() {
            output
        } else {
            std::env::current_dir()?.join(output)
        };
        Ok(Self {
            output,
            candidate,
            external_command,
            scenario_ids,
            lane_ids,
            repetitions,
            strong_model,
            cheap_model,
            reviewer_model,
            candidate_timeout_seconds,
            sandbox_external,
            allow_red,
            list,
        })
    }
}

fn failed_result(
    candidate: CandidateKind,
    scenario: &model::Scenario,
    lane: &LaneSpec,
    error: DynError,
) -> CandidateResult {
    CandidateResult {
        schema_version: 1,
        candidate_id: candidate.id().into(),
        scenario_id: scenario.id.clone(),
        lane_id: lane.id.clone(),
        delegated: false,
        delegation_reason: "candidate execution failed".into(),
        planning: None,
        tasks: Vec::new(),
        change_packages: Vec::new(),
        contract_decisions: Vec::new(),
        recoveries: Vec::new(),
        integration: None,
        usage: UsageReceipt::default(),
        safety: SafetyReceipt::default(),
        interaction: None,
        claimed_complete: false,
        error: Some(error.to_string()),
    }
}

fn select_scenarios(
    scenarios: Vec<model::Scenario>,
    selected: &[String],
) -> Result<Vec<model::Scenario>, DynError> {
    select(
        scenarios,
        selected,
        |scenario| scenario.id.as_str(),
        "scenario",
    )
}

fn select_lanes(lanes: Vec<LaneSpec>, selected: &[String]) -> Result<Vec<LaneSpec>, DynError> {
    select(lanes, selected, |lane| lane.id.as_str(), "lane")
}

fn select<T>(
    values: Vec<T>,
    selected: &[String],
    id: impl Fn(&T) -> &str,
    kind: &str,
) -> Result<Vec<T>, DynError> {
    if selected.is_empty() || selected.iter().any(|value| value == "all") {
        return Ok(values);
    }
    for wanted in selected {
        if !values.iter().any(|value| id(value) == wanted) {
            return Err(format!("unknown {kind}: {wanted}").into());
        }
    }
    Ok(values
        .into_iter()
        .filter(|value| selected.iter().any(|wanted| wanted == id(value)))
        .collect())
}

fn next(values: &mut impl Iterator<Item = String>, option: &str) -> Result<String, DynError> {
    values
        .next()
        .ok_or_else(|| format!("missing value for {option}").into())
}

fn comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn print_help() {
    println!(
        "Clark multi-repository orchestration benchmark\n\n\
         Usage: cargo run -p provider-local --example multi_repo_orchestration_benchmark -- [options]\n\n\
         --candidate clark-current|reference|external\n\
         --candidate-command-json '[\"program\",\"arg\"]'\n\
         --candidate-timeout-seconds N (default 900)\n\
         --unsafe-external (disable the default write sandbox)\n\
         --scenario id,id|all   --lane id,id|all   --repetitions N\n\
         --strong-model NAME    --cheap-model NAME --reviewer-model NAME\n\
         --out PATH             --allow-red        --list"
    );
}

#[cfg(test)]
#[path = "multi_repo_benchmark/tests.rs"]
mod tests;
