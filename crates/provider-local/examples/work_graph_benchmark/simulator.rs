use std::cmp::max;
use std::collections::{BTreeMap, BTreeSet};

use super::model::{
    ArtifactReceipt, CandidateResult, DependencyWakeupReceipt, FaultInjection, LaneKind, LaneSpec,
    RecoveryReceipt, ResourceOutcome, ResourceReceipt, SafetyReceipt, Scenario, TaskOutcome,
    TaskReceipt, TaskRole, TraceAuthority, TraceEvent, UsageReceipt, VerificationReceipt,
};
use super::simulator_helpers::{digest, plan, simple_interaction};
use super::workspace::{DynError, SeededWorkspace};

pub fn run_reference(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
) -> Result<CandidateResult, DynError> {
    workspace.apply_solution(scenario)?;
    let mut result = match lane.kind {
        LaneKind::NaiveParallel => simulate_naive(scenario, lane, workspace),
        LaneKind::Single | LaneKind::EqualBudgetSingle => simulate_ordered(
            scenario,
            lane,
            workspace,
            false,
            TraceAuthority::HostSimulation,
        ),
        _ if !scenario.expected_delegate => simulate_ordered(
            scenario,
            lane,
            workspace,
            false,
            TraceAuthority::HostSimulation,
        ),
        _ => simulate_graph(scenario, lane, workspace),
    };
    result.candidate_id = "reference".into();
    Ok(result)
}

pub fn run_current(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
) -> Result<CandidateResult, DynError> {
    workspace.apply_solution(scenario)?;
    let mut result = simulate_ordered(
        scenario,
        lane,
        workspace,
        false,
        TraceAuthority::SelfReported,
    );
    result.candidate_id = "current-agent".into();
    result.delegation_reason =
        "Current provider has one authoritative writer and no production work-graph trace".into();
    result.plan = None;
    result.resources.clear();
    result.artifacts.clear();
    result.wakeups.clear();
    result.recoveries.clear();
    result.verification = None;
    result.events.clear();
    result.interaction = None;
    Ok(result)
}

fn simulate_graph(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
) -> CandidateResult {
    let mut resources = provision_resources(scenario);
    let resource_ready = resources
        .iter()
        .filter(|receipt| receipt.outcome == ResourceOutcome::Ready)
        .filter_map(|receipt| {
            receipt
                .ready_ms
                .map(|ready| (receipt.resource_id.clone(), ready))
        })
        .collect::<BTreeMap<_, _>>();
    let mut tasks = Vec::new();
    let mut finish_by_id = BTreeMap::new();
    let mut recoveries = Vec::new();
    let crash_target = if scenario.fault == FaultInjection::WorkerCrashAfterArtifact {
        scenario
            .tasks
            .iter()
            .rev()
            .find(|task| task.role.writes())
            .map(|task| task.id.as_str())
    } else {
        None
    };

    for spec in &scenario.tasks {
        let dependency_ready = spec
            .dependencies
            .iter()
            .filter_map(|dependency| finish_by_id.get(dependency))
            .copied()
            .max()
            .unwrap_or(40);
        let environment_ready = spec
            .resources
            .iter()
            .filter_map(|resource| resource_ready.get(resource))
            .copied()
            .max()
            .unwrap_or(40);
        let earliest_ms = max(dependency_ready, environment_ready) + 10;
        let first_duration = if crash_target == Some(spec.id.as_str()) {
            100
        } else {
            spec.duration_ms
        };
        let mut started_ms =
            scheduled_start(earliest_ms, first_duration, &tasks, lane.max_parallel_tasks);
        let mut attempt = 1;
        if crash_target == Some(spec.id.as_str()) {
            let failed = task_receipt(
                spec,
                lane,
                1,
                started_ms,
                started_ms + 100,
                TaskOutcome::Failed,
                true,
            );
            tasks.push(failed);
            started_ms = scheduled_start(
                started_ms + 110,
                spec.duration_ms,
                &tasks,
                lane.max_parallel_tasks,
            );
            attempt = 2;
            recoveries.push(RecoveryReceipt {
                failed_subject: spec.id.clone(),
                replacement_subject: spec.id.clone(),
                reason: "worker crashed after publishing an attempt artifact".into(),
                preserved_artifact_shas: Vec::new(),
                restarted_subjects: vec![spec.id.clone()],
            });
        }
        let receipt = task_receipt(
            spec,
            lane,
            attempt,
            started_ms,
            started_ms + spec.duration_ms,
            TaskOutcome::Completed,
            true,
        );
        finish_by_id.insert(spec.id.clone(), receipt.finished_ms);
        tasks.push(receipt);
    }

    let review_required = scenario.requires_independent_review;
    if review_required {
        let earliest = tasks
            .iter()
            .filter(|task| task.outcome == TaskOutcome::Completed && task.role.writes())
            .map(|task| task.finished_ms)
            .max()
            .unwrap_or(50)
            + 10;
        let started = scheduled_start(earliest, 180, &tasks, lane.max_parallel_tasks);
        tasks.push(TaskReceipt {
            id: "independent-review".into(),
            attempt: 1,
            role: TaskRole::Review,
            dependencies: scenario
                .tasks
                .iter()
                .filter(|task| task.role.writes())
                .map(|task| task.id.clone())
                .collect(),
            resources: Vec::new(),
            model: lane
                .reviewer_model
                .clone()
                .unwrap_or_else(|| lane.root_model.clone()),
            model_tier: if lane.kind == LaneKind::WorkGraphDiverseReview {
                "independent".into()
            } else {
                "strong".into()
            },
            harness: "review-harness".into(),
            workspace_id: "fresh-review".into(),
            write_scope: BTreeSet::new(),
            reserved_tokens: 8_000,
            started_ms: started,
            finished_ms: started + 180,
            outcome: TaskOutcome::Completed,
        });
    }

    let mut artifacts = build_artifacts(scenario, workspace, &tasks, review_required);
    if scenario.fault == FaultInjection::SourceBaselineDrift {
        let mut stale = artifacts[0].clone();
        stale.content_sha256 = digest(&format!("stale:{}", stale.artifact_id));
        if let Some((_project, baseline)) = stale.source_baselines.iter_mut().next() {
            *baseline = format!("obsolete-{baseline}");
        }
        stale.stale = true;
        stale.rejected = true;
        stale.consumed_by.clear();
        stale.verified_by.clear();
        stale.integrity_sha256 = stale.expected_integrity();
        let rejected_sha = stale.integrity_sha256.clone();
        artifacts.insert(0, stale);
        recoveries.push(RecoveryReceipt {
            failed_subject: rejected_sha,
            replacement_subject: artifacts[1].integrity_sha256.clone(),
            reason: "source baseline changed before the artifact was consumed".into(),
            preserved_artifact_shas: artifacts
                .iter()
                .skip(2)
                .map(|artifact| artifact.integrity_sha256.clone())
                .collect(),
            restarted_subjects: vec![artifacts[1].producer_task.clone()],
        });
    }
    if let Some(recovery) = recoveries
        .iter_mut()
        .find(|receipt| receipt.reason.contains("worker crashed"))
    {
        recovery.preserved_artifact_shas = artifacts
            .iter()
            .filter(|artifact| artifact.producer_task != recovery.failed_subject)
            .map(|artifact| artifact.integrity_sha256.clone())
            .collect();
    }
    if scenario.fault == FaultInjection::ResourceProvisionFailure {
        recoveries.push(RecoveryReceipt {
            failed_subject: scenario.resources[0].id.clone(),
            replacement_subject: format!("{}-attempt-2", scenario.resources[0].id),
            reason: "resource failed its readiness probe and was reprovisioned".into(),
            preserved_artifact_shas: artifacts
                .iter()
                .map(|artifact| artifact.integrity_sha256.clone())
                .collect(),
            restarted_subjects: vec![scenario.resources[0].id.clone()],
        });
    }
    if scenario.fault == FaultInjection::ResourceExpiry {
        recoveries.push(RecoveryReceipt {
            failed_subject: scenario.resources[0].id.clone(),
            replacement_subject: format!("{}-attempt-2", scenario.resources[0].id),
            reason: "resource lease expired before dependent work became runnable".into(),
            preserved_artifact_shas: artifacts
                .iter()
                .filter(|artifact| artifact.producer_task == "diagnose-runner")
                .map(|artifact| artifact.integrity_sha256.clone())
                .collect(),
            restarted_subjects: vec![scenario.resources[0].id.clone()],
        });
    }

    attach_resource_users_and_cleanup(&mut resources, &tasks);
    let wakeups = graph_wakeups(&tasks, &resources);
    let verification = verification(scenario, &tasks, &artifacts);
    let events = build_events(&tasks, &resources, &artifacts);
    let usage = usage(lane, &tasks, 0, 0);
    CandidateResult {
        schema_version: 1,
        candidate_id: "reference".into(),
        scenario_id: scenario.id.clone(),
        lane_id: lane.id.clone(),
        production_trace_id: None,
        delegated: true,
        delegation_reason:
            "Independent runnable nodes have isolated ownership and typed dependencies".into(),
        plan: Some(plan(scenario, true, TraceAuthority::HostSimulation)),
        tasks,
        resources,
        artifacts,
        wakeups,
        recoveries,
        verification: Some(verification),
        events,
        usage,
        safety: SafetyReceipt::default(),
        interaction: Some(simple_interaction()),
        claimed_complete: true,
        error: None,
    }
}

fn simulate_ordered(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
    delegated: bool,
    authority: TraceAuthority,
) -> CandidateResult {
    let mut resources = provision_resources(scenario);
    let mut clock = resources
        .iter()
        .filter_map(|resource| resource.ready_ms)
        .max()
        .unwrap_or(40);
    let mut tasks = Vec::new();
    let mut recovery_subject = None;
    let crash_target = if scenario.fault == FaultInjection::WorkerCrashAfterArtifact {
        scenario
            .tasks
            .iter()
            .rev()
            .find(|task| task.role.writes())
            .map(|task| task.id.as_str())
    } else {
        None
    };
    for spec in &scenario.tasks {
        let mut started = clock + 10;
        let mut attempt = 1;
        if crash_target == Some(spec.id.as_str()) {
            let mut failed = task_receipt(
                spec,
                lane,
                1,
                started,
                started + 100,
                TaskOutcome::Failed,
                false,
            );
            failed.workspace_id = "root-workspace".into();
            failed.model = lane.root_model.clone();
            failed.model_tier = "strong".into();
            clock = failed.finished_ms;
            tasks.push(failed);
            started = clock + 10;
            attempt = 2;
            recovery_subject = Some(spec.id.clone());
        }
        clock = started + spec.duration_ms;
        let mut receipt = task_receipt(
            spec,
            lane,
            attempt,
            started,
            clock,
            TaskOutcome::Completed,
            false,
        );
        receipt.workspace_id = "root-workspace".into();
        receipt.model = lane.root_model.clone();
        receipt.model_tier = "strong".into();
        tasks.push(receipt);
    }
    attach_resource_users_and_cleanup(&mut resources, &tasks);
    let artifacts = build_artifacts(scenario, workspace, &tasks, false);
    let recoveries = ordered_recoveries(scenario, &artifacts, recovery_subject);
    let verification = verification(scenario, &tasks, &artifacts);
    let polling = scenario.resources.iter().map(|_| 2_000).sum();
    let usage = usage(lane, &tasks, polling, 0);
    CandidateResult {
        schema_version: 1,
        candidate_id: "reference".into(),
        scenario_id: scenario.id.clone(),
        lane_id: lane.id.clone(),
        production_trace_id: None,
        delegated,
        delegation_reason: "The work is serialized in one coding session".into(),
        plan: Some(plan(scenario, delegated, authority)),
        tasks,
        resources,
        artifacts,
        wakeups: Vec::new(),
        recoveries,
        verification: Some(verification),
        events: Vec::new(),
        usage,
        safety: SafetyReceipt::default(),
        interaction: Some(simple_interaction()),
        claimed_complete: true,
        error: None,
    }
}

fn ordered_recoveries(
    scenario: &Scenario,
    artifacts: &[ArtifactReceipt],
    worker_subject: Option<String>,
) -> Vec<RecoveryReceipt> {
    let preserved = |excluded_producer: Option<&str>| {
        artifacts
            .iter()
            .filter(|artifact| excluded_producer != Some(artifact.producer_task.as_str()))
            .map(|artifact| artifact.integrity_sha256.clone())
            .collect::<Vec<_>>()
    };
    match scenario.fault {
        FaultInjection::None => Vec::new(),
        FaultInjection::WorkerCrashAfterArtifact => worker_subject
            .map(|subject| RecoveryReceipt {
                failed_subject: subject.clone(),
                replacement_subject: subject.clone(),
                reason: "single-agent writer attempt failed and was retried in place".into(),
                preserved_artifact_shas: preserved(Some(&subject)),
                restarted_subjects: vec![subject],
            })
            .into_iter()
            .collect(),
        FaultInjection::ResourceProvisionFailure => vec![RecoveryReceipt {
            failed_subject: scenario.resources[0].id.clone(),
            replacement_subject: format!("{}-attempt-2", scenario.resources[0].id),
            reason: "single-agent resource provisioning retried after readiness failure".into(),
            preserved_artifact_shas: preserved(None),
            restarted_subjects: vec![scenario.resources[0].id.clone()],
        }],
        FaultInjection::ResourceExpiry => vec![RecoveryReceipt {
            failed_subject: scenario.resources[0].id.clone(),
            replacement_subject: format!("{}-attempt-2", scenario.resources[0].id),
            reason: "single-agent resource lease was renewed after expiry".into(),
            preserved_artifact_shas: preserved(None),
            restarted_subjects: vec![scenario.resources[0].id.clone()],
        }],
        FaultInjection::SourceBaselineDrift => vec![RecoveryReceipt {
            failed_subject: "stale-artifact".into(),
            replacement_subject: artifacts
                .first()
                .map(|artifact| artifact.integrity_sha256.clone())
                .unwrap_or_else(|| "recomputed-artifact".into()),
            reason: "single-agent artifact was recomputed after source baseline drift".into(),
            preserved_artifact_shas: preserved(None),
            restarted_subjects: vec![artifacts
                .first()
                .map(|artifact| artifact.producer_task.clone())
                .unwrap_or_else(|| "producer".into())],
        }],
    }
}

fn simulate_naive(
    scenario: &Scenario,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
) -> CandidateResult {
    let tasks = scenario
        .tasks
        .iter()
        .map(|spec| {
            let mut receipt = task_receipt(
                spec,
                lane,
                1,
                100,
                100 + spec.duration_ms,
                TaskOutcome::Completed,
                false,
            );
            receipt.workspace_id = "shared-root-workspace".into();
            receipt
        })
        .collect::<Vec<_>>();
    let resources = scenario
        .resources
        .iter()
        .flat_map(|resource| {
            let users = scenario
                .tasks
                .iter()
                .filter(|task| task.resources.contains(&resource.id))
                .collect::<Vec<_>>();
            users
                .iter()
                .enumerate()
                .map(|(index, task)| ResourceReceipt {
                    resource_id: resource.id.clone(),
                    instance_id: format!("{}-duplicate-{index}", resource.id),
                    attempt: 1,
                    kind: resource.kind.clone(),
                    requested_ms: 0,
                    ready_ms: Some(resource.provision_ms),
                    expires_ms: Some(resource.provision_ms + resource.ttl_ms),
                    released_ms: None,
                    outcome: ResourceOutcome::Ready,
                    used_by: vec![task.id.clone()],
                    health_checks: 0,
                    host_supervised: false,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let artifacts = build_artifacts(scenario, workspace, &tasks, false);
    let verification = verification(scenario, &tasks, &artifacts);
    let duplicate_setup = scenario.resources.len() as u64 * 8_000;
    let mut usage = usage(lane, &tasks, 12_000, duplicate_setup);
    usage.peak_reserved_tokens = lane.token_budget.saturating_add(1);
    CandidateResult {
        schema_version: 1,
        candidate_id: "reference".into(),
        scenario_id: scenario.id.clone(),
        lane_id: lane.id.clone(),
        production_trace_id: None,
        delegated: scenario.expected_delegate,
        delegation_reason: "Every visible subproblem was spawned immediately".into(),
        plan: Some(plan(
            scenario,
            scenario.expected_delegate,
            TraceAuthority::SelfReported,
        )),
        tasks,
        resources,
        artifacts,
        wakeups: Vec::new(),
        recoveries: Vec::new(),
        verification: Some(verification),
        events: Vec::new(),
        usage,
        safety: SafetyReceipt {
            raw_process_handoffs: scenario
                .resources
                .iter()
                .map(|resource| format!("raw-handle:{}", resource.id))
                .collect(),
            ..SafetyReceipt::default()
        },
        interaction: Some(simple_interaction()),
        claimed_complete: true,
        error: None,
    }
}

fn provision_resources(scenario: &Scenario) -> Vec<ResourceReceipt> {
    let mut receipts = Vec::new();
    for resource in &scenario.resources {
        let failed = scenario.fault == FaultInjection::ResourceProvisionFailure;
        let expired = scenario.fault == FaultInjection::ResourceExpiry;
        if failed {
            receipts.push(ResourceReceipt {
                resource_id: resource.id.clone(),
                instance_id: format!("{}-attempt-1", resource.id),
                attempt: 1,
                kind: resource.kind.clone(),
                requested_ms: 0,
                ready_ms: None,
                expires_ms: None,
                released_ms: Some(180),
                outcome: ResourceOutcome::Failed,
                used_by: Vec::new(),
                health_checks: 1,
                host_supervised: true,
            });
        }
        if expired {
            let ready = resource.provision_ms;
            let expires = ready + resource.ttl_ms;
            receipts.push(ResourceReceipt {
                resource_id: resource.id.clone(),
                instance_id: format!("{}-attempt-1", resource.id),
                attempt: 1,
                kind: resource.kind.clone(),
                requested_ms: 0,
                ready_ms: Some(ready),
                expires_ms: Some(expires),
                released_ms: Some(expires),
                outcome: ResourceOutcome::Expired,
                used_by: Vec::new(),
                health_checks: 1,
                host_supervised: true,
            });
        }
        let requested = if failed {
            190
        } else if expired {
            resource.provision_ms + resource.ttl_ms + 10
        } else {
            0
        };
        let ready = requested + resource.provision_ms;
        receipts.push(ResourceReceipt {
            resource_id: resource.id.clone(),
            instance_id: format!("{}-attempt-{}", resource.id, if failed { 2 } else { 1 }),
            attempt: if failed || expired { 2 } else { 1 },
            kind: resource.kind.clone(),
            requested_ms: requested,
            ready_ms: Some(ready),
            expires_ms: Some(ready + resource.ttl_ms),
            released_ms: None,
            outcome: ResourceOutcome::Ready,
            used_by: Vec::new(),
            health_checks: 2,
            host_supervised: true,
        });
    }
    receipts
}

fn attach_resource_users_and_cleanup(resources: &mut [ResourceReceipt], tasks: &[TaskReceipt]) {
    for resource in resources
        .iter_mut()
        .filter(|resource| resource.outcome == ResourceOutcome::Ready)
    {
        resource.used_by = tasks
            .iter()
            .filter(|task| task.resources.contains(&resource.resource_id))
            .map(|task| task.id.clone())
            .collect();
        let last_use = tasks
            .iter()
            .filter(|task| task.resources.contains(&resource.resource_id))
            .map(|task| task.finished_ms)
            .max()
            .unwrap_or_else(|| resource.ready_ms.unwrap_or(0));
        resource.released_ms = Some(last_use + 20);
    }
}

fn build_artifacts(
    scenario: &Scenario,
    workspace: &SeededWorkspace,
    tasks: &[TaskReceipt],
    reviewed: bool,
) -> Vec<ArtifactReceipt> {
    let baselines = workspace.baselines();
    let verifier = tasks
        .iter()
        .find(|task| task.role == TaskRole::Verify)
        .map(|task| task.id.clone())
        .unwrap_or_else(|| "fresh-verifier".into());
    let mut artifacts = Vec::new();
    for spec in &scenario.tasks {
        let Some(task) = tasks
            .iter()
            .filter(|task| task.id == spec.id && task.outcome == TaskOutcome::Completed)
            .max_by_key(|task| task.attempt)
        else {
            continue;
        };
        let input_shas = artifacts
            .iter()
            .filter(|artifact: &&ArtifactReceipt| {
                spec.dependencies.contains(&artifact.producer_task)
            })
            .map(|artifact| artifact.integrity_sha256.clone())
            .collect::<Vec<_>>();
        for output in &spec.outputs {
            let mut receipt = ArtifactReceipt {
                artifact_id: output.clone(),
                producer_task: spec.id.clone(),
                source_baselines: baselines.clone(),
                input_artifact_shas: input_shas.clone(),
                content_sha256: digest(&format!("{}:{}:fixed", scenario.id, output)),
                integrity_sha256: String::new(),
                produced_ms: task.finished_ms,
                consumed_by: scenario
                    .tasks
                    .iter()
                    .filter(|consumer| consumer.dependencies.contains(&spec.id))
                    .map(|consumer| consumer.id.clone())
                    .collect(),
                verified_by: if scenario.final_artifacts.contains(output) {
                    let mut verifiers = vec![verifier.clone()];
                    if reviewed {
                        verifiers.push("independent-review".into());
                    }
                    verifiers
                } else {
                    Vec::new()
                },
                stale: false,
                rejected: false,
            };
            receipt.integrity_sha256 = receipt.expected_integrity();
            artifacts.push(receipt);
        }
    }
    artifacts
}

fn task_receipt(
    spec: &super::model::TaskSpec,
    lane: &LaneSpec,
    attempt: u32,
    started_ms: u64,
    finished_ms: u64,
    outcome: TaskOutcome,
    isolated: bool,
) -> TaskReceipt {
    let cheap = matches!(
        lane.kind,
        LaneKind::WorkGraphCheapSupport
            | LaneKind::WorkGraphDiverseReview
            | LaneKind::WorkGraphCloud
    ) && spec.role.cheap_eligible();
    let cloud = lane.kind == LaneKind::WorkGraphCloud && spec.cloud_eligible;
    TaskReceipt {
        id: spec.id.clone(),
        attempt,
        role: spec.role,
        dependencies: spec.dependencies.clone(),
        resources: spec.resources.clone(),
        model: if cheap {
            lane.support_model.clone()
        } else {
            lane.root_model.clone()
        },
        model_tier: if cheap { "cheap" } else { "strong" }.into(),
        harness: if cloud { "brokered-cloud" } else { "local" }.into(),
        workspace_id: if isolated && spec.role.writes() {
            format!("isolated:{}", spec.id)
        } else {
            format!("shared-read:{}", spec.id)
        },
        write_scope: spec.write_scope.clone(),
        reserved_tokens: spec.token_estimate,
        started_ms,
        finished_ms,
        outcome,
    }
}

fn graph_wakeups(
    tasks: &[TaskReceipt],
    resources: &[ResourceReceipt],
) -> Vec<DependencyWakeupReceipt> {
    let completed = tasks
        .iter()
        .filter(|task| task.outcome == TaskOutcome::Completed)
        .map(|task| (task.id.as_str(), task.finished_ms))
        .collect::<BTreeMap<_, _>>();
    let ready = resources
        .iter()
        .filter(|resource| resource.outcome == ResourceOutcome::Ready)
        .filter_map(|resource| {
            resource
                .ready_ms
                .map(|at| (resource.resource_id.as_str(), at))
        })
        .collect::<BTreeMap<_, _>>();
    let mut receipts = Vec::new();
    for task in tasks
        .iter()
        .filter(|task| task.outcome == TaskOutcome::Completed)
    {
        for dependency in &task.dependencies {
            if let Some(at) = completed.get(dependency.as_str()) {
                receipts.push(DependencyWakeupReceipt {
                    task_id: task.id.clone(),
                    dependency_id: dependency.clone(),
                    dependency_kind: "task".into(),
                    at_ms: *at,
                    host_generated: true,
                });
            }
        }
        for resource in &task.resources {
            if let Some(at) = ready.get(resource.as_str()) {
                receipts.push(DependencyWakeupReceipt {
                    task_id: task.id.clone(),
                    dependency_id: resource.clone(),
                    dependency_kind: "resource".into(),
                    at_ms: *at,
                    host_generated: true,
                });
            }
        }
    }
    receipts
}

fn verification(
    scenario: &Scenario,
    tasks: &[TaskReceipt],
    artifacts: &[ArtifactReceipt],
) -> VerificationReceipt {
    let verifier = tasks
        .iter()
        .find(|task| task.role == TaskRole::Verify)
        .map(|task| task.id.clone())
        .unwrap_or_else(|| "fresh-verifier".into());
    VerificationReceipt {
        verifier_task: verifier,
        fresh_workspace: true,
        checked_artifact_shas: artifacts
            .iter()
            .filter(|artifact| {
                !artifact.stale && scenario.final_artifacts.contains(&artifact.artifact_id)
            })
            .map(|artifact| artifact.integrity_sha256.clone())
            .collect(),
        checks: vec!["hidden repository state".into(), "fresh integration".into()],
        passed: true,
    }
}

fn build_events(
    tasks: &[TaskReceipt],
    resources: &[ResourceReceipt],
    artifacts: &[ArtifactReceipt],
) -> Vec<TraceEvent> {
    let mut raw = Vec::new();
    for resource in resources {
        raw.push((
            resource.requested_ms,
            "resource_requested",
            resource.instance_id.clone(),
        ));
        if let Some(ready) = resource.ready_ms {
            raw.push((ready, "resource_ready", resource.instance_id.clone()));
        }
        if let Some(released) = resource.released_ms {
            raw.push((released, "resource_released", resource.instance_id.clone()));
        }
    }
    for task in tasks {
        raw.push((task.started_ms, "task_started", task.id.clone()));
        raw.push((task.finished_ms, "task_finished", task.id.clone()));
    }
    for artifact in artifacts {
        raw.push((
            artifact.produced_ms,
            "artifact_published",
            artifact.artifact_id.clone(),
        ));
    }
    raw.sort_by_key(|(at, kind, subject)| (*at, *kind, subject.clone()));
    raw.into_iter()
        .enumerate()
        .map(|(index, (at_ms, kind, subject))| TraceEvent {
            sequence: index as u64,
            at_ms,
            kind: kind.into(),
            subject,
        })
        .collect()
}

fn usage(
    _lane: &LaneSpec,
    tasks: &[TaskReceipt],
    polling_tokens: u64,
    duplicate_setup_tokens: u64,
) -> UsageReceipt {
    let reserved = tasks.iter().map(|task| task.reserved_tokens).sum::<u64>();
    let useful_reserved = tasks
        .iter()
        .filter(|task| task.outcome == TaskOutcome::Completed)
        .map(|task| task.reserved_tokens)
        .sum::<u64>();
    let input_tokens = reserved.saturating_mul(4) / 5 + polling_tokens + duplicate_setup_tokens;
    let output_tokens = reserved / 5;
    let wall_ms = tasks.iter().map(|task| task.finished_ms).max().unwrap_or(0);
    let agent_ms = tasks
        .iter()
        .map(|task| task.finished_ms.saturating_sub(task.started_ms))
        .sum();
    let task_cost = tasks
        .iter()
        .map(|task| {
            let task_input = task.reserved_tokens.saturating_mul(4) / 5;
            let task_output = task.reserved_tokens / 5;
            let (input_rate, output_rate) = if task.model_tier == "cheap" {
                (0.000_000_05, 0.000_000_2)
            } else {
                (0.000_000_2, 0.000_000_8)
            };
            task_input as f64 * input_rate + task_output as f64 * output_rate
        })
        .sum::<f64>();
    let overhead_cost = (polling_tokens + duplicate_setup_tokens) as f64 * 0.000_000_2;
    UsageReceipt {
        input_tokens,
        output_tokens,
        useful_tokens: useful_reserved.saturating_mul(3) / 4,
        model_polling_tokens: polling_tokens,
        duplicate_setup_tokens,
        cost_usd: task_cost + overhead_cost,
        wall_ms,
        agent_ms,
        peak_reserved_tokens: peak_reserved(tasks),
    }
}

fn scheduled_start(
    earliest_ms: u64,
    duration_ms: u64,
    tasks: &[TaskReceipt],
    max_parallel_tasks: usize,
) -> u64 {
    let mut start = earliest_ms;
    loop {
        let finish = start.saturating_add(duration_ms);
        let mut moments = vec![start];
        moments.extend(
            tasks
                .iter()
                .filter(|task| start < task.started_ms && task.started_ms < finish)
                .map(|task| task.started_ms),
        );
        moments.sort_unstable();
        let blocked_at = moments.into_iter().find(|moment| {
            tasks
                .iter()
                .filter(|task| task.started_ms <= *moment && *moment < task.finished_ms)
                .count()
                >= max_parallel_tasks
        });
        let Some(blocked_at) = blocked_at else {
            return start;
        };
        start = tasks
            .iter()
            .filter(|task| task.started_ms <= blocked_at && blocked_at < task.finished_ms)
            .map(|task| task.finished_ms)
            .min()
            .unwrap_or(start.saturating_add(1));
    }
}

fn peak_reserved(tasks: &[TaskReceipt]) -> u64 {
    let mut moments = tasks
        .iter()
        .flat_map(|task| [task.started_ms, task.finished_ms])
        .collect::<Vec<_>>();
    moments.sort_unstable();
    moments
        .into_iter()
        .map(|moment| {
            tasks
                .iter()
                .filter(|task| task.started_ms <= moment && moment < task.finished_ms)
                .map(|task| task.reserved_tokens)
                .sum()
        })
        .max()
        .unwrap_or(0)
}
