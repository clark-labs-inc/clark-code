#[path = "work_graph_benchmark/fixtures.rs"]
mod fixtures;
#[path = "work_graph_benchmark/grader.rs"]
mod grader;
#[path = "work_graph_benchmark/model.rs"]
mod model;
#[path = "work_graph_benchmark/report.rs"]
mod report;
#[path = "work_graph_benchmark/simulator.rs"]
mod simulator;
#[path = "work_graph_benchmark/simulator_helpers.rs"]
mod simulator_helpers;
#[path = "work_graph_benchmark/workspace.rs"]
mod workspace;

#[cfg(test)]
#[path = "work_graph_benchmark/tests.rs"]
mod tests;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use model::{
    CandidateKind, CandidateResult, EvidenceLevel, LaneSpec, SafetyReceipt, Scenario, UsageReceipt,
};
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
            println!("  {:34} {}", scenario.id, scenario.title);
        }
        println!("Lanes:");
        for lane in &lanes {
            println!("  {:34} {} tokens", lane.id, lane.token_budget);
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
                let candidate_result_path = run_root.join("candidate-result.json");
                let request = workspace.request(scenario, lane, &candidate_result_path);
                fs::write(
                    run_root.join("task-manifest.json"),
                    serde_json::to_vec_pretty(&request.task)?,
                )?;
                let result = match options.candidate {
                    CandidateKind::ClarkCurrent => {
                        simulator::run_current(scenario, lane, &workspace)
                    }
                    CandidateKind::Reference => {
                        simulator::run_reference(scenario, lane, &workspace)
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
                let evidence = if options.candidate == CandidateKind::External {
                    EvidenceLevel::ExternalTrace
                } else {
                    EvidenceLevel::Simulation
                };
                let record = grader::grade(
                    run_id,
                    evidence,
                    options.candidate,
                    scenario,
                    repetition,
                    lane,
                    &workspace,
                    result,
                )?;
                serde_json::to_writer(&mut result_log, &record)?;
                result_log.write_all(b"\n")?;
                println!(
                    "{:34} {:28} behavior={:.0}% lifecycle={:.0}% {}",
                    scenario.id,
                    lane.id,
                    record.behavioral_correctness * 100.0,
                    record.lifecycle_conformance * 100.0,
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
        "Required work-graph pass rate: {:.1}%",
        summary.required_graph_pass_rate * 100.0
    );

    let required_red = records
        .iter()
        .filter(|record| record.lane.is_work_graph())
        .any(|record| !record.passed());
    if required_red && !options.allow_red {
        return Err(format!(
            "candidate failed one or more required work-graph gates; inspect {} (use --allow-red only to retain an expected-red baseline)",
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
            PathBuf::from("target/work-graph-orchestration-benchmark")
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
    scenario: &Scenario,
    lane: &LaneSpec,
    error: DynError,
) -> CandidateResult {
    CandidateResult {
        schema_version: 1,
        candidate_id: candidate.id().into(),
        scenario_id: scenario.id.clone(),
        lane_id: lane.id.clone(),
        production_trace_id: None,
        delegated: false,
        delegation_reason: "candidate failed before producing a lifecycle trace".into(),
        plan: None,
        tasks: Vec::new(),
        resources: Vec::new(),
        artifacts: Vec::new(),
        wakeups: Vec::new(),
        recoveries: Vec::new(),
        verification: None,
        events: Vec::new(),
        usage: UsageReceipt::default(),
        safety: SafetyReceipt::default(),
        interaction: None,
        claimed_complete: false,
        error: Some(error.to_string()),
    }
}

fn run_external(
    command: &[String],
    request: &model::CandidateRequest,
    timeout: Duration,
    sandbox_root: &Path,
    sandboxed: bool,
) -> Result<CandidateResult, DynError> {
    let (program, args) = command
        .split_first()
        .ok_or("external candidate command cannot be empty")?;
    let stdout_path = sandbox_root.join("candidate.stdout.json");
    let stderr_path = sandbox_root.join("candidate.stderr.log");
    let candidate_home = sandbox_root.join("candidate-home");
    let candidate_tmp = sandbox_root.join("candidate-tmp");
    fs::create_dir_all(&candidate_home)?;
    fs::create_dir_all(&candidate_tmp)?;
    let mut process = sandboxed_command(program, args, sandbox_root, sandboxed)?;
    let mut child = process
        .current_dir(&request.workspace_path)
        .env("HOME", &candidate_home)
        .env("TMPDIR", &candidate_tmp)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(fs::File::create(&stdout_path)?))
        .stderr(Stdio::from(fs::File::create(&stderr_path)?))
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("external candidate stdin was unavailable")?
        .write_all(&serde_json::to_vec(request)?)?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            child.kill()?;
            child.wait()?;
            return Err(
                format!("external candidate exceeded {timeout:?} and was terminated").into(),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        return Err(format!(
            "external candidate failed ({status}): {}",
            fs::read_to_string(&stderr_path).unwrap_or_default().trim()
        )
        .into());
    }
    let bytes = fs::read(&request.result_path).or_else(|_| fs::read(&stdout_path))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn sandboxed_command(
    program: &str,
    args: &[String],
    sandbox_root: &Path,
    sandboxed: bool,
) -> Result<Command, DynError> {
    if !sandboxed {
        let mut command = Command::new(program);
        command.args(args);
        return Ok(command);
    }
    #[cfg(target_os = "macos")]
    {
        let root = sandbox_root
            .canonicalize()?
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let profile = format!(
            "(version 1) (allow default) (deny file-write*) (allow file-write* (subpath \"{root}\"))"
        );
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command.args(["-p", &profile, "--", program]).args(args);
        Ok(command)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (program, args, sandbox_root);
        Err("external candidates require a filesystem sandbox; use --unsafe-external only on a disposable machine".into())
    }
}

fn select_scenarios(
    scenarios: Vec<Scenario>,
    requested: &[String],
) -> Result<Vec<Scenario>, DynError> {
    if requested.is_empty() {
        return Ok(scenarios);
    }
    select_by_id(scenarios, requested, |scenario| &scenario.id, "scenario")
}

fn select_lanes(lanes: Vec<LaneSpec>, requested: &[String]) -> Result<Vec<LaneSpec>, DynError> {
    if requested.is_empty() {
        return Ok(lanes);
    }
    select_by_id(lanes, requested, |lane| &lane.id, "lane")
}

fn select_by_id<T: Clone>(
    values: Vec<T>,
    requested: &[String],
    id: impl Fn(&T) -> &String,
    kind: &str,
) -> Result<Vec<T>, DynError> {
    let mut selected = Vec::new();
    for requested_id in requested {
        let index = values
            .iter()
            .position(|value| id(value) == requested_id)
            .ok_or_else(|| format!("unknown {kind}: {requested_id}"))?;
        if selected.iter().all(|value| id(value) != requested_id) {
            selected.push(values[index].clone());
        }
    }
    Ok(selected)
}

fn next(
    values: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    argument: &str,
) -> Result<String, DynError> {
    values
        .next()
        .ok_or_else(|| format!("{argument} requires a value").into())
}

fn comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn print_help() {
    println!(
        "Universal work-graph orchestration benchmark\n\n\
         --candidate <clark-current|reference|external>\n\
         --candidate-command-json '[\"program\", ...]'\n\
         --scenario <id,id>  --lane <id,id>  --repetitions <n>\n\
         --out <directory>  --allow-red  --unsafe-external  --list"
    );
}
