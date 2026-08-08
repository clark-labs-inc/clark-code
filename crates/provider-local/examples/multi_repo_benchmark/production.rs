use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_core::domain::{AgentEvent, ContentBlock, Role, RunOutcome, RunStatus, RunUsage};
use agent_core::error::{Error as CoreError, Result as CoreResult};
use agent_core::ids::{ProviderId, RunId, SessionId};
use agent_core::provider::{
    ClientResponse, EventStream, PromptInput, Provider, ProviderCapabilities, ProviderConfig,
    Session, SessionEnvironment, SessionOptions,
};
use agent_orchestration::{
    BudgetConfig, ContractDecision as CoreContractDecision, HarnessKind, IntegrationCheck,
    ModelTier, MultiRepoCoordinator, MultiRepoPlan, MultiRepoRunResult, MultiRepoTask,
    MultiRepoTaskRole, ProviderFactory, RepositoryContractEdge, RepositoryId, SharedBudget, TaskId,
    TaskRunOutcome,
};
use async_trait::async_trait;
use base64::Engine;
use futures::stream;
use provider_local::{
    BrokeredCloudWriterConfig, BrokeredCloudWriterHarness, LocalExecutor, LocalMultiRepoRuntime,
    RepositorySelection, RepositorySelectionRequest,
};
use tokio_util::sync::CancellationToken;

use super::model::{
    CandidateResult, ChangePackage, ContractDecision, FaultInjection, FileFixture,
    IntegrationReceipt, LaneKind, LaneSpec, RecoveryReceipt, RepositorySpec, SafetyReceipt,
    Scenario, TaskOutcome, TaskReceipt, TaskRole, UsageReceipt,
};
use super::workspace::{apply_patch, sha256, write_files, DynError, SeededWorkspace};

pub fn run(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
) -> Result<CandidateResult, DynError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async(scenario, lane, workspace))
}

async fn run_async(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
) -> Result<CandidateResult, DynError> {
    let selection = Arc::new(
        RepositorySelection::resolve(
            &LocalExecutor,
            scenario
                .repositories
                .iter()
                .map(|repo| RepositorySelectionRequest {
                    repository_id: RepositoryId::new(&repo.id).expect("fixture repository id"),
                    root: workspace.repositories[&repo.id].root.clone(),
                    allowed_changed_paths: repo.allowed_changed_paths.clone(),
                    cloud_eligible: repo.cloud_eligible,
                })
                .collect(),
        )
        .await?,
    );
    let plan = Arc::new(build_plan(scenario, lane, &selection)?);
    let run_root = workspace
        .root
        .parent()
        .ok_or("benchmark workspace has no run root")?;
    let scratch_root = run_root.join("production-scratch");
    let artifact_root = run_root.join("production-artifacts");
    let state = Arc::new(FixtureState::new(scenario));
    let factory: Arc<dyn ProviderFactory> = Arc::new({
        let state = state.clone();
        move || {
            Box::new(FixtureProvider {
                state: state.clone(),
                cwd: None,
            }) as Box<dyn Provider>
        }
    });
    let provider_config = ProviderConfig {
        extra: serde_json::json!({}),
        ..Default::default()
    };
    let local = LocalMultiRepoRuntime::new(
        provider_local::LocalMultiRepoRuntimeConfig {
            provider_config: provider_config.clone(),
            timeout: Duration::from_secs(10),
            scratch_root: scratch_root.clone(),
            artifact_root: artifact_root.clone(),
            selection: selection.clone(),
            plan: plan.clone(),
            integration_gate: None,
        },
        factory.clone(),
        Arc::new(LocalExecutor),
    )?;
    let integrator = Arc::new(local.integration_harness("local-integrator")?);
    let mut coordinator = MultiRepoCoordinator::new(
        (*plan).clone(),
        SharedBudget::new(BudgetConfig {
            limit_weighted_tokens: lane.token_budget,
            ..Default::default()
        })?,
        2,
        integrator,
    )?;
    coordinator.register_reader(Arc::new(local.reader_harness("local-reader")?))?;
    if plan
        .tasks
        .iter()
        .any(|task| task.role == MultiRepoTaskRole::Writer && task.harness == "local-writer")
    {
        coordinator.register_writer(Arc::new(local.writer_harness("local-writer")?))?;
    }
    if plan
        .tasks
        .iter()
        .any(|task| task.role == MultiRepoTaskRole::Writer && task.harness == "cloud-writer")
    {
        coordinator.register_writer(Arc::new(BrokeredCloudWriterHarness::new(
            BrokeredCloudWriterConfig {
                id: "cloud-writer".into(),
                provider_config,
                timeout: Duration::from_secs(10),
                scratch_root,
                artifact_root,
                max_upload_bytes: 1_000_000,
            },
            selection,
            plan.clone(),
            factory,
            Arc::new(LocalExecutor),
        )?))?;
    }
    if plan.requires_independent_review {
        coordinator.register_reviewer(Arc::new(local.reviewer_harness("local-review")?))?;
    }
    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await?;
    if result.error.is_none() {
        apply_verified_packages(workspace, &result)?;
    }
    Ok(candidate_result(scenario, lane, &plan, result))
}

fn build_plan(
    scenario: &Scenario,
    lane: &LaneSpec,
    selection: &RepositorySelection,
) -> Result<MultiRepoPlan, String> {
    let planner = TaskId::new("planner")?;
    let mut tasks = vec![task(
        "planner",
        MultiRepoTaskRole::Planner,
        None,
        BTreeSet::new(),
        "Decide repository boundaries and compatibility contracts".into(),
        "planner",
        HarnessKind::Local,
        &lane.root_model,
        ModelTier::Strong,
        BTreeSet::new(),
    )?];
    let reader_tier = if lane.kind == LaneKind::MultiStrong {
        ModelTier::Strong
    } else {
        ModelTier::Cheap
    };
    let reader_model = if reader_tier == ModelTier::Strong {
        &lane.root_model
    } else {
        &lane.worker_model
    };
    for repo in &scenario.repositories {
        tasks.push(task(
            &format!("{}-reader", repo.id),
            MultiRepoTaskRole::Reader,
            Some(&repo.id),
            BTreeSet::from([planner.clone()]),
            format!(
                "Find implementation evidence for fixture_repository={};",
                repo.id
            ),
            "local-reader",
            HarnessKind::Local,
            reader_model,
            reader_tier,
            BTreeSet::new(),
        )?);
    }
    let mut writer_ids = Vec::new();
    for repo in scenario
        .repositories
        .iter()
        .filter(|repo| !repo.allowed_changed_paths.is_empty())
    {
        let id = format!("{}-writer", repo.id);
        let harness_kind = if lane.kind == LaneKind::CloudMixed && repo.cloud_eligible {
            HarnessKind::BrokeredCloud
        } else {
            HarnessKind::Local
        };
        let harness = if harness_kind == HarnessKind::BrokeredCloud {
            "cloud-writer"
        } else {
            "local-writer"
        };
        tasks.push(task(
            &id,
            MultiRepoTaskRole::Writer,
            Some(&repo.id),
            BTreeSet::from([planner.clone(), TaskId::new(format!("{}-reader", repo.id))?]),
            format!("fixture_repository={}; {}", repo.id, scenario.prompt),
            harness,
            harness_kind,
            &lane.root_model,
            ModelTier::Strong,
            repo.allowed_changed_paths.clone(),
        )?);
        writer_ids.push(TaskId::new(id)?);
    }
    let review_required = matches!(
        lane.kind,
        LaneKind::MultiDiverseReview | LaneKind::CloudMixed
    );
    let integration_dependencies = if review_required {
        tasks.push(task(
            "reviewer",
            MultiRepoTaskRole::Reviewer,
            None,
            writer_ids.iter().cloned().collect(),
            format!("Independently verify {}", scenario.prompt),
            "local-review",
            HarnessKind::Local,
            lane.reviewer_model.as_deref().unwrap_or("reviewer"),
            ModelTier::Reviewer,
            BTreeSet::new(),
        )?);
        BTreeSet::from([TaskId::new("reviewer")?])
    } else {
        writer_ids.iter().cloned().collect()
    };
    tasks.push(task(
        "integrator",
        MultiRepoTaskRole::Integrator,
        None,
        integration_dependencies,
        format!("Freshly replay and verify {}", scenario.prompt),
        "local-integrator",
        HarnessKind::Local,
        &lane.root_model,
        ModelTier::Strong,
        BTreeSet::new(),
    )?);
    let contracts = scenario
        .edges
        .iter()
        .map(|edge| {
            Ok(RepositoryContractEdge {
                id: edge.id.clone(),
                producer: RepositoryId::new(&edge.producer_repo)?,
                consumers: edge
                    .consumer_repos
                    .iter()
                    .map(RepositoryId::new)
                    .collect::<Result<_, _>>()?,
                artifact: edge.artifact.clone(),
                compatibility_rule: edge.compatibility_rule.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let contract_decisions = scenario
        .edges
        .iter()
        .map(|edge| CoreContractDecision {
            edge_id: edge.id.clone(),
            decided_by: planner.clone(),
            artifact_sha256: sha256(
                format!("{}:{}", edge.artifact, edge.compatibility_rule).as_bytes(),
            ),
            compatibility_rule: edge.compatibility_rule.clone(),
        })
        .collect();
    let plan = MultiRepoPlan {
        repositories: selection.baselines(),
        contracts,
        contract_decisions,
        tasks,
        integration_checks: scenario
            .repositories
            .iter()
            .filter(|repo| !repo.solution_files.is_empty())
            .map(|repo| IntegrationCheck {
                id: format!("{}-syntax-and-files", repo.id),
                repository_id: RepositoryId::new(&repo.id).expect("fixture repository id"),
                argv: fixture_check_argv(repo),
                timeout_ms: 10_000,
            })
            .collect(),
        max_parallel_writers: lane.max_parallel_writers,
        requires_independent_review: review_required,
    };
    plan.validate()?;
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
fn task(
    id: &str,
    role: MultiRepoTaskRole,
    repository: Option<&str>,
    dependencies: BTreeSet<TaskId>,
    objective: String,
    harness: &str,
    harness_kind: HarnessKind,
    model: &str,
    model_tier: ModelTier,
    allowed_changed_paths: BTreeSet<String>,
) -> Result<MultiRepoTask, String> {
    Ok(MultiRepoTask {
        id: TaskId::new(id)?,
        role,
        repository_id: repository.map(RepositoryId::new).transpose()?,
        dependencies,
        objective,
        harness: harness.into(),
        harness_kind,
        model: model.into(),
        model_tier,
        budget_reservation: match model_tier {
            ModelTier::Cheap => 2_000,
            ModelTier::Strong => 20_000,
            ModelTier::Reviewer => 10_000,
        },
        allowed_changed_paths,
    })
}

fn fixture_check_argv(repository: &RepositorySpec) -> Vec<String> {
    let mut argv = vec![
        "python3".into(),
        "-c".into(),
        "import pathlib,sys\nfor raw in sys.argv[1:]:\n p=pathlib.Path(raw); data=p.read_bytes(); assert data\n if p.suffix == '.py': compile(data, raw, 'exec')"
            .into(),
    ];
    argv.extend(
        repository
            .solution_files
            .iter()
            .map(|file| file.path.clone()),
    );
    argv
}

fn apply_verified_packages(
    workspace: &SeededWorkspace,
    result: &MultiRepoRunResult,
) -> Result<(), DynError> {
    for package in &result.change_packages {
        let patch = fs::read(&package.artifact_path)?;
        if sha256(&patch) != package.patch_sha256 {
            return Err("production package changed after coordinator validation".into());
        }
        apply_patch(
            &workspace.repositories[package.repository_id.as_str()].root,
            &patch,
        )?;
    }
    Ok(())
}

fn candidate_result(
    scenario: &Scenario,
    lane: &LaneSpec,
    plan: &MultiRepoPlan,
    result: MultiRepoRunResult,
) -> CandidateResult {
    let receipts = result
        .tasks
        .iter()
        .map(|receipt| task_receipt(plan, receipt))
        .collect::<Vec<_>>();
    let packages = result
        .change_packages
        .iter()
        .map(|package| ChangePackage {
            task_id: package.task_id.0.clone(),
            repo_id: package.repository_id.to_string(),
            base_sha: package.base_head_oid.clone(),
            changed_paths: package.changed_paths.clone(),
            patch_path: package.artifact_path.clone(),
            patch_sha256: package.patch_sha256.clone(),
            result_tree_sha256: package.result_tree_sha256.clone(),
            isolation: match package.isolation {
                agent_orchestration::IsolationKind::CloudEphemeralClone => "cloud-ephemeral-clone",
                agent_orchestration::IsolationKind::LocalEphemeralClone => "local-ephemeral-clone",
                agent_orchestration::IsolationKind::DetachedWorktree => "detached-worktree",
            }
            .into(),
            tests: package.checks_run.clone(),
        })
        .collect::<Vec<_>>();
    let recoveries = result
        .recoveries
        .iter()
        .map(|recovery| RecoveryReceipt {
            failed_task_id: format!("{}-attempt-1", recovery.failed_task_id),
            replacement_task_id: recovery.failed_task_id.0.clone(),
            preserved_task_ids: packages
                .iter()
                .filter(|package| {
                    recovery
                        .preserved_package_sha256
                        .contains(&package.patch_sha256)
                })
                .map(|package| package.task_id.clone())
                .collect(),
            reused_artifact_sha256: recovery.reused_artifact_sha256.clone(),
        })
        .collect();
    let integration = result
        .integration
        .as_ref()
        .map(|receipt| IntegrationReceipt {
            fresh_workspace: receipt.fresh_workspace,
            repo_baselines: receipt
                .repository_baselines
                .iter()
                .map(|(id, sha)| (id.to_string(), sha.clone()))
                .collect(),
            repo_result_trees: receipt
                .repository_result_trees
                .iter()
                .map(|(id, sha)| (id.to_string(), sha.clone()))
                .collect(),
            applied_patch_sha256: receipt.applied_patch_sha256.clone(),
            checks_run: receipt.checks_run.clone(),
            passed: receipt.passed,
        });
    let error = result.error.clone();
    CandidateResult {
        schema_version: 1,
        candidate_id: "current-agent".into(),
        scenario_id: scenario.id.clone(),
        lane_id: lane.id.clone(),
        delegated: result.decomposition.delegated,
        delegation_reason: result.decomposition.reasons.join("; "),
        planning: Some(super::model::PlanningReceipt {
            planner_task_id: result.planning.planner_task_id.0.clone(),
            plan_sha256: result.planning.plan_sha256.clone(),
            repository_baselines: result
                .planning
                .repository_baselines
                .iter()
                .map(|(id, sha)| (id.to_string(), sha.clone()))
                .collect(),
            delegated: result.planning.delegated,
            validated_ms: result.planning.validated_ms,
        }),
        tasks: receipts,
        change_packages: packages,
        contract_decisions: scenario
            .edges
            .iter()
            .zip(plan.contract_decisions.iter())
            .map(|(edge, decision)| ContractDecision {
                edge_id: edge.id.clone(),
                producer_repo: edge.producer_repo.clone(),
                consumer_repos: edge.consumer_repos.clone(),
                artifact_sha256: decision.artifact_sha256.clone(),
                compatibility_rule: decision.compatibility_rule.clone(),
                approved_by: decision.decided_by.0.clone(),
            })
            .collect(),
        recoveries,
        integration,
        usage: usage(&result),
        safety: SafetyReceipt::default(),
        // This backend adapter cannot prove the desktop's default UI. Keeping
        // this absent intentionally leaves the current lane red until the real UI trace
        // is wired into the benchmark.
        interaction: None,
        claimed_complete: error.is_none(),
        error,
    }
}

fn task_receipt(
    plan: &MultiRepoPlan,
    receipt: &agent_orchestration::TaskExecutionReceipt,
) -> TaskReceipt {
    let task = plan
        .tasks
        .iter()
        .find(|task| task.id == receipt.task_id)
        .expect("coordinator receipt belongs to the validated plan");
    let id =
        if receipt.role == MultiRepoTaskRole::Writer && receipt.outcome == TaskRunOutcome::Failed {
            format!("{}-attempt-{}", receipt.task_id, receipt.attempt)
        } else {
            receipt.task_id.0.clone()
        };
    TaskReceipt {
        id,
        repo_id: receipt.repository_id.as_ref().map(ToString::to_string),
        role: match receipt.role {
            MultiRepoTaskRole::Planner => TaskRole::Planner,
            MultiRepoTaskRole::Reader => TaskRole::Reader,
            MultiRepoTaskRole::Writer => TaskRole::Writer,
            MultiRepoTaskRole::Reviewer => TaskRole::Reviewer,
            MultiRepoTaskRole::Integrator => TaskRole::Integrator,
        },
        dependencies: task.dependencies.iter().map(|id| id.0.clone()).collect(),
        model: receipt.model.clone(),
        model_tier: match receipt.model_tier {
            ModelTier::Cheap => "cheap",
            ModelTier::Strong => "strong",
            ModelTier::Reviewer => "reviewer",
        }
        .into(),
        harness: match task.harness_kind {
            HarnessKind::BrokeredCloud => "brokered-cloud".into(),
            _ => receipt.harness.clone(),
        },
        isolated: receipt.role != MultiRepoTaskRole::Planner,
        started_ms: receipt.started_ms,
        finished_ms: receipt.finished_ms,
        outcome: match receipt.outcome {
            TaskRunOutcome::Completed => TaskOutcome::Completed,
            TaskRunOutcome::Failed => TaskOutcome::Failed,
        },
    }
}

fn usage(result: &MultiRepoRunResult) -> UsageReceipt {
    let input_tokens = result
        .tasks
        .iter()
        .map(|task| task.usage.input_tokens)
        .sum();
    let output_tokens = result
        .tasks
        .iter()
        .map(|task| task.usage.output_tokens)
        .sum();
    let cached_input_tokens = result
        .tasks
        .iter()
        .map(|task| task.usage.cached_input_tokens)
        .sum();
    let cost_usd = result.tasks.iter().map(|task| task.usage.cost_usd).sum();
    let started = result
        .tasks
        .iter()
        .map(|task| task.started_ms)
        .min()
        .unwrap_or(0);
    let finished = result
        .tasks
        .iter()
        .map(|task| task.finished_ms)
        .max()
        .unwrap_or(started);
    UsageReceipt {
        input_tokens,
        output_tokens,
        cached_input_tokens,
        useful_tokens: output_tokens,
        duplicate_read_tokens: 0,
        cost_usd,
        wall_ms: finished.saturating_sub(started),
        agent_ms: result
            .tasks
            .iter()
            .map(|task| task.finished_ms.saturating_sub(task.started_ms))
            .sum(),
    }
}

struct FixtureState {
    solutions: BTreeMap<String, Vec<FileFixture>>,
    fault: FaultInjection,
    fault_repository: Option<String>,
    failed_once: Mutex<BTreeSet<String>>,
}

impl FixtureState {
    fn new(scenario: &Scenario) -> Self {
        Self {
            solutions: scenario
                .repositories
                .iter()
                .map(|repo| (repo.id.clone(), repo.solution_files.clone()))
                .collect(),
            fault: scenario.fault,
            fault_repository: scenario
                .repositories
                .iter()
                .rev()
                .find(|repo| !repo.solution_files.is_empty())
                .map(|repo| repo.id.clone()),
            failed_once: Mutex::new(BTreeSet::new()),
        }
    }

    fn should_fail(&self, repository: &str) -> bool {
        if !matches!(
            self.fault,
            FaultInjection::ChildCrashAfterArtifact | FaultInjection::BaselineDrift
        ) || self.fault_repository.as_deref() != Some(repository)
        {
            return false;
        }
        self.failed_once
            .lock()
            .expect("fixture failure lock")
            .insert(repository.to_string())
    }
}

struct FixtureProvider {
    state: Arc<FixtureState>,
    cwd: Option<PathBuf>,
}

#[async_trait]
impl Provider for FixtureProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("benchmark-fixture")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    async fn connect(&mut self, _config: ProviderConfig) -> CoreResult<()> {
        Ok(())
    }

    async fn new_session(&mut self, options: SessionOptions) -> CoreResult<Session> {
        self.cwd = options.cwd.map(PathBuf::from);
        Ok(Session {
            id: SessionId::new(uuid::Uuid::new_v4().to_string()),
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: options.mode,
            collaboration_mode: options.collaboration_mode.unwrap_or_default(),
            environment: Some(SessionEnvironment::default()),
        })
    }

    async fn load_session(&mut self, _id: SessionId) -> CoreResult<Session> {
        Err(CoreError::Unsupported(
            "fixture provider cannot resume".into(),
        ))
    }

    async fn prompt(
        &mut self,
        _session: &SessionId,
        input: PromptInput,
    ) -> CoreResult<EventStream> {
        let text = input
            .blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let (message, input_tokens, output_tokens, cost) = if text
            .contains("bounded, read-only repository reader")
        {
            (
                    r#"{"summary":"located the leased contract surface","evidence_refs":["repository:1"]}"#.to_string(),
                    20,
                    5,
                    0.000_01,
                )
        } else if text.contains("independent, read-only reviewer") {
            (
                r#"{"decision":"accept","rework_task_ids":[],"findings":[]}"#.to_string(),
                30,
                8,
                0.000_05,
            )
        } else {
            let repository = fixture_repository(&text)
                .ok_or_else(|| CoreError::Protocol("writer prompt omitted repository".into()))?;
            let files = self
                .state
                .solutions
                .get(repository)
                .ok_or_else(|| CoreError::Protocol("unknown fixture repository".into()))?;
            tokio::time::sleep(Duration::from_millis(25)).await;
            if self.state.should_fail(repository) {
                return Err(CoreError::Other(format!(
                    "injected {:?} for {repository}",
                    self.state.fault
                )));
            }
            if text.contains("brokered cloud repository writer") {
                if input.attachments.len() != 1 {
                    return Err(CoreError::Protocol(
                        "cloud writer must receive one bounded source lease".into(),
                    ));
                }
                let cwd = self.cwd.as_deref().ok_or_else(|| {
                    CoreError::Protocol("cloud fixture session has no clone".into())
                })?;
                let patch = patch_for_solution(cwd, files)?;
                (
                    serde_json::json!({
                        "patch_base64": base64::engine::general_purpose::STANDARD.encode(patch)
                    })
                    .to_string(),
                    80,
                    20,
                    0.000_2,
                )
            } else {
                let cwd = self.cwd.as_deref().ok_or_else(|| {
                    CoreError::Protocol("local fixture session has no clone".into())
                })?;
                write_files(cwd, files).map_err(|error| CoreError::Io(error.to_string()))?;
                ("implemented in isolated clone".into(), 80, 20, 0.000_2)
            }
        };
        let run = RunId::new(uuid::Uuid::new_v4().to_string());
        Ok(Box::pin(stream::iter(vec![
            AgentEvent::MessageChunk {
                run: run.clone(),
                role: Role::Agent,
                delta: ContentBlock::text(message),
            },
            AgentEvent::RunFinished {
                run,
                outcome: RunOutcome {
                    status: RunStatus::Done,
                    stop_reason: None,
                    error: None,
                    failure_kind: None,
                    usage: Some(RunUsage {
                        input_tokens,
                        output_tokens,
                        cost_usd: Some(cost),
                        ..Default::default()
                    }),
                    execution: None,
                },
            },
        ])))
    }

    async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> CoreResult<()> {
        Ok(())
    }

    async fn respond(&mut self, _session: &SessionId, _response: ClientResponse) -> CoreResult<()> {
        Ok(())
    }
}

fn fixture_repository(prompt: &str) -> Option<&str> {
    prompt
        .split("fixture_repository=")
        .nth(1)
        .and_then(|suffix| suffix.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn patch_for_solution(root: &Path, files: &[FileFixture]) -> CoreResult<Vec<u8>> {
    let temp = tempfile::tempdir()?;
    let clone = temp.path().join("repository");
    command(
        Command::new("git")
            .args(["clone", "--quiet", "--no-hardlinks", "--no-checkout", "--"])
            .arg(root)
            .arg(&clone),
        "clone Cloud fixture baseline",
    )?;
    command(
        Command::new("git")
            .current_dir(&clone)
            .args(["checkout", "--quiet", "--detach", "HEAD"]),
        "checkout Cloud fixture baseline",
    )?;
    write_files(&clone, files).map_err(|error| CoreError::Io(error.to_string()))?;
    let output = Command::new("git")
        .current_dir(&clone)
        .args(["diff", "--binary", "--no-ext-diff", "HEAD", "--"])
        .output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(CoreError::Other(format!(
            "create Cloud fixture patch failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn command(command: &mut Command, context: &str) -> CoreResult<()> {
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CoreError::Other(format!(
            "{context}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}
