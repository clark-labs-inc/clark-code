use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_core::domain::{FanOutAgent, FanOutStatus, ToolKind};
use agent_core::provider::{Provider, ProviderConfig};
use agent_orchestration::{
    BudgetConfig, HarnessKind, IntegrationCheck, ModelTier, MultiRepoCoordinator,
    MultiRepoCoordinatorEvent, MultiRepoPlan, MultiRepoRunResult, MultiRepoTask, MultiRepoTaskRole,
    ProviderFactory, RepositoryId, SharedBudget, TaskId, TaskRunOutcome,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[path = "orchestration_writer_resources.rs"]
mod resources;

use crate::orchestration::OrchestrationToolsConfig;
use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};
use crate::{
    LocalAgentProvider, LocalMultiRepoRuntime, LocalMultiRepoRuntimeConfig, RepositorySelection,
    RepositorySelectionRequest,
};

const WRITER_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
struct PendingRun {
    selection: Arc<RepositorySelection>,
    plan: Arc<MultiRepoPlan>,
    result: MultiRepoRunResult,
    scratch_root: PathBuf,
    resources: Vec<resources::ResourceReceipt>,
}

struct SharedState {
    config: OrchestrationToolsConfig,
    pending: Mutex<HashMap<String, PendingRun>>,
}

pub(super) fn tools(config: OrchestrationToolsConfig) -> Vec<Arc<dyn ToolExecutor>> {
    let shared = Arc::new(SharedState {
        config,
        pending: Mutex::new(HashMap::new()),
    });
    vec![
        Arc::new(DelegateCodingWorkstreams {
            shared: shared.clone(),
        }),
        Arc::new(ResolveCodingWorkstreams { shared }),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateArgs {
    objective: String,
    integration_checks: Vec<CheckArgs>,
    #[serde(default)]
    resources: Vec<resources::ResourceArgs>,
    workstreams: Vec<WorkstreamArgs>,
    #[serde(default)]
    independent_review: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkstreamArgs {
    id: String,
    objective: String,
    paths: BTreeSet<String>,
    #[serde(default)]
    dependencies: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckArgs {
    id: String,
    argv: Vec<String>,
    #[serde(default = "default_check_timeout")]
    timeout_ms: u64,
}

fn default_check_timeout() -> u64 {
    120_000
}

struct DelegateCodingWorkstreams {
    shared: Arc<SharedState>,
}

#[async_trait]
impl ToolExecutor for DelegateCodingWorkstreams {
    fn name(&self) -> &str {
        "delegate_coding_workstreams"
    }

    fn description(&self) -> &str {
        "Implement at least two genuinely independent workstreams in parallel disposable Git clones. Use only when the current user request, repository instructions, or an active skill explicitly authorizes delegation and parallel work has material value. Every writer gets an exact, non-overlapping path lease; the host hashes each patch, freshly replays all patches, and runs approved integration checks without touching the primary checkout. Resolve the successful run separately to apply or discard it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "objective": {"type": "string", "description": "The complete implementation outcome shared by every workstream."},
                "integration_checks": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Program and arguments, without shell interpolation."},
                            "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 600000}
                        },
                        "required": ["id", "argv"],
                        "additionalProperties": false
                    }
                },
                "resources": {
                    "type": "array",
                    "description": "Optional environment setup or service commands. The host starts all of them before writers, supervises readiness without model polling, gates integration on readiness, and cleans them up afterward.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "description": "Stable resource lease id."},
                            "workdir": {"type": "string", "description": "Optional project-relative working directory."},
                            "command": {"type": "string", "description": "Setup or service command to run in the project."},
                            "output_contains": {"type": "string", "description": "Optional exact readiness marker. Without it, successful process exit means ready."},
                            "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 600000}
                        },
                        "required": ["id", "command"],
                        "additionalProperties": false
                    }
                },
                "workstreams": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 4,
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "description": "Stable lowercase task id."},
                            "objective": {"type": "string", "description": "A complete implementation assignment, not an investigation."},
                            "paths": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Exact repository-relative files this writer alone may change."},
                            "dependencies": {"type": "array", "items": {"type": "string"}, "description": "Other workstream ids that must finish first. Omit for parallel work."}
                        },
                        "required": ["id", "objective", "paths"],
                        "additionalProperties": false
                    }
                },
                "independent_review": {"type": "boolean", "description": "Add a separate read-only model review and bounded targeted rework gate."}
            },
            "required": ["objective", "integration_checks", "workstreams"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn mutating(&self) -> bool {
        true
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        let parsed: DelegateArgs = serde_json::from_value(args.clone()).ok()?;
        let workstreams = parsed
            .workstreams
            .iter()
            .map(|workstream| format!("{}: {:?}", workstream.id, workstream.paths))
            .collect::<Vec<_>>()
            .join("\n");
        let checks = parsed
            .integration_checks
            .iter()
            .map(|check| format!("{}: {:?}", check.id, check.argv))
            .collect::<Vec<_>>()
            .join("\n");
        let resources = parsed
            .resources
            .iter()
            .map(|resource| format!("{}: {}", resource.id, resource.command))
            .collect::<Vec<_>>()
            .join("\n");
        Some(format!(
            "Run isolated coding agents; the primary checkout remains unchanged.\nWorkstreams:\n{workstreams}\nEnvironment commands:\n{resources}\nIntegration commands:\n{checks}"
        ))
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: DelegateArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::error(format!("invalid coding delegation: {error}")),
        };
        if !ctx.executor.is_local() {
            return ToolOutcome::error(
                "isolated coding workstreams currently require a local Git checkout",
            );
        }
        if !self.shared.pending.lock().expect("pending lock").is_empty() {
            return ToolOutcome::error(
                "apply or discard the existing coding workstream result before starting another",
            );
        }
        match run_workstreams(&self.shared, args, ctx).await {
            Ok(outcome) => outcome,
            Err(error) => ToolOutcome::error(error),
        }
    }
}

async fn run_workstreams(
    shared: &Arc<SharedState>,
    mut args: DelegateArgs,
    ctx: &ToolCtx,
) -> Result<ToolOutcome, String> {
    if args.objective.trim().is_empty() {
        return Err("coding delegation objective must not be empty".into());
    }
    if args.workstreams.len() < 2 || args.workstreams.len() > shared.config.policy.max_agents {
        return Err(format!(
            "coding delegation needs 2 to {} workstreams",
            shared.config.policy.max_agents
        ));
    }
    let repository_id = RepositoryId::new("workspace")?;
    let allowed_changed_paths = args
        .workstreams
        .iter()
        .flat_map(|workstream| workstream.paths.iter().cloned())
        .collect::<BTreeSet<_>>();
    let selection = Arc::new(
        RepositorySelection::resolve(
            ctx.executor.as_ref(),
            vec![RepositorySelectionRequest {
                repository_id: repository_id.clone(),
                root: ctx.sandbox.root().to_path_buf(),
                allowed_changed_paths,
                cloud_eligible: false,
            }],
        )
        .await?,
    );
    if selection.repositories()[&repository_id]
        .baseline
        .checkout_root
        != ctx.sandbox.root().to_string_lossy()
    {
        return Err(
            "coding workstreams require the selected project root to be the Git checkout root"
                .into(),
        );
    }
    let resource_args = std::mem::take(&mut args.resources);
    let plan = Arc::new(build_plan(shared, args, selection.as_ref(), repository_id)?);
    selection.validate_delegated_scope(&plan)?;
    if !plan.decomposition_decision()?.delegated {
        return Err(
            "workstream dependencies do not contain two parallel writers; continue single-agent"
                .into(),
        );
    }

    let scratch_root = std::env::temp_dir()
        .join("clark-orchestration")
        .join(Uuid::new_v4().to_string());
    let artifact_root = scratch_root.join("artifacts");
    let provider_config = ProviderConfig {
        cwd: Some(ctx.sandbox.root().to_string_lossy().into_owned()),
        auth_token: shared.config.api_key.clone(),
        headers: shared.config.headers.clone(),
        extra: json!({
            "base_url": shared.config.base_url,
            "model": shared.config.root_model,
            "reasoning_effort": shared.config.reasoning_effort,
            "temperature": 0.0,
            "max_iterations": 96
        }),
        ..Default::default()
    };
    let factory: Arc<dyn ProviderFactory> =
        Arc::new(|| Box::new(LocalAgentProvider::new()) as Box<dyn Provider>);
    let (integration_gate, readiness_sender) =
        resources::readiness_channel(!resource_args.is_empty());
    let runtime = LocalMultiRepoRuntime::new(
        LocalMultiRepoRuntimeConfig {
            provider_config,
            timeout: WRITER_TIMEOUT,
            scratch_root: scratch_root.clone(),
            artifact_root,
            selection: selection.clone(),
            plan: plan.clone(),
            integration_gate,
        },
        factory,
        ctx.executor.clone(),
    )?;
    let integrator = Arc::new(runtime.integration_harness("isolated-integrator")?);
    let mut coordinator = MultiRepoCoordinator::new(
        (*plan).clone(),
        SharedBudget::new(BudgetConfig {
            limit_weighted_tokens: shared.config.policy.token_budget,
            ..Default::default()
        })?,
        shared.config.policy.max_attempts,
        integrator,
    )?;
    coordinator.register_writer(Arc::new(runtime.writer_harness("isolated-writer")?))?;
    if plan.requires_independent_review {
        coordinator.register_reviewer(Arc::new(runtime.reviewer_harness("isolated-reviewer")?))?;
    }
    let captured = Arc::new(Mutex::new(Vec::<MultiRepoCoordinatorEvent>::new()));
    let event_capture = captured.clone();
    let progress = ctx.progress.clone();
    let agent_progress = ctx.agent_progress.clone();
    if let Some(agent_progress) = &agent_progress {
        for task in &plan.tasks {
            agent_progress(FanOutAgent {
                id: task.id.to_string(),
                label: task_label(task),
                status: FanOutStatus::Queued,
            });
        }
    }
    let events = Arc::new(move |event: MultiRepoCoordinatorEvent| {
        if let Some(progress) = &progress {
            progress(format!("{:?}\n", event));
        }
        if let Some(agent_progress) = &agent_progress {
            match &event {
                MultiRepoCoordinatorEvent::TaskStarted { task_id, .. } => {
                    agent_progress(FanOutAgent {
                        id: task_id.to_string(),
                        label: String::new(),
                        status: FanOutStatus::Running,
                    });
                }
                MultiRepoCoordinatorEvent::TaskFinished {
                    task_id, outcome, ..
                } => {
                    agent_progress(FanOutAgent {
                        id: task_id.to_string(),
                        label: String::new(),
                        status: if *outcome == TaskRunOutcome::Completed {
                            FanOutStatus::Done
                        } else {
                            FanOutStatus::Failed
                        },
                    });
                }
                MultiRepoCoordinatorEvent::RecoveryScheduled {
                    failed_task_id,
                    replacement_task_id,
                } => {
                    agent_progress(FanOutAgent {
                        id: failed_task_id.to_string(),
                        label: String::new(),
                        status: FanOutStatus::Failed,
                    });
                    agent_progress(FanOutAgent {
                        id: replacement_task_id.to_string(),
                        label: "Retry failed work safely".into(),
                        status: FanOutStatus::Queued,
                    });
                }
                _ => {}
            }
        }
        event_capture.lock().expect("event lock").push(event);
    });
    let started_resources = match resources::start(&resource_args, ctx).await {
        Ok(resources) => resources,
        Err(error) => {
            let _ = ctx.executor.remove_dir_all(&scratch_root).await;
            return Err(error);
        }
    };
    let (result, resource_result) = if let Some(sender) = readiness_sender {
        tokio::join!(
            coordinator.run(ctx.cancel.child_token(), events),
            resources::supervise(&started_resources, ctx, sender)
        )
    } else {
        (
            coordinator.run(ctx.cancel.child_token(), events).await,
            Ok(Vec::new()),
        )
    };
    let mut resource_receipts = resource_result;
    resources::release(&started_resources, ctx, resource_receipts.as_mut().ok()).await;
    let result = result?;
    let resource_receipts = resource_receipts?;
    if let Some(execution) = ctx.session.lock().await.active_execution.clone() {
        execution.record_child_budget(result.budget.weighted_tokens_used, result.budget.cost_usd);
    }
    if !result.passed() {
        let _ = ctx.executor.remove_dir_all(&scratch_root).await;
        return Err(format!(
            "isolated coding workstreams did not pass fresh integration: {}",
            result
                .error
                .as_deref()
                .unwrap_or("a required receipt was missing")
        ));
    }
    let run_id = format!("coding-{}", Uuid::new_v4());
    shared.pending.lock().expect("pending lock").insert(
        run_id.clone(),
        PendingRun {
            selection,
            plan,
            result: result.clone(),
            scratch_root,
            resources: resource_receipts.clone(),
        },
    );
    Ok(ToolOutcome::ok(
        serde_json::to_string_pretty(&json!({
            "run_id": run_id,
            "status": "verified_pending_resolution",
            "primary_checkout_changed": false,
            "change_packages": result.change_packages,
            "integration": result.integration,
            "budget": result.budget,
            "resources": resource_receipts,
            "next": "Call resolve_coding_workstreams with action apply or discard."
        }))
        .map_err(|error| error.to_string())?,
    )
    .with_details(json!({
        "run": result,
        "resources": resource_receipts,
        "events": captured.lock().expect("event lock").clone()
    })))
}

fn task_label(task: &MultiRepoTask) -> String {
    match task.role {
        MultiRepoTaskRole::Planner => "Plan the work".into(),
        MultiRepoTaskRole::Reviewer => "Review the completed changes".into(),
        MultiRepoTaskRole::Integrator => "Combine and verify the result".into(),
        MultiRepoTaskRole::Reader | MultiRepoTaskRole::Writer => task
            .objective
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("Complete an independent part")
            .trim()
            .to_string(),
    }
}

fn build_plan(
    shared: &SharedState,
    args: DelegateArgs,
    selection: &RepositorySelection,
    repository_id: RepositoryId,
) -> Result<MultiRepoPlan, String> {
    let planner = TaskId::new("planner")?;
    let writer_ids = args
        .workstreams
        .iter()
        .map(|workstream| TaskId::new(&workstream.id))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if writer_ids.len() != args.workstreams.len() {
        return Err("coding workstream ids must be unique".into());
    }
    let writer_count = args.workstreams.len() as u64;
    let lifecycle_minimum = 1 + u64::from(args.independent_review);
    let minimum_budget = writer_count + lifecycle_minimum;
    if shared.config.policy.token_budget < minimum_budget {
        return Err(format!(
            "coding workstream budget must be at least {minimum_budget} weighted tokens"
        ));
    }
    let lifecycle_headroom = (shared.config.policy.token_budget / 10)
        .max(lifecycle_minimum)
        .min(shared.config.policy.token_budget - writer_count);
    let writer_pool = shared.config.policy.token_budget - lifecycle_headroom;
    let reservation = (writer_pool / writer_count).max(1);
    let mut tasks = vec![global_task(
        planner.clone(),
        MultiRepoTaskRole::Planner,
        BTreeSet::new(),
        args.objective.clone(),
        "host-planner",
        &shared.config.root_model,
        ModelTier::Strong,
    )];
    for workstream in args.workstreams {
        let id = TaskId::new(workstream.id)?;
        let mut dependencies = workstream
            .dependencies
            .into_iter()
            .map(TaskId::new)
            .collect::<Result<BTreeSet<_>, _>>()?;
        dependencies.insert(planner.clone());
        tasks.push(MultiRepoTask {
            id,
            role: MultiRepoTaskRole::Writer,
            repository_id: Some(repository_id.clone()),
            dependencies,
            objective: format!(
                "{}\n\nOverall outcome: {}",
                workstream.objective, args.objective
            ),
            harness: "isolated-writer".into(),
            harness_kind: HarnessKind::Local,
            model: shared.config.root_model.clone(),
            model_tier: ModelTier::Strong,
            budget_reservation: reservation,
            allowed_changed_paths: workstream.paths,
        });
    }
    let integration_dependencies = if args.independent_review {
        tasks.push(global_task(
            TaskId::new("reviewer")?,
            MultiRepoTaskRole::Reviewer,
            writer_ids.clone(),
            format!("Independently review: {}", args.objective),
            "isolated-reviewer",
            &shared.config.root_model,
            ModelTier::Reviewer,
        ));
        BTreeSet::from([TaskId::new("reviewer")?])
    } else {
        writer_ids
    };
    tasks.push(global_task(
        TaskId::new("integrator")?,
        MultiRepoTaskRole::Integrator,
        integration_dependencies,
        format!("Freshly replay and verify: {}", args.objective),
        "isolated-integrator",
        &shared.config.root_model,
        ModelTier::Strong,
    ));
    let plan = MultiRepoPlan {
        repositories: selection.baselines(),
        contracts: Vec::new(),
        contract_decisions: Vec::new(),
        tasks,
        integration_checks: args
            .integration_checks
            .into_iter()
            .map(|check| IntegrationCheck {
                id: check.id,
                repository_id: repository_id.clone(),
                argv: check.argv,
                timeout_ms: check.timeout_ms,
            })
            .collect(),
        max_parallel_writers: shared.config.policy.max_agents,
        requires_independent_review: args.independent_review,
    };
    plan.validate()?;
    Ok(plan)
}

fn global_task(
    id: TaskId,
    role: MultiRepoTaskRole,
    dependencies: BTreeSet<TaskId>,
    objective: String,
    harness: &str,
    model: &str,
    model_tier: ModelTier,
) -> MultiRepoTask {
    MultiRepoTask {
        id,
        role,
        repository_id: None,
        dependencies,
        objective,
        harness: harness.into(),
        harness_kind: HarnessKind::Local,
        model: model.into(),
        model_tier,
        budget_reservation: 1,
        allowed_changed_paths: BTreeSet::new(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResolutionAction {
    Apply,
    Discard,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveArgs {
    run_id: String,
    action: ResolutionAction,
}

struct ResolveCodingWorkstreams {
    shared: Arc<SharedState>,
}

#[async_trait]
impl ToolExecutor for ResolveCodingWorkstreams {
    fn name(&self) -> &str {
        "resolve_coding_workstreams"
    }

    fn description(&self) -> &str {
        "Resolve one freshly verified isolated coding run. Apply rechecks the original HEAD and dirty-tree fingerprint, rejects overlap with pre-existing work, preflights every patch, then applies the exact hashed package set. Discard deletes only temporary orchestration data."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {"type": "string"},
                "action": {"type": "string", "enum": ["apply", "discard"]}
            },
            "required": ["run_id", "action"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn mutating(&self) -> bool {
        true
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        let args: ResolveArgs = serde_json::from_value(args.clone()).ok()?;
        let action = match args.action {
            ResolutionAction::Apply => "Apply the verified patches to the primary checkout",
            ResolutionAction::Discard => {
                "Discard the isolated patches without changing the primary checkout"
            }
        };
        Some(format!("{action}.\nRun: {}", args.run_id))
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ResolveArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::error(format!("invalid resolution: {error}")),
        };
        let pending = self
            .shared
            .pending
            .lock()
            .expect("pending lock")
            .get(&args.run_id)
            .cloned();
        let Some(pending) = pending else {
            return ToolOutcome::error(format!("no pending coding run `{}`", args.run_id));
        };
        let outcome = match args.action {
            ResolutionAction::Discard => ToolOutcome::ok(format!(
                "Discarded `{}`. The primary checkout was not changed.",
                args.run_id
            )),
            ResolutionAction::Apply => match pending
                .selection
                .apply_verified_packages(
                    ctx.executor.as_ref(),
                    &pending.plan,
                    &pending.result.change_packages,
                    &pending.scratch_root,
                )
                .await
            {
                Ok(receipt) => ToolOutcome::ok(
                    serde_json::to_string_pretty(&receipt)
                        .unwrap_or_else(|_| "verified changes applied".into()),
                )
                .with_details(json!({
                    "application": receipt,
                    "run": pending.result,
                    "resources": pending.resources
                })),
                Err(error) => return ToolOutcome::error(error),
            },
        };
        self.shared
            .pending
            .lock()
            .expect("pending lock")
            .remove(&args.run_id);
        let _ = ctx.executor.remove_dir_all(&pending.scratch_root).await;
        outcome
    }
}

#[cfg(test)]
#[path = "orchestration_writer_tool_tests.rs"]
mod tests;
