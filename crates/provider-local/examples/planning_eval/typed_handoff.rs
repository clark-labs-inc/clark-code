use crate::context::{
    context_packet, direct_context_packet, seed_project_memory, select_evidence, sha256,
};
use crate::gateway::Gateway;
use crate::model::{
    CaseRecord, HandoffMode, KnowledgeDelivery, Lane, PlanOrigin, RouteReceipt, Scenario,
    TrajectoryReceipt, UsageReceipt,
};
use crate::plan_bank::PlanBankEntry;
use crate::retrieval::retrieval_treatment;
use crate::retry::{retryable_error, wait_with_progress, PHASE_DELAYS};
use crate::route::LiveConfig;
use crate::runner::{
    build_planner_prompt, connect_provider, drive_connected, handoff_receipt, oracle_proposal,
    phase_retry, snapshot_files, tree_digest, ConnectedProvider, DriveOptions, ProviderRun,
};
use agent_core::domain::{ProposedPlan, ProposedPlanStatus};
use agent_core::provider::{
    ClientResponse, CollaborationMode, PlanDecision, PlanImplementationContext, Provider,
    ResumeItem, ResumeTranscript,
};

struct PreparedExecution {
    workspace: tempfile::TempDir,
    gateway: Gateway,
    connected: ConnectedProvider,
    baseline_sha256: String,
    receipt_offset: usize,
    reused_planner_provider: bool,
    reused_planner_session: bool,
}

pub async fn run_live_typed_case(
    run_id: &str,
    scenario: &Scenario,
    lane: &Lane,
    repetition: usize,
    route: &RouteReceipt,
    config: &LiveConfig,
    bank_entry: Option<&PlanBankEntry>,
) -> Result<CaseRecord, String> {
    let planner_evidence = select_evidence(scenario, &lane.planner_sources);
    let (planner_packet, mut planner_context) =
        if lane.knowledge_delivery() == KnowledgeDelivery::PrefetchedCapsule {
            crate::context::prefetched_planner_packet(&planner_evidence)
        } else {
            context_packet(&planner_evidence)
        };
    let task_prompt = bank_entry
        .map(|entry| entry.task_prompt.clone())
        .unwrap_or_else(|| {
            build_planner_prompt(scenario, &planner_packet, lane.knowledge_delivery())
        });
    let planning_contract = bank_entry
        .map(|entry| entry.planning_contract.clone())
        .unwrap_or_else(|| provider_local::planning_prompt_contract_for_eval(&config.profile));
    let fixture = tempfile::tempdir().map_err(|error| error.to_string())?;
    (scenario.seed)(fixture.path())?;
    let fixture_sha256 = tree_digest(fixture.path())?;
    let mut retries = Vec::new();

    let mut retained_planner = None;
    let mut planner_run = empty_run();
    if matches!(lane.plan_origin, PlanOrigin::BankNone | PlanOrigin::BankAll) {
        let entry = bank_entry.ok_or("bank-backed typed lane is missing its frozen plan")?;
        planner_context = entry.planner_context.clone();
        planner_run.proposal = Some(entry.proposal.clone());
        planner_run.trajectory = entry.planner_trajectory.clone();
    }
    if lane.plan_origin == PlanOrigin::Generated {
        for attempt in 1..=3 {
            let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
            (scenario.seed)(workspace.path())?;
            seed_project_memory(workspace.path(), &planner_evidence)?;
            let phase_before = tree_digest(workspace.path())?;
            let gateway =
                Gateway::start(&config.base_url, &config.api_key, &planner_evidence).await?;
            let options = DriveOptions {
                mode: Some(CollaborationMode::Plan),
                // The real product connects with the eventual execution policy.
                // Plan Mode's read-only executor, not a benchmark-only deny policy,
                // is what must prevent writes during planning.
                writable: true,
                memories: true,
                base_url: &gateway.base_url,
                planner_tools: true,
                preactivated_tools: Vec::new(),
            };
            let mut connected = connect_provider(workspace.path(), &options, config, None).await?;
            planner_run = drive_connected(&mut connected, &task_prompt, false).await?;
            let mutated = phase_before != tree_digest(workspace.path())?;
            let retry_reason = planner_run.error.as_deref().filter(|error| {
                retryable_error(error) && planner_run.proposal.is_none() && !mutated && attempt < 3
            });
            if let Some(reason) = retry_reason {
                let delay = PHASE_DELAYS[attempt - 1];
                let waited = wait_with_progress("planner_phase", delay).await;
                retries.push(phase_retry(
                    "planner", attempt, reason, delay, waited, false, mutated,
                ));
                continue;
            }
            planner_context.retrievals.extend(gateway.receipts());
            retained_planner = Some((workspace, gateway, connected, phase_before));
            break;
        }
    }

    let proposal = match lane.plan_origin {
        PlanOrigin::Generated => planner_run
            .proposal
            .clone()
            .ok_or("typed handoff requires a proposed plan")?,
        PlanOrigin::Oracle => oracle_proposal(scenario),
        PlanOrigin::BankNone | PlanOrigin::BankAll => bank_entry
            .ok_or("bank-backed typed lane is missing its frozen plan")?
            .proposal
            .clone(),
        PlanOrigin::None => return Err("typed handoff requires a plan origin".into()),
    };
    let plan = proposal.markdown.clone();
    let executor_evidence = select_evidence(scenario, &lane.executor_sources);
    let (_, mut executor_context) = direct_context_packet(&executor_evidence);
    let executor_prompt = "Implement the approved plan.";

    let mut initial_execution = match lane.handoff {
        HandoffMode::TypedCurrent | HandoffMode::TypedFresh => {
            let (workspace, gateway, mut connected, baseline_sha256) =
                retained_planner.take().ok_or("planner workspace missing")?;
            let receipt_offset = gateway.receipts().len();
            approve(
                &mut connected,
                &proposal,
                if lane.handoff == HandoffMode::TypedCurrent {
                    PlanImplementationContext::Current
                } else {
                    PlanImplementationContext::Fresh
                },
            )
            .await?;
            Some(PreparedExecution {
                workspace,
                gateway,
                connected,
                baseline_sha256,
                receipt_offset,
                reused_planner_provider: true,
                reused_planner_session: true,
            })
        }
        HandoffMode::TypedReplayFresh => {
            drop(retained_planner.take());
            Some(prepare_replay(scenario, &proposal, &executor_evidence, config).await?)
        }
        _ => return Err("non-typed lane reached typed handoff runner".into()),
    };

    let mut executor_run = empty_run();
    let mut retained_executor = None;
    for attempt in 1..=3 {
        let mut execution = match initial_execution.take() {
            Some(execution) => execution,
            None => prepare_replay(scenario, &proposal, &executor_evidence, config).await?,
        };
        executor_run = drive_connected(&mut execution.connected, executor_prompt, true).await?;
        let mutated = execution.baseline_sha256 != tree_digest(execution.workspace.path())?;
        let retry_reason = executor_run.error.as_deref().filter(|error| {
            retryable_error(error) && executor_run.usage.output_tokens == 0 && attempt < 3
        });
        if let Some(reason) = retry_reason {
            let delay = PHASE_DELAYS[attempt - 1];
            let waited = wait_with_progress("executor_phase", delay).await;
            retries.push(phase_retry(
                "executor", attempt, reason, delay, waited, false, mutated,
            ));
            continue;
        }
        let receipts = execution.gateway.receipts();
        executor_context
            .retrievals
            .extend(receipts.into_iter().skip(execution.receipt_offset));
        retained_executor = Some(execution);
        break;
    }

    let executor = retained_executor.ok_or("typed executor exhausted clean retries")?;
    let verification = (scenario.verify)(executor.workspace.path());
    let retrieval_treatment = retrieval_treatment(lane, &planner_context, &planner_run.trajectory);
    let mut handoff = handoff_receipt(
        lane.handoff,
        Some(proposal.id.clone()),
        Some(proposal.revision),
        Some(&proposal.markdown),
        Some(&provider_local::complete_plan_markdown_for_eval(
            &proposal.markdown,
        )),
        true,
        executor.reused_planner_provider,
        executor.reused_planner_session,
    );
    handoff.plan_bank_id = bank_entry.map(PlanBankEntry::bank_id);
    let executor_tree_sha256 = tree_digest(executor.workspace.path())?;
    let error = planner_run.error.clone().or(executor_run.error.clone());
    Ok(CaseRecord {
        schema_version: 5,
        run_id: run_id.into(),
        mode: "live".into(),
        scenario: scenario.id.into(),
        lane: lane.id.clone(),
        repetition,
        profile: config.profile.clone(),
        route: route.clone(),
        fixture_sha256,
        planning_contract: planning_contract.clone(),
        planning_prompt_sha256: sha256(&planning_contract),
        task_prompt: task_prompt.clone(),
        task_prompt_sha256: sha256(&task_prompt),
        executor_prompt: executor_prompt.into(),
        executor_prompt_sha256: sha256(executor_prompt),
        handoff,
        planner_context,
        executor_context,
        plan: Some(plan),
        retrieval_treatment,
        planner_usage: planner_run.usage,
        executor_usage: executor_run.usage,
        planner_trajectory: planner_run.trajectory,
        executor_trajectory: executor_run.trajectory,
        verification,
        executor_tree_sha256,
        executor_files: snapshot_files(executor.workspace.path())?,
        retries,
        error,
    })
}

async fn prepare_replay(
    scenario: &Scenario,
    proposal: &ProposedPlan,
    executor_evidence: &[&crate::model::Evidence],
    config: &LiveConfig,
) -> Result<PreparedExecution, String> {
    let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
    (scenario.seed)(workspace.path())?;
    seed_project_memory(workspace.path(), executor_evidence)?;
    let baseline_sha256 = tree_digest(workspace.path())?;
    let gateway = Gateway::start(&config.base_url, &config.api_key, executor_evidence).await?;
    let options = DriveOptions {
        mode: Some(CollaborationMode::Plan),
        writable: true,
        memories: true,
        base_url: &gateway.base_url,
        planner_tools: false,
        preactivated_tools: Vec::new(),
    };
    let mut replay_plan = proposal.clone();
    replay_plan.status = ProposedPlanStatus::AwaitingDecision;
    let resume = ResumeTranscript {
        items: vec![ResumeItem::ProposedPlan { plan: replay_plan }],
        truncated: false,
    };
    let mut connected = connect_provider(workspace.path(), &options, config, Some(resume)).await?;
    approve(&mut connected, proposal, PlanImplementationContext::Fresh).await?;
    Ok(PreparedExecution {
        workspace,
        gateway,
        connected,
        baseline_sha256,
        receipt_offset: 0,
        reused_planner_provider: false,
        reused_planner_session: false,
    })
}

async fn approve(
    connected: &mut ConnectedProvider,
    proposal: &ProposedPlan,
    context: PlanImplementationContext,
) -> Result<(), String> {
    connected
        .provider
        .respond(
            &connected.session.id,
            ClientResponse::PlanDecision {
                plan_id: proposal.id.clone(),
                decision: PlanDecision::Implement { context },
            },
        )
        .await
        .map_err(|error| error.to_string())
}

fn empty_run() -> ProviderRun {
    ProviderRun {
        proposal: None,
        usage: UsageReceipt::default(),
        trajectory: TrajectoryReceipt::default(),
        error: None,
    }
}
