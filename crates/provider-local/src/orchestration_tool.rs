use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_core::domain::ToolKind;
use agent_core::provider::ProviderConfig;
use agent_orchestration::{
    AdmissionPolicy, AdmissionRequest, AgentPath, Authorization, BudgetConfig, ControlPlane,
    Coordinator, FanOutRequest, HarnessKind, ModelRate, OrchestrationId, OrchestrationPurpose,
    ReadOnlyEnforcement, ReadOnlyTask, ReportStatus, RiskSignals, SharedBudget, TaskId,
    WorkstreamEstimate,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::orchestration::{
    local_read_only_harness, AcpHarnessConfig, OrchestrationToolsConfig, WorkspaceDigestGuard,
};
use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};

#[path = "orchestration_tool_resolution.rs"]
mod resolution;
#[path = "orchestration_tool_schema.rs"]
mod schema;
#[path = "orchestration_scout/mod.rs"]
mod scout;
#[path = "orchestration_tool_support.rs"]
mod support;
#[path = "orchestration_writer_tool.rs"]
mod writer;

use schema::delegate_schema;
use support::{event_sink, role_for_purpose};

const ACP_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const LOCAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, PartialEq, Eq)]
struct DelegationModelPolicy {
    root_model: String,
    child_model: String,
    harness: String,
    reasoning_effort: Option<String>,
}

fn delegation_model_policy(
    config: &OrchestrationToolsConfig,
    model_override: Option<crate::tools::TurnModelOverride>,
) -> DelegationModelPolicy {
    match model_override {
        Some(policy) => DelegationModelPolicy {
            root_model: policy.model.clone(),
            child_model: policy.model,
            harness: "local".to_string(),
            reasoning_effort: policy.reasoning_effort,
        },
        None => DelegationModelPolicy {
            root_model: config.root_model.clone(),
            child_model: config
                .policy
                .subagent_model
                .clone()
                .unwrap_or_else(|| config.root_model.clone()),
            harness: config.policy.read_only_harness.clone(),
            reasoning_effort: config.reasoning_effort.clone(),
        },
    }
}

pub(super) struct StoredOrchestration {
    pub coordinator: Arc<Coordinator>,
    pub parent_context: String,
}

pub(super) struct SharedState {
    pub config: OrchestrationToolsConfig,
    pub orchestrations: Mutex<HashMap<String, StoredOrchestration>>,
    execution_slots: Arc<Semaphore>,
}

pub(crate) fn orchestration_tools(config: OrchestrationToolsConfig) -> Vec<Arc<dyn ToolExecutor>> {
    let writer_config = config.clone();
    let shared = Arc::new(SharedState {
        execution_slots: Arc::new(Semaphore::new(config.policy.max_agents)),
        config,
        orchestrations: Mutex::new(HashMap::new()),
    });
    let mut tools: Vec<Arc<dyn ToolExecutor>> = vec![
        Arc::new(DelegateReadOnly {
            shared: shared.clone(),
        }),
        resolution::tool(shared.clone()),
    ];
    tools.extend(writer::tools(writer_config));
    tools.extend(scout::tools(
        shared.config.scout_capsules.clone(),
        shared.config.clone(),
    ));
    tools
}

struct DelegateReadOnly {
    shared: Arc<SharedState>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateArgs {
    objective: String,
    purpose: OrchestrationPurpose,
    workstreams: Vec<WorkstreamArgs>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkstreamArgs {
    id: String,
    objective: String,
    scopes: BTreeSet<String>,
    acceptance: Vec<String>,
}

fn default_output_tokens() -> u64 {
    2_000
}

#[async_trait]
impl ToolExecutor for DelegateReadOnly {
    fn name(&self) -> &str {
        "delegate_read_only"
    }

    fn description(&self) -> &str {
        "Fan out clearly independent, high-context repository investigation to bounded read-only agents. Use only when the user or repository instructions explicitly authorize delegation. The root remains the sole writer. Review/verification is admitted only for concrete risk signals. Returns reported results that must be explicitly accepted or sent for rework with resolve_delegation."
    }

    fn parameters(&self) -> Value {
        delegate_schema()
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: DelegateArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => {
                return ToolOutcome::error(format!("invalid delegation request: {error}"))
            }
        };
        if args.purpose == OrchestrationPurpose::ExternalResearch {
            return ToolOutcome::error(
                "use an installed brokered research capability for external research; coding fan-out cannot perform it",
            );
        }
        if !self
            .shared
            .orchestrations
            .lock()
            .expect("orchestration lock")
            .is_empty()
        {
            return ToolOutcome::error(
                "resolve the existing delegation before starting another fan-out",
            );
        }
        let slot_count = match u32::try_from(args.workstreams.len()) {
            Ok(0) | Err(_) => return ToolOutcome::error("workstreams must not be empty"),
            Ok(count) => count,
        };
        let _execution_slots = match self
            .shared
            .execution_slots
            .clone()
            .try_acquire_many_owned(slot_count)
        {
            Ok(permit) => permit,
            Err(_) => return ToolOutcome::error("read-only agent execution limit reached"),
        };
        match run_delegation(&self.shared, args, ctx).await {
            Ok(outcome) => outcome,
            Err(error) => ToolOutcome::error(error),
        }
    }
}

async fn run_delegation(
    shared: &Arc<SharedState>,
    args: DelegateArgs,
    ctx: &ToolCtx,
) -> Result<ToolOutcome, String> {
    let entries = ctx.executor.walk(ctx.sandbox.root()).await?;
    let role = role_for_purpose(args.purpose)?;
    let risk = RiskSignals {
        // Selecting a gated review/verify purpose is the structured trigger;
        // cost, harness, and the rest of the gate policy remain host-owned.
        user_requested_review: matches!(
            args.purpose,
            OrchestrationPurpose::Review | OrchestrationPurpose::Verify
        ),
        ..RiskSignals::default()
    };
    let model_policy = delegation_model_policy(&shared.config, ctx.model_override.clone());
    let mut tasks = Vec::with_capacity(args.workstreams.len());
    let mut estimates = Vec::with_capacity(args.workstreams.len());
    for workstream in args.workstreams {
        let task_id = TaskId::new(workstream.id)?;
        let scopes = resolve_scopes(ctx, &workstream.scopes)?;
        let context_tokens = estimate_context_tokens(&entries, &scopes);
        let harness = &model_policy.harness;
        let (harness_kind, model, rate) =
            harness_metadata(shared, harness, &model_policy.child_model)?;
        estimates.push(WorkstreamEstimate {
            task_id: task_id.clone(),
            scopes: workstream.scopes.clone(),
            estimated_context_tokens: context_tokens,
            estimated_output_tokens: default_output_tokens(),
            harness_kind,
            model,
            model_rate: rate,
        });
        tasks.push(ReadOnlyTask {
            id: task_id,
            role,
            objective: workstream.objective,
            scopes: workstream.scopes,
            acceptance: workstream.acceptance,
            harness: harness.clone(),
        });
    }
    let orchestration_id = OrchestrationId::new(format!("fanout-{}", Uuid::new_v4()))?;
    let admission = AdmissionRequest {
        // The model cannot widen this typed value through tool arguments. The
        // host-injected explicit-request-only policy controls when the model may
        // call the default-available tool; admission still owns cost and scope.
        authorization: Authorization::UserRequested,
        purpose: args.purpose,
        workstreams: estimates,
        root_model: model_policy.root_model.clone(),
        root_model_rate: shared.config.policy.root_model_rate,
        root_estimated_output_tokens: default_output_tokens(),
        risk,
        external_research_required: false,
    };
    let policy = AdmissionPolicy {
        max_agents: shared.config.policy.max_agents,
        minimum_parallel_context_tokens: shared.config.policy.minimum_context_tokens,
        child_system_prompt_tokens: shared.config.policy.child_system_prompt_tokens,
        max_projected_cost_ratio: shared.config.policy.max_projected_cost_ratio,
        max_projected_weighted_tokens: Some(shared.config.policy.token_budget as f64),
        output_token_weight: 4.0,
        require_explicit_authorization: true,
    };
    let preview = policy.evaluate(&admission);
    if !preview.admitted {
        return Err(format!(
            "fan-out admission rejected: {}",
            preview
                .rejections
                .iter()
                .map(|rejection| format!("{}: {}", rejection.code, rejection.detail))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let budget = SharedBudget::new(BudgetConfig {
        limit_weighted_tokens: shared.config.policy.token_budget,
        ..Default::default()
    })?;
    let control = ControlPlane::new(shared.config.policy.max_agents, 1, budget)?;
    let mut coordinator = Coordinator::new(policy, control);
    register_harnesses(
        &mut coordinator,
        shared,
        ctx,
        &model_policy.child_model,
        model_policy.reasoning_effort.as_deref(),
    )?;
    let coordinator = Arc::new(coordinator);
    let root_execution = ctx.session.lock().await.active_execution.clone();
    if let Some(execution) = &root_execution {
        for task in &tasks {
            execution.attach_child(AgentPath::root().child(&task.id)?, task.role);
        }
    }
    let (event_sink, captured) = event_sink(ctx, root_execution.clone());
    let result = coordinator
        .run_fanout(
            FanOutRequest {
                id: orchestration_id.clone(),
                admission,
                tasks,
                parent_context: args.objective.clone(),
            },
            event_sink,
        )
        .await
        .map_err(|error| error.to_string())?;
    if let Some(execution) = &root_execution {
        execution.record_child_budget(
            result.control.budget.weighted_tokens_used,
            result.control.budget.cost_usd,
        );
    }
    let needs_resolution = result
        .control
        .agents
        .values()
        .any(|agent| agent.report_status == ReportStatus::Reported);
    if needs_resolution {
        shared
            .orchestrations
            .lock()
            .expect("orchestration lock")
            .insert(
                orchestration_id.0.clone(),
                StoredOrchestration {
                    coordinator,
                    parent_context: args.objective,
                },
            );
    }
    let next = if needs_resolution {
        "Call resolve_delegation for every reported task: accept sound evidence or request bounded rework."
    } else {
        "No task produced a valid report; inspect the failure state and continue single-agent."
    };
    let content = serde_json::to_string_pretty(&json!({
        "orchestration_id": orchestration_id,
        "state": result.control,
        "next": next
    }))
    .map_err(|error| error.to_string())?;
    Ok(ToolOutcome::ok(content).with_details(json!({
        "orchestration": result,
        "events": captured.lock().expect("event lock").clone()
    })))
}

fn resolve_scopes(ctx: &ToolCtx, scopes: &BTreeSet<String>) -> Result<Vec<PathBuf>, String> {
    if scopes.is_empty() {
        return Err("every workstream needs at least one scope".to_string());
    }
    scopes
        .iter()
        .map(|scope| ctx.sandbox.resolve_existing(scope))
        .collect()
}

fn estimate_context_tokens(entries: &[exec_core::WalkEntry], scopes: &[PathBuf]) -> u64 {
    let bytes = entries
        .iter()
        .filter(|entry| scopes.iter().any(|scope| entry.path.starts_with(scope)))
        .map(|entry| entry.len)
        .sum::<u64>();
    bytes.saturating_add(3) / 4
}

fn harness_metadata(
    shared: &SharedState,
    harness: &str,
    local_model: &str,
) -> Result<(HarnessKind, String, Option<ModelRate>), String> {
    if harness == "local" {
        return Ok((
            HarnessKind::Local,
            local_model.to_string(),
            shared.config.policy.subagent_model_rate,
        ));
    }
    let acp = shared
        .config
        .policy
        .acp_harnesses
        .iter()
        .find(|candidate| candidate.id == harness)
        .ok_or_else(|| format!("unknown harness: {harness}"))?;
    Ok((HarnessKind::Acp, acp.model.clone(), acp.model_rate))
}

fn register_harnesses(
    coordinator: &mut Coordinator,
    shared: &SharedState,
    ctx: &ToolCtx,
    local_model: &str,
    reasoning_effort: Option<&str>,
) -> Result<(), String> {
    let root = ctx.sandbox.root().to_string_lossy().into_owned();
    let workspace = Arc::new(WorkspaceDigestGuard::new(
        ctx.sandbox.root(),
        ctx.executor.clone(),
    ));
    let mut extra = Map::new();
    extra.insert("base_url".to_string(), json!(shared.config.base_url));
    extra.insert("model".to_string(), json!(local_model));
    extra.insert("reasoning_effort".to_string(), json!(reasoning_effort));
    extra.insert("temperature".to_string(), json!(0.0));
    let extra = Value::Object(extra);
    coordinator.register_harness(Arc::new(local_read_only_harness(
        agent_orchestration::ProviderHarnessConfig {
            id: "local".to_string(),
            kind: HarnessKind::Local,
            provider: "local".to_string(),
            model: local_model.to_string(),
            provider_config: ProviderConfig {
                cwd: Some(root.clone()),
                auth_token: shared.config.api_key.clone(),
                headers: shared.config.headers.clone(),
                extra,
                ..Default::default()
            },
            cwd: root.clone(),
            timeout: LOCAL_TIMEOUT,
            enforcement: ReadOnlyEnforcement::HostToolGate,
        },
        workspace.clone(),
    )?))?;
    for acp in &shared.config.policy.acp_harnesses {
        coordinator.register_harness(Arc::new(acp_harness(acp, &root, workspace.clone())?))?;
    }
    Ok(())
}

fn acp_harness(
    acp: &AcpHarnessConfig,
    root: &str,
    workspace: Arc<dyn agent_orchestration::WorkspaceGuard>,
) -> Result<agent_orchestration::ProviderHarness, String> {
    let command = os_read_only_command(&acp.command, root)?;
    provider_acp::read_only_harness(
        agent_orchestration::ProviderHarnessConfig {
            id: acp.id.clone(),
            kind: HarnessKind::Acp,
            provider: "acp".to_string(),
            model: acp.model.clone(),
            provider_config: ProviderConfig {
                command: Some(command),
                cwd: Some(root.to_string()),
                ..Default::default()
            },
            cwd: root.to_string(),
            timeout: ACP_TIMEOUT,
            enforcement: acp.enforcement,
        },
        workspace,
    )
}

fn os_read_only_command(command: &[String], root: &str) -> Result<Vec<String>, String> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| "ACP sandbox command is empty".to_string())?;
    let policy = exec_sandbox::SandboxPolicy::read_only();
    let manager = exec_sandbox::SandboxManager::current(policy)?;
    let process = manager
        .prepare_process(exec_core::ProcessSpec::argv(program, root).args(args.iter().cloned()))?;
    let mut wrapped = vec![process.program.to_string_lossy().into_owned()];
    wrapped.extend(
        process
            .args
            .into_iter()
            .map(|part| part.to_string_lossy().into_owned()),
    );
    Ok(wrapped)
}

#[cfg(test)]
#[path = "orchestration_tool_tests.rs"]
mod tests;
