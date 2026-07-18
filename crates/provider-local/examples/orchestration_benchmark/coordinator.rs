use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent_core::domain::AgentEvent;
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::future::join_all;
use futures::StreamExt;
use uuid::Uuid;

use crate::control::{ControlMessage, ControlPlane};
use crate::model::{
    AgentStatus, AttemptRecord, BenchmarkRecord, CheckResult, DeliveryMode, EvidenceLevel,
    HardFailure, LaneKind, LaneSpec, PermissionCeiling, ReviewFinding, ReviewVerdict,
    StructuredHandoff, TaskContract, TaskMode, TaskStatus,
};
use crate::scenarios::{self, FaultInjection, Scenario, SeededRepository};
use crate::scripted_provider::{
    attempt_from_events, ScriptedAction, ScriptedFault, ScriptedProfile, ScriptedProvider,
    ScriptedTaskEnvelope,
};

#[path = "coordinator/support.rs"]
pub(crate) mod support;
use support::{
    checkpoint, final_task_statuses, metrics, planner_task, reader_task, review_task,
    trigger_metrics, unix_ms, verify_task, writer_task,
};

const SCRIPTED_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ScriptedRunOptions {
    pub artifact_root: PathBuf,
    pub repetition: u32,
}

struct AttemptRequest {
    task: TaskContract,
    action: ScriptedAction,
    fault: ScriptedFault,
    profile: ScriptedProfile,
    requested_permission: PermissionCeiling,
}

pub async fn run_scripted(
    scenario: &Scenario,
    lane: &LaneSpec,
    options: &ScriptedRunOptions,
) -> Result<BenchmarkRecord, String> {
    let run_id = format!(
        "{}-{}-r{}-{}",
        scenario.id,
        lane.id,
        options.repetition,
        &Uuid::new_v4().to_string()[..8]
    );
    let run_root = options.artifact_root.join(&run_id);
    let repo_root = run_root.join("repo");
    let attempt_root = run_root.join("attempts");
    std::fs::create_dir_all(&attempt_root).map_err(|error| error.to_string())?;
    let seeded = scenarios::seed(&repo_root, scenario)?;
    let baseline_checkpoint = checkpoint(&seeded)
        .await
        .or_else(|| seeded.git_baseline.clone());
    let started_at_unix_ms = unix_ms();
    let started = Instant::now();
    let control = ControlPlane::new(lane.max_concurrency.max(1), lane.token_budget);
    let actual_delegate = should_delegate(lane.kind, scenario);
    let mut tasks = Vec::new();
    let mut attempts = Vec::new();
    let mut handoffs = Vec::new();
    let mut reviews = Vec::new();
    let mut recovered_failures = 0;
    let mut unrecovered_failures = 0;
    let mut review_catches = 0;
    let mut review_false_vetoes = 0;
    let mut error = None;

    if matches!(lane.kind, LaneKind::PlannedSingle) {
        let task = planner_task(scenario);
        tasks.push(task.clone());
        let request = AttemptRequest {
            task,
            action: ScriptedAction::Inspect {
                finding: "single-agent plan captured before implementation".into(),
            },
            fault: ScriptedFault::None,
            profile: profile(lane, "planner", false, false),
            requested_permission: PermissionCeiling::ReadOnly,
        };
        match run_attempt(
            &control,
            &seeded,
            baseline_checkpoint.clone(),
            request,
            &attempt_root,
        )
        .await
        {
            Ok(attempt) => collect_attempt(&control, attempt, &mut attempts, &mut handoffs),
            Err(reason) => error = Some(reason),
        }
    }

    if actual_delegate {
        let requests: Vec<_> = scenario
            .reader_tasks
            .iter()
            .map(|reader| {
                let task = reader_task(reader);
                tasks.push(task.clone());
                let use_cloud = lane.cloud_agents && reader.cloud_eligible;
                AttemptRequest {
                    task,
                    action: ScriptedAction::Inspect {
                        finding: reader.expected_finding.clone(),
                    },
                    fault: ScriptedFault::None,
                    profile: profile(lane, "reader", true, use_cloud),
                    requested_permission: PermissionCeiling::ReadOnly,
                }
            })
            .collect();
        let futures = requests.into_iter().map(|request| {
            run_attempt(
                &control,
                &seeded,
                baseline_checkpoint.clone(),
                request,
                &attempt_root,
            )
        });
        for result in join_all(futures).await {
            match result {
                Ok(attempt) => {
                    control.send_message(ControlMessage {
                        sender: attempt.agent_path.clone(),
                        target: "/root/implement".into(),
                        body: format!("result available for {}", attempt.task_id),
                        mode: DeliveryMode::QueueOnly,
                    });
                    collect_attempt(&control, attempt, &mut attempts, &mut handoffs);
                }
                Err(reason) => {
                    unrecovered_failures += 1;
                    error.get_or_insert(reason);
                }
            }
        }
        control.send_message(ControlMessage {
            sender: "/root".into(),
            target: "/root/implement".into(),
            body: "reader barrier complete".into(),
            mode: DeliveryMode::TriggerTurn,
        });
        let delivery_receipt = serde_json::json!({
            "queued_messages": control.drain_mailbox("/root/implement").len(),
            "wake_count": control.wake_count("/root/implement"),
        });
        std::fs::write(
            run_root.join("control-delivery.json"),
            serde_json::to_vec_pretty(&delivery_receipt).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;

        if scenario
            .faults
            .contains(&FaultInjection::RestartAfterReaders)
        {
            let restored = ControlPlane::restore(control.snapshot());
            let snapshot = restored.snapshot();
            std::fs::write(
                run_root.join("restored-control.json"),
                serde_json::to_vec_pretty(&snapshot).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        }
    }

    let writer_task = writer_task(scenario, &tasks);
    tasks.push(writer_task.clone());
    let first_writer_fault = writer_fault(scenario, lane);
    let first_writer_files = if scenario.faults.contains(&FaultInjection::ReviewerSeededBug) {
        reviewer_seeded_bug_files(scenario)
    } else {
        scenario.solution.clone()
    };
    let first_request = AttemptRequest {
        task: writer_task.clone(),
        action: ScriptedAction::Apply {
            files: first_writer_files,
        },
        fault: first_writer_fault,
        profile: profile(lane, "writer", false, false),
        requested_permission: if scenario
            .faults
            .contains(&FaultInjection::PermissionEscalation)
        {
            PermissionCeiling::Full
        } else {
            PermissionCeiling::WorkspaceWrite
        },
    };
    let mut writer_succeeded = false;
    match run_attempt(
        &control,
        &seeded,
        baseline_checkpoint.clone(),
        first_request,
        &attempt_root,
    )
    .await
    {
        Ok(attempt) if valid_writer_handoff(&attempt, &seeded) => {
            writer_succeeded = attempt.status == AgentStatus::Completed;
            collect_attempt(&control, attempt, &mut attempts, &mut handoffs);
        }
        Ok(attempt) => {
            attempts.push(attempt);
            recovered_failures += 1;
        }
        Err(_reason) => {
            recovered_failures += 1;
        }
    }

    if !writer_succeeded {
        let retry = AttemptRequest {
            task: writer_task.clone(),
            action: ScriptedAction::Apply {
                files: scenario.solution.clone(),
            },
            fault: ScriptedFault::None,
            profile: profile(lane, "writer-retry", false, false),
            requested_permission: PermissionCeiling::WorkspaceWrite,
        };
        match run_attempt(
            &control,
            &seeded,
            baseline_checkpoint.clone(),
            retry,
            &attempt_root,
        )
        .await
        {
            Ok(attempt) if valid_writer_handoff(&attempt, &seeded) => {
                writer_succeeded = attempt.status == AgentStatus::Completed;
                collect_attempt(&control, attempt, &mut attempts, &mut handoffs);
            }
            Ok(attempt) => {
                attempts.push(attempt);
                unrecovered_failures += 1;
            }
            Err(reason) => {
                unrecovered_failures += 1;
                error.get_or_insert(reason);
            }
        }
    }

    if writer_succeeded && lane.reviewer {
        let seeded_bug = scenario.faults.contains(&FaultInjection::ReviewerSeededBug);
        let review_task = review_task(scenario, &writer_task);
        tasks.push(review_task.clone());
        let review_request = AttemptRequest {
            task: review_task.clone(),
            action: ScriptedAction::Review {
                accepted: !seeded_bug,
                finding: seeded_bug.then(|| "writer did not apply the required change".into()),
            },
            fault: ScriptedFault::None,
            profile: profile(lane, "reviewer", true, false),
            requested_permission: PermissionCeiling::ReadOnly,
        };
        match run_attempt(
            &control,
            &seeded,
            baseline_checkpoint.clone(),
            review_request,
            &attempt_root,
        )
        .await
        {
            Ok(attempt) => {
                collect_attempt(&control, attempt, &mut attempts, &mut handoffs);
                let verdict = ReviewVerdict {
                    task_id: writer_task.id.clone(),
                    accepted: !seeded_bug,
                    findings: seeded_bug
                        .then(|| ReviewFinding {
                            severity: "high".into(),
                            path: scenario.solution.first().map(|file| file.path.clone()),
                            message: "required implementation is absent".into(),
                            evidence_ref: "review:diff".into(),
                        })
                        .into_iter()
                        .collect(),
                };
                if seeded_bug {
                    review_catches += 1;
                    control.set_task_status(&writer_task.id, TaskStatus::Rework);
                    let rework = AttemptRequest {
                        task: writer_task.clone(),
                        action: ScriptedAction::Apply {
                            files: scenario.solution.clone(),
                        },
                        fault: ScriptedFault::None,
                        profile: profile(lane, "writer-rework", false, false),
                        requested_permission: PermissionCeiling::WorkspaceWrite,
                    };
                    match run_attempt(
                        &control,
                        &seeded,
                        baseline_checkpoint.clone(),
                        rework,
                        &attempt_root,
                    )
                    .await
                    {
                        Ok(attempt) => {
                            collect_attempt(&control, attempt, &mut attempts, &mut handoffs);
                            writer_succeeded = true;
                        }
                        Err(reason) => {
                            writer_succeeded = false;
                            unrecovered_failures += 1;
                            error.get_or_insert(reason);
                        }
                    }
                }
                reviews.push(verdict);
            }
            Err(reason) => {
                review_false_vetoes += 1;
                error.get_or_insert(reason);
            }
        }
    }

    if writer_succeeded && lane.verifier {
        let verify_task = verify_task(scenario, &writer_task);
        tasks.push(verify_task.clone());
        let request = AttemptRequest {
            task: verify_task,
            action: ScriptedAction::Verify,
            fault: ScriptedFault::None,
            profile: profile(lane, "verifier", true, false),
            requested_permission: PermissionCeiling::ReadOnly,
        };
        match run_attempt(
            &control,
            &seeded,
            baseline_checkpoint.clone(),
            request,
            &attempt_root,
        )
        .await
        {
            Ok(attempt) => collect_attempt(&control, attempt, &mut attempts, &mut handoffs),
            Err(reason) => {
                unrecovered_failures += 1;
                error.get_or_insert(reason);
            }
        }
    }

    if scenario.faults.contains(&FaultInjection::DuplicateReport) {
        if let Some(handoff) = handoffs.last().cloned() {
            let _ = control.report_result(handoff);
        }
    }

    if writer_succeeded {
        match handoffs
            .iter()
            .rev()
            .find(|handoff| handoff.task_id == writer_task.id)
        {
            Some(handoff) => {
                if let Err(reason) = control.accept_result(&writer_task.id, &handoff.attempt_id) {
                    writer_succeeded = false;
                    unrecovered_failures += 1;
                    error.get_or_insert(reason);
                }
            }
            None => writer_succeeded = false,
        }
    }

    let grade = scenarios::grade(scenario, &seeded)?;
    let result_checkpoint = checkpoint(&seeded).await;
    let mut hard_failures = BTreeSet::new();
    if !grade.unexpected_changed_paths.is_empty() {
        hard_failures.insert(HardFailure::OutOfScopeWrite);
    }
    if !grade.lost_user_changes.is_empty() {
        hard_failures.insert(HardFailure::LostUserChange);
    }
    if attempts.is_empty() {
        hard_failures.insert(HardFailure::CausalTraceMissing);
    }
    if writer_succeeded
        && !handoffs
            .iter()
            .any(|handoff| handoff.task_id == writer_task.id)
    {
        hard_failures.insert(HardFailure::AcceptedUnverifiableResult);
    }
    let checks = grade
        .checks
        .into_iter()
        .map(|(id, passed, detail)| CheckResult { id, passed, detail })
        .collect();
    let task_statuses = final_task_statuses(&control, &tasks, grade.correctness, writer_succeeded);
    let duration_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let metrics = metrics(
        &attempts,
        grade.correctness,
        grade.changed_path_precision,
        recovered_failures,
        unrecovered_failures,
        review_catches,
        review_false_vetoes,
        duration_ms,
        lane,
        actual_delegate,
    );
    if metrics.lifecycle_trace_failures > 0 {
        hard_failures.insert(HardFailure::LifecycleTraceInvalid);
    }
    if metrics.duplicate_tool_receipts > 0 {
        hard_failures.insert(HardFailure::DuplicateToolReceipt);
    }
    let trigger = trigger_metrics(scenario, lane, actual_delegate);
    let orchestration_complete =
        hard_failures.is_empty() && grade.correctness >= 1.0 && writer_succeeded && error.is_none();
    if error.is_some() {
        control.cancel();
    }
    let record = BenchmarkRecord {
        schema_version: 1,
        run_id,
        evidence_level: EvidenceLevel::Scripted,
        scenario_id: scenario.id.clone(),
        scenario_family: scenario.family.clone(),
        variant: scenario.variant,
        repetition: options.repetition,
        lane: lane.clone(),
        repository_path: repo_root.to_string_lossy().into_owned(),
        baseline_checkpoint,
        result_checkpoint,
        started_at_unix_ms,
        tasks,
        task_statuses,
        attempts,
        handoffs,
        reviews,
        actual_changed_paths: grade.changed_paths,
        checks,
        hard_failures,
        trigger,
        metrics,
        orchestration_complete,
        error,
    };
    std::fs::write(
        run_root.join("record.json"),
        serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(record)
}

async fn run_attempt(
    control: &ControlPlane,
    seeded: &SeededRepository,
    baseline_checkpoint: Option<String>,
    request: AttemptRequest,
    attempt_root: &Path,
) -> Result<AttemptRecord, String> {
    control.check_permission(
        request.task.permission_ceiling,
        request.requested_permission,
    )?;
    let attempt_id = Uuid::new_v4().to_string();
    let reservation = control.reserve_spawn(request.task.logical_path.clone())?;
    reservation.commit(attempt_id.clone())?;
    control.set_task_status(&request.task.id, TaskStatus::Running);
    let result = async {
        let _writer_lease = if request.task.mode == TaskMode::Write {
            Some(control.acquire_writer(&request.task.id)?)
        } else {
            None
        };
        let mut provider = ScriptedProvider::new(request.profile.clone());
        provider
            .connect(ProviderConfig::default())
            .await
            .map_err(|error| error.to_string())?;
        let session = provider
            .new_session(SessionOptions {
                cwd: Some(seeded.root.to_string_lossy().into_owned()),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let envelope = ScriptedTaskEnvelope {
            task: request.task.clone(),
            attempt_id: attempt_id.clone(),
            baseline_checkpoint,
            action: request.action,
            fault: request.fault,
        };
        let started = Instant::now();
        let stream = provider
            .prompt(
                &session.id,
                PromptInput::text(
                    serde_json::to_string(&envelope).map_err(|error| error.to_string())?,
                ),
            )
            .await
            .map_err(|error| error.to_string())?;
        let events = tokio::time::timeout(
            SCRIPTED_ATTEMPT_TIMEOUT,
            stream.collect::<Vec<AgentEvent>>(),
        )
        .await
        .map_err(|_| format!("attempt timed out: {attempt_id}"))?;
        let duration_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let artifact = attempt_root.join(format!("{attempt_id}.events.json"));
        std::fs::write(
            &artifact,
            serde_json::to_vec_pretty(&events).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let mut attempt = attempt_from_events(
            &request.profile,
            &request.task,
            &attempt_id,
            &events,
            duration_ms,
        );
        if let Some(handoff) = attempt.handoff.as_mut() {
            handoff.result_checkpoint = checkpoint(seeded).await;
            handoff
                .artifact_refs
                .push(artifact.to_string_lossy().into_owned());
        }
        let tokens = attempt
            .usage
            .input_tokens
            .saturating_add(attempt.usage.output_tokens);
        if let Err(reason) = control.record_usage(tokens) {
            attempt.status = AgentStatus::Errored;
            attempt.error.get_or_insert(reason);
        }
        Ok(attempt)
    }
    .await;
    control.release_agent(&attempt_id);
    result
}

fn collect_attempt(
    control: &ControlPlane,
    attempt: AttemptRecord,
    attempts: &mut Vec<AttemptRecord>,
    handoffs: &mut Vec<StructuredHandoff>,
) {
    if let Some(handoff) = attempt.handoff.clone() {
        if control.report_result(handoff.clone()).is_ok() {
            handoffs.push(handoff);
        }
    }
    attempts.push(attempt);
}

fn valid_writer_handoff(attempt: &AttemptRecord, seeded: &SeededRepository) -> bool {
    let Some(handoff) = &attempt.handoff else {
        return false;
    };
    if attempt.status != AgentStatus::Completed || handoff.reported_status != TaskStatus::Reported {
        return false;
    }
    let Ok(after) = scenarios::snapshot(&seeded.root) else {
        return false;
    };
    !handoff.changed_paths.is_empty()
        && handoff.changed_paths.iter().all(|path| {
            seeded.before_agent.get(path) != after.get(path) && after.contains_key(path)
        })
}

fn reviewer_seeded_bug_files(scenario: &Scenario) -> Vec<scenarios::FileFixture> {
    scenario
        .solution
        .iter()
        .take(1)
        .cloned()
        .map(|mut file| {
            file.content.push_str("\nBENCHMARK_SEEDED_DEFECT\n");
            file
        })
        .collect()
}

fn should_delegate(kind: LaneKind, scenario: &Scenario) -> bool {
    !matches!(kind, LaneKind::Single | LaneKind::PlannedSingle)
        && scenarios::trigger_policy(scenario)
        && !scenario.reader_tasks.is_empty()
}

fn writer_fault(scenario: &Scenario, lane: &LaneSpec) -> ScriptedFault {
    if scenario.faults.contains(&FaultInjection::CrashFirstAttempt) {
        ScriptedFault::Crash
    } else if scenario.faults.contains(&FaultInjection::MissingHandoff) {
        ScriptedFault::MissingHandoff
    } else if scenario.faults.contains(&FaultInjection::FalseHandoff) {
        ScriptedFault::FalseHandoff
    } else if scenario.faults.contains(&FaultInjection::BudgetExhaustion)
        && !matches!(lane.kind, LaneKind::CheapSubagents)
    {
        ScriptedFault::Crash
    } else {
        ScriptedFault::None
    }
}

fn profile(lane: &LaneSpec, role: &str, subagent: bool, cloud: bool) -> ScriptedProfile {
    ScriptedProfile {
        provider: if cloud {
            "scripted-clark-cloud".into()
        } else if matches!(lane.kind, LaneKind::MixedHarness) && subagent {
            "scripted-acp".into()
        } else {
            "scripted-local".into()
        },
        model: if subagent {
            lane.subagent_model
                .clone()
                .unwrap_or_else(|| lane.root_model.clone())
        } else {
            lane.root_model.clone()
        },
        role: role.into(),
        permission_ceiling: if role.starts_with("writer") {
            PermissionCeiling::WorkspaceWrite
        } else {
            PermissionCeiling::ReadOnly
        },
    }
}

#[cfg(test)]
#[path = "coordinator/tests.rs"]
mod tests;
