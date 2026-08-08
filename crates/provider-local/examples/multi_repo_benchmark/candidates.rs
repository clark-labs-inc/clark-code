use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::model::{
    CandidateRequest, CandidateResult, ChangePackage, ContractDecision, FaultInjection,
    IntegrationReceipt, InteractionReceipt, LaneKind, LaneSpec, PlanningReceipt, RecoveryReceipt,
    SafetyReceipt, Scenario, TaskOutcome, TaskReceipt, TaskRole, UsageReceipt,
};
use super::workspace::{
    apply_patch, clone_repository, git_patch, result_tree_sha256, sha256, solution_changed_paths,
    write_files, DynError, SeededWorkspace,
};

pub fn run_current(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
) -> Result<CandidateResult, DynError> {
    if lane.is_multi() && scenario.expected_delegate {
        return super::production::run(scenario, lane, workspace);
    }
    let skip_last = scenario.single_agent_trap && !lane.is_multi();
    let last_changed = scenario
        .repositories
        .iter()
        .rposition(|repo| !repo.solution_files.is_empty());
    for (index, repo) in scenario.repositories.iter().enumerate() {
        if skip_last && Some(index) == last_changed {
            continue;
        }
        write_files(&workspace.repositories[&repo.id].root, &repo.solution_files)?;
    }

    let delegated = false;
    let planner = TaskReceipt {
        id: "planner".into(),
        repo_id: None,
        role: TaskRole::Planner,
        dependencies: Vec::new(),
        model: lane.root_model.clone(),
        model_tier: "strong".into(),
        harness: "host-planner".into(),
        isolated: false,
        started_ms: 0,
        finished_ms: 1,
        outcome: TaskOutcome::Completed,
    };
    let task = TaskReceipt {
        id: "root-direct-writer".into(),
        repo_id: None,
        role: TaskRole::Writer,
        dependencies: Vec::new(),
        model: lane.root_model.clone(),
        model_tier: "strong".into(),
        harness: "provider-local".into(),
        isolated: false,
        started_ms: 2,
        finished_ms: if delegated { 1_400 } else { 900 },
        outcome: TaskOutcome::Completed,
    };
    Ok(CandidateResult {
        schema_version: 1,
        candidate_id: "current-agent".into(),
        scenario_id: scenario.id.clone(),
        lane_id: lane.id.clone(),
        delegated,
        delegation_reason: if delegated {
            "The current adapter can delegate reading, but the root remains the only writer".into()
        } else {
            "Single root coding loop".into()
        },
        planning: Some(scripted_planning_receipt(
            scenario, lane, workspace, delegated, 1,
        )?),
        tasks: vec![planner, task],
        change_packages: Vec::new(),
        contract_decisions: Vec::new(),
        recoveries: Vec::new(),
        integration: None,
        usage: scripted_usage(lane, false),
        safety: SafetyReceipt::default(),
        interaction: None,
        claimed_complete: true,
        error: None,
    })
}

pub fn run_reference(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
    run_root: &Path,
) -> Result<CandidateResult, DynError> {
    let mut tasks = vec![task(
        "planner",
        None,
        TaskRole::Planner,
        &lane.root_model,
        "strong",
        "reference",
        false,
        0,
        100,
        TaskOutcome::Completed,
    )];
    if lane.is_multi() {
        for (index, repo) in scenario.repositories.iter().enumerate() {
            tasks.push(task(
                &format!("reader-{}", repo.id),
                Some(&repo.id),
                TaskRole::Reader,
                &lane.worker_model,
                if lane.kind == LaneKind::MultiStrong {
                    "strong"
                } else {
                    "cheap"
                },
                "reference",
                true,
                110 + index as u64 * 5,
                210 + index as u64 * 5,
                TaskOutcome::Completed,
            ));
        }
    }

    let changed_repos = scenario
        .repositories
        .iter()
        .filter(|repo| !repo.solution_files.is_empty())
        .collect::<Vec<_>>();
    let recovery_repo = matches!(
        scenario.fault,
        FaultInjection::ChildCrashAfterArtifact | FaultInjection::BaselineDrift
    )
    .then(|| changed_repos.last().map(|repo| repo.id.as_str()))
    .flatten();
    let mut failed_task_id = None;
    if let Some(repo_id) = recovery_repo {
        let id = format!("writer-{repo_id}-stale");
        tasks.push(task(
            &id,
            Some(repo_id),
            TaskRole::Writer,
            &lane.root_model,
            "strong",
            "reference",
            true,
            240,
            320,
            TaskOutcome::Failed,
        ));
        failed_task_id = Some(id);
    }

    let artifacts = run_root.join("artifacts");
    let workers = run_root.join("workers");
    fs::create_dir_all(&artifacts)?;
    fs::create_dir_all(&workers)?;
    let mut change_packages = Vec::new();
    let mut writer_ids = Vec::new();
    for (index, repo) in changed_repos.iter().enumerate() {
        let retry = recovery_repo == Some(repo.id.as_str());
        let task_id = if retry {
            format!("writer-{}-retry", repo.id)
        } else {
            format!("writer-{}", repo.id)
        };
        let concurrent_start = if scenario.expected_delegate && lane.is_multi() {
            350
        } else {
            350 + index as u64 * 350
        };
        let harness = if lane.kind == LaneKind::CloudMixed && repo.cloud_eligible {
            "brokered-cloud"
        } else {
            "reference"
        };
        tasks.push(task(
            &task_id,
            Some(&repo.id),
            TaskRole::Writer,
            &lane.root_model,
            "strong",
            harness,
            true,
            concurrent_start,
            concurrent_start + 300,
            TaskOutcome::Completed,
        ));
        writer_ids.push(task_id.clone());

        let source = &workspace.repositories[&repo.id];
        let worker = workers.join(&repo.id);
        clone_repository(&source.root, &worker)?;
        write_files(&worker, &repo.solution_files)?;
        let patch = git_patch(&worker)?;
        let patch_path = artifacts.join(format!("{}.patch", repo.id));
        fs::write(&patch_path, &patch)?;
        let changed_paths = solution_changed_paths(&worker)?;
        let patch_sha256 = sha256(&patch);
        let package = ChangePackage {
            task_id,
            repo_id: repo.id.clone(),
            base_sha: source.baseline_sha.clone(),
            changed_paths: changed_paths.clone(),
            patch_path: patch_path.display().to_string(),
            patch_sha256: patch_sha256.clone(),
            result_tree_sha256: result_tree_sha256(
                &worker,
                &source.baseline_sha,
                &patch_sha256,
                &changed_paths,
            )?,
            isolation: if harness == "brokered-cloud" {
                "cloud-ephemeral-clone".into()
            } else {
                "local-ephemeral-clone".into()
            },
            tests: repo.public_checks.clone(),
        };
        apply_patch(&source.root, &patch)?;
        change_packages.push(package);
    }

    let review_required = matches!(
        lane.kind,
        LaneKind::MultiDiverseReview | LaneKind::CloudMixed
    );
    if review_required {
        tasks.push(task(
            "independent-reviewer",
            None,
            TaskRole::Reviewer,
            lane.reviewer_model.as_deref().unwrap_or("reviewer"),
            "reviewer",
            "reference-review",
            true,
            680,
            790,
            TaskOutcome::Completed,
        ));
    }
    tasks.push(task(
        "fresh-integrator",
        None,
        TaskRole::Integrator,
        &lane.root_model,
        "strong",
        "reference",
        true,
        800,
        1_000,
        TaskOutcome::Completed,
    ));

    let contract_decisions = scenario
        .edges
        .iter()
        .map(|edge| ContractDecision {
            edge_id: edge.id.clone(),
            producer_repo: edge.producer_repo.clone(),
            consumer_repos: edge.consumer_repos.clone(),
            artifact_sha256: sha256(
                format!("{}:{}", edge.artifact, edge.compatibility_rule).as_bytes(),
            ),
            compatibility_rule: edge.compatibility_rule.clone(),
            approved_by: if review_required {
                "independent-reviewer".into()
            } else {
                "fresh-integrator".into()
            },
        })
        .collect::<Vec<_>>();
    let recoveries = failed_task_id
        .map(|failed| {
            let replacement = writer_ids
                .iter()
                .find(|id| id.ends_with("-retry"))
                .cloned()
                .unwrap_or_default();
            vec![RecoveryReceipt {
                failed_task_id: failed,
                replacement_task_id: replacement.clone(),
                preserved_task_ids: writer_ids
                    .iter()
                    .filter(|id| **id != replacement)
                    .cloned()
                    .collect(),
                reused_artifact_sha256: change_packages
                    .iter()
                    .find(|package| package.task_id == replacement)
                    .map(|package| package.patch_sha256.clone()),
            }]
        })
        .unwrap_or_default();
    let integration = IntegrationReceipt {
        fresh_workspace: true,
        repo_baselines: workspace
            .repositories
            .iter()
            .map(|(id, repo)| (id.clone(), repo.baseline_sha.clone()))
            .collect(),
        repo_result_trees: change_packages
            .iter()
            .map(|package| (package.repo_id.clone(), package.result_tree_sha256.clone()))
            .collect(),
        applied_patch_sha256: change_packages
            .iter()
            .map(|package| package.patch_sha256.clone())
            .collect(),
        checks_run: scenario
            .hidden_checks
            .iter()
            .enumerate()
            .map(|(index, _)| format!("hidden-check-{index}"))
            .collect(),
        passed: true,
    };
    Ok(CandidateResult {
        schema_version: 1,
        candidate_id: "reference".into(),
        scenario_id: scenario.id.clone(),
        lane_id: lane.id.clone(),
        delegated: scenario.expected_delegate && lane.is_multi(),
        delegation_reason: if scenario.expected_delegate && lane.is_multi() {
            "Independent repositories with an explicit integration contract".into()
        } else {
            "Sequential dependency chain or single-agent control".into()
        },
        planning: Some(scripted_planning_receipt(
            scenario,
            lane,
            workspace,
            scenario.expected_delegate && lane.is_multi(),
            50,
        )?),
        tasks,
        change_packages,
        contract_decisions,
        recoveries,
        integration: Some(integration),
        usage: scripted_usage(lane, true),
        safety: SafetyReceipt::default(),
        interaction: Some(non_technical_reference_flow(scenario)),
        claimed_complete: true,
        error: None,
    })
}

fn scripted_planning_receipt(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
    delegated: bool,
    validated_ms: u64,
) -> Result<PlanningReceipt, DynError> {
    Ok(PlanningReceipt {
        planner_task_id: "planner".into(),
        plan_sha256: sha256(&serde_json::to_vec(
            &workspace.public_manifest(scenario, lane),
        )?),
        repository_baselines: workspace
            .repositories
            .iter()
            .map(|(id, repository)| (id.clone(), repository.baseline_sha.clone()))
            .collect(),
        delegated,
        validated_ms,
    })
}

fn non_technical_reference_flow(scenario: &Scenario) -> InteractionReceipt {
    InteractionReceipt {
        default_flow: true,
        setup_actions: 2,
        cloud_consent_prompts: u32::from(
            scenario.repositories.iter().any(|repo| repo.cloud_eligible),
        ),
        completion_actions: 1,
        model_choice_required: false,
        agent_configuration_required: false,
        version_control_knowledge_required: false,
        advanced_details_collapsed: true,
        plain_language_progress: true,
        exposed_internal_terms: Vec::new(),
    }
}

pub fn run_external(
    command: &[String],
    request: &CandidateRequest,
    timeout: Duration,
    sandbox_root: &Path,
    sandboxed: bool,
) -> Result<CandidateResult, DynError> {
    let (program, args) = command
        .split_first()
        .ok_or("external candidate command cannot be empty")?;
    let stdout_path = format!("{}.stdout.json", request.result_path);
    let stderr_path = format!("{}.stderr.log", request.result_path);
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
            "external candidate failed ({}): {}",
            status,
            fs::read_to_string(&stderr_path).unwrap_or_default().trim()
        )
        .into());
    }
    let bytes = fs::read(&request.result_path).or_else(|_| {
        let stdout = fs::read(&stdout_path)?;
        if stdout.is_empty() {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "candidate wrote neither result_path nor stdout",
            ))
        } else {
            Ok(stdout)
        }
    })?;
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
        Err("external candidates require a filesystem sandbox; use --unsafe-external only in a disposable environment".into())
    }
}

#[allow(clippy::too_many_arguments)]
fn task(
    id: &str,
    repo_id: Option<&str>,
    role: TaskRole,
    model: &str,
    model_tier: &str,
    harness: &str,
    isolated: bool,
    started_ms: u64,
    finished_ms: u64,
    outcome: TaskOutcome,
) -> TaskReceipt {
    TaskReceipt {
        id: id.into(),
        repo_id: repo_id.map(str::to_string),
        role,
        dependencies: Vec::new(),
        model: model.into(),
        model_tier: model_tier.into(),
        harness: harness.into(),
        isolated,
        started_ms,
        finished_ms,
        outcome,
    }
}

fn scripted_usage(lane: &LaneSpec, reference: bool) -> UsageReceipt {
    let (input, output, useful, duplicate, wall, agent) = match lane.kind {
        LaneKind::Single => (58_000, 8_000, 26_000, 9_000, 1_500, 1_500),
        LaneKind::EqualBudgetSingle => (210_000, 26_000, 62_000, 88_000, 4_500, 4_500),
        LaneKind::MultiCheap => (118_000, 19_000, 74_000, 17_000, 1_750, 5_600),
        LaneKind::MultiStrong => (126_000, 23_000, 80_000, 19_000, 1_850, 5_900),
        LaneKind::MultiDiverseReview => (146_000, 27_000, 92_000, 20_000, 2_100, 6_800),
        LaneKind::CloudMixed => (158_000, 29_000, 96_000, 22_000, 2_500, 7_400),
    };
    let penalty = if reference { 1.0 } else { 1.18 };
    UsageReceipt {
        input_tokens: (input as f64 * penalty) as u64,
        output_tokens: (output as f64 * penalty) as u64,
        cached_input_tokens: input / 5,
        useful_tokens: useful,
        duplicate_read_tokens: duplicate,
        cost_usd: (input as f64 * 0.000_000_2 + output as f64 * 0.000_000_8) * penalty,
        wall_ms: wall,
        agent_ms: agent,
    }
}
