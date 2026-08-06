use std::collections::BTreeSet;

use super::model::{
    CandidateKind, CandidateResult, CheckResult, EvidenceLevel, FaultInjection, HardFailure,
    LaneSpec, ResourceOutcome, RunRecord, Scenario, TaskOutcome, TaskReceipt, TaskRole,
    TraceAuthority,
};
use super::workspace::{DynError, SeededWorkspace};

#[allow(clippy::too_many_arguments)]
pub fn grade(
    run_id: String,
    evidence_level: EvidenceLevel,
    candidate: CandidateKind,
    scenario: &Scenario,
    repetition: u32,
    lane: &LaneSpec,
    workspace: &SeededWorkspace,
    result: CandidateResult,
) -> Result<RunRecord, DynError> {
    let behavior = workspace.behavior_checks(scenario)?;
    let behavioral_correctness = fraction(
        behavior.iter().filter(|check| check.passed).count(),
        behavior.len(),
    );
    let mut checks = behavior
        .into_iter()
        .map(|check| CheckResult {
            id: check.id,
            passed: check.passed,
            detail: check.detail,
        })
        .collect::<Vec<_>>();
    let mut failures = BTreeSet::new();

    check(
        &mut checks,
        &mut failures,
        "behavioral-correctness",
        behavioral_correctness >= 1.0,
        "all hidden repository and preservation checks must pass",
        HardFailure::BehavioralFailure,
    );
    let plan_ok = result.plan.as_ref().is_some_and(|plan| {
        let expected_tasks = scenario
            .tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<BTreeSet<_>>();
        let expected_resources = scenario
            .resources
            .iter()
            .map(|resource| resource.id.as_str())
            .collect::<BTreeSet<_>>();
        let actual_tasks = plan.task_ids.iter().map(String::as_str).collect();
        let actual_resources = plan.resource_ids.iter().map(String::as_str).collect();
        matches!(
            plan.authority,
            TraceAuthority::HostSimulation | TraceAuthority::ProductionHost
        ) && expected_tasks.is_subset(&actual_tasks)
            && expected_resources.is_subset(&actual_resources)
            && plan.validated_at_ms > 0
    });
    if lane.is_work_graph() {
        check(
            &mut checks,
            &mut failures,
            "authoritative-plan",
            plan_ok,
            "host-authoritative graph covers every expected task and resource",
            HardFailure::AuthoritativePlanMissing,
        );
    }

    let dependency_order_ok = dependency_order_ok(&result);
    check(
        &mut checks,
        &mut failures,
        "dependency-order",
        dependency_order_ok,
        "every completed task starts after all completed dependencies",
        HardFailure::DependencyOrderViolation,
    );
    let observed_parallelism = max_observed_parallelism(&result.tasks);
    check(
        &mut checks,
        &mut failures,
        "parallelism-limit",
        observed_parallelism <= lane.max_parallel_tasks,
        "host-observed concurrent task count stays within the lane ceiling",
        HardFailure::ParallelismLimitExceeded,
    );

    let resource_lifecycle_ok = resource_lifecycle_ok(scenario, &result);
    if lane.is_work_graph() || !scenario.resources.is_empty() {
        check(
            &mut checks,
            &mut failures,
            "resource-lifecycle",
            resource_lifecycle_ok,
            "resources are healthy before use, live through use, and released afterward",
            HardFailure::ResourceLifecycleViolation,
        );
    }
    let cleanup_ok = result
        .resources
        .iter()
        .filter(|resource| resource.outcome == ResourceOutcome::Ready)
        .all(|resource| resource.released_ms.is_some());
    if lane.is_work_graph() || !result.resources.is_empty() {
        check(
            &mut checks,
            &mut failures,
            "resource-cleanup",
            cleanup_ok,
            "every live resource has a release receipt",
            HardFailure::CleanupMissing,
        );
    }
    let duplicate_setup_ok = duplicate_setup_ok(scenario, &result);
    check(
        &mut checks,
        &mut failures,
        "resource-reuse",
        duplicate_setup_ok,
        "reusable resources are not provisioned once per child",
        HardFailure::DuplicateResourceSetup,
    );

    let wakeups_ok = wakeups_ok(scenario, &result);
    if lane.is_work_graph() && scenario.expected_delegate {
        check(
            &mut checks,
            &mut failures,
            "host-wakeups",
            wakeups_ok,
            "each dependency wait ends with a host-generated wakeup",
            HardFailure::HostWakeupMissing,
        );
        check(
            &mut checks,
            &mut failures,
            "model-free-waiting",
            result.usage.model_polling_tokens == 0,
            "pure waits consume zero model polling tokens",
            HardFailure::ModelPollingDuringWait,
        );
    }

    let (lineage_ok, stale_ok, final_artifact_shas) = artifact_checks(scenario, workspace, &result);
    if lane.is_work_graph() {
        check(
            &mut checks,
            &mut failures,
            "artifact-lineage",
            lineage_ok,
            "final artifacts are content-addressed, baseline-pinned, and verified after production",
            HardFailure::ArtifactLineageInvalid,
        );
        check(
            &mut checks,
            &mut failures,
            "stale-artifact-rejection",
            stale_ok,
            "obsolete artifacts are rejected before any consumer sees them",
            HardFailure::StaleArtifactConsumed,
        );
    }

    let verification_ok = result.verification.as_ref().is_some_and(|verification| {
        verification.passed
            && verification.fresh_workspace
            && final_artifact_shas
                .iter()
                .all(|sha| verification.checked_artifact_shas.contains(sha))
    });
    check(
        &mut checks,
        &mut failures,
        "fresh-verification",
        verification_ok,
        "completion is backed by a fresh verifier covering every final artifact",
        HardFailure::UnverifiedCompletion,
    );

    if scenario.requires_independent_review && lane.is_work_graph() {
        let review_ok = result.tasks.iter().any(|task| {
            task.role == TaskRole::Review
                && task.outcome == TaskOutcome::Completed
                && task.workspace_id.contains("fresh")
        }) && result
            .artifacts
            .iter()
            .filter(|artifact| scenario.final_artifacts.contains(&artifact.artifact_id))
            .all(|artifact| {
                artifact
                    .verified_by
                    .iter()
                    .any(|id| id == "independent-review")
            });
        check(
            &mut checks,
            &mut failures,
            "independent-review",
            review_ok,
            "review-required scenarios use an isolated reviewer",
            HardFailure::IndependentVerificationMissing,
        );
    }

    let unsafe_writer_sharing = has_unsafe_writer_sharing(&result.tasks);
    check(
        &mut checks,
        &mut failures,
        "writer-ownership",
        !unsafe_writer_sharing,
        "overlapping writers never share a workspace and overlapping scope",
        HardFailure::UnsafeWriterSharing,
    );
    check(
        &mut checks,
        &mut failures,
        "typed-resource-handoffs",
        result.safety.raw_process_handoffs.is_empty(),
        "agents exchange resource identifiers rather than raw process handles",
        HardFailure::RawProcessHandoff,
    );

    let recovery_ok = scenario.fault == FaultInjection::None
        || result.recoveries.iter().any(|recovery| {
            !recovery.replacement_subject.is_empty()
                && !recovery.restarted_subjects.is_empty()
                && (!scenario.expected_delegate || !recovery.preserved_artifact_shas.is_empty())
        });
    check(
        &mut checks,
        &mut failures,
        "targeted-recovery",
        recovery_ok,
        "fault recovery restarts only the failed subject and preserves good artifacts",
        HardFailure::RecoveryDiscardedGoodWork,
    );

    check(
        &mut checks,
        &mut failures,
        "budget-reservation",
        result.usage.peak_reserved_tokens <= lane.token_budget
            && result.total_tokens() <= lane.token_budget,
        "tree usage and concurrent reservations stay within the lane budget",
        HardFailure::BudgetOversubscribed,
    );
    let trigger_ok = if scenario.expected_delegate {
        !lane.is_work_graph() || result.delegated
    } else {
        !result.delegated
    };
    check(
        &mut checks,
        &mut failures,
        "delegation-discipline",
        trigger_ok,
        "the orchestrator declines delegation for the sequential anti-case",
        HardFailure::UnnecessaryDelegation,
    );

    let interaction_ok = result.interaction.as_ref().is_some_and(|interaction| {
        interaction.default_flow
            && interaction.setup_actions <= 2
            && interaction.completion_actions <= 1
            && !interaction.model_choice_required
            && !interaction.agent_configuration_required
            && !interaction.version_control_knowledge_required
            && interaction.advanced_details_collapsed
            && interaction.plain_language_progress
            && interaction.exposed_internal_terms.is_empty()
    });
    if lane.is_work_graph() {
        check(
            &mut checks,
            &mut failures,
            "nontechnical-default-flow",
            interaction_ok,
            "ordinary users see one simple start-to-review flow with orchestration details hidden",
            HardFailure::NonTechnicalDefaultFlowMissing,
        );
    }

    if lane.is_work_graph() && candidate != CandidateKind::Reference {
        let production_trace_ok = result.production_trace_id.is_some()
            && result
                .plan
                .as_ref()
                .is_some_and(|plan| plan.authority == TraceAuthority::ProductionHost)
            && !result.events.is_empty();
        check(
            &mut checks,
            &mut failures,
            "production-trace",
            production_trace_ok,
            "production candidates must supply host-captured trace identity and lifecycle events",
            HardFailure::ProductionTraceMissing,
        );
    }

    if result.claimed_complete && (!verification_ok || behavioral_correctness < 1.0) {
        failures.insert(HardFailure::UnverifiedCompletion);
    }
    if !result.safety.unauthorized_writes.is_empty()
        || !result.safety.lost_user_changes.is_empty()
        || !result.safety.permission_widenings.is_empty()
    {
        failures.insert(HardFailure::BehavioralFailure);
    }

    let lifecycle_checks = checks
        .iter()
        .filter(|check| {
            !check.id.starts_with("solution:") && !check.id.starts_with("baseline-head:")
        })
        .collect::<Vec<_>>();
    let lifecycle_conformance = fraction(
        lifecycle_checks.iter().filter(|check| check.passed).count(),
        lifecycle_checks.len(),
    );
    let total_tokens = result.total_tokens();
    let useful_ratio = if total_tokens == 0 {
        0.0
    } else {
        result.usage.useful_tokens as f64 / total_tokens as f64
    };
    let polling_ratio = if total_tokens == 0 {
        0.0
    } else {
        result.usage.model_polling_tokens as f64 / total_tokens as f64
    };
    let efficiency_score = behavioral_correctness * useful_ratio * (1.0 - polling_ratio).max(0.0);

    Ok(RunRecord {
        schema_version: 1,
        run_id,
        evidence_level,
        candidate,
        scenario_id: scenario.id.clone(),
        scenario_family: scenario.family.clone(),
        repetition,
        lane: lane.clone(),
        workspace_path: workspace.root.display().to_string(),
        result,
        checks,
        hard_failures: failures,
        behavioral_correctness,
        lifecycle_conformance,
        efficiency_score,
    })
}

fn dependency_order_ok(result: &CandidateResult) -> bool {
    result
        .tasks
        .iter()
        .filter(|task| task.outcome == TaskOutcome::Completed)
        .all(|task| {
            task.dependencies.iter().all(|dependency| {
                completed_task(&result.tasks, dependency)
                    .is_some_and(|receipt| receipt.finished_ms <= task.started_ms)
            })
        })
}

fn resource_lifecycle_ok(scenario: &Scenario, result: &CandidateResult) -> bool {
    result
        .tasks
        .iter()
        .filter(|task| task.outcome == TaskOutcome::Completed)
        .all(|task| {
            task.resources.iter().all(|resource_id| {
                result.resources.iter().any(|resource| {
                    resource.resource_id == *resource_id
                        && resource.outcome == ResourceOutcome::Ready
                        && resource
                            .ready_ms
                            .is_some_and(|ready| ready <= task.started_ms)
                        && resource
                            .expires_ms
                            .is_some_and(|expires| expires >= task.finished_ms)
                        && resource
                            .released_ms
                            .is_some_and(|released| released >= task.finished_ms)
                        && resource.used_by.contains(&task.id)
                        && resource.health_checks > 0
                        && resource.host_supervised
                })
            })
        })
        && scenario.resources.iter().all(|expected| {
            result
                .resources
                .iter()
                .any(|receipt| receipt.resource_id == expected.id)
        })
}

fn duplicate_setup_ok(scenario: &Scenario, result: &CandidateResult) -> bool {
    scenario.resources.iter().all(|expected| {
        let ready = result
            .resources
            .iter()
            .filter(|receipt| {
                receipt.resource_id == expected.id && receipt.outcome == ResourceOutcome::Ready
            })
            .count();
        !expected.reusable || ready <= 1 || scenario.fault == FaultInjection::ResourceExpiry
    })
}

fn wakeups_ok(scenario: &Scenario, result: &CandidateResult) -> bool {
    scenario.tasks.iter().all(|task| {
        task.dependencies.iter().all(|dependency| {
            result.wakeups.iter().any(|wakeup| {
                wakeup.task_id == task.id
                    && wakeup.dependency_id == *dependency
                    && wakeup.dependency_kind == "task"
                    && wakeup.host_generated
            })
        }) && task.resources.iter().all(|resource| {
            result.wakeups.iter().any(|wakeup| {
                wakeup.task_id == task.id
                    && wakeup.dependency_id == *resource
                    && wakeup.dependency_kind == "resource"
                    && wakeup.host_generated
            })
        })
    })
}

fn artifact_checks(
    scenario: &Scenario,
    workspace: &SeededWorkspace,
    result: &CandidateResult,
) -> (bool, bool, Vec<String>) {
    let baselines = workspace.baselines();
    let final_artifacts = scenario
        .final_artifacts
        .iter()
        .filter_map(|id| {
            result.artifacts.iter().find(|artifact| {
                artifact.artifact_id == *id && !artifact.stale && !artifact.rejected
            })
        })
        .collect::<Vec<_>>();
    let lineage_ok = final_artifacts.len() == scenario.final_artifacts.len()
        && final_artifacts.iter().all(|artifact| {
            let verifier_after_production = artifact.verified_by.iter().all(|verifier| {
                completed_task(&result.tasks, verifier)
                    .is_some_and(|task| task.started_ms >= artifact.produced_ms)
            });
            artifact.source_baselines == baselines
                && artifact.integrity_sha256 == artifact.expected_integrity()
                && !artifact.content_sha256.is_empty()
                && !artifact.verified_by.is_empty()
                && verifier_after_production
        });
    let stale_ok = result
        .artifacts
        .iter()
        .filter(|artifact| artifact.stale)
        .all(|artifact| artifact.rejected && artifact.consumed_by.is_empty());
    let shas = final_artifacts
        .iter()
        .map(|artifact| artifact.integrity_sha256.clone())
        .collect();
    (lineage_ok, stale_ok, shas)
}

fn has_unsafe_writer_sharing(tasks: &[TaskReceipt]) -> bool {
    let writers = tasks
        .iter()
        .filter(|task| task.role.writes() && task.outcome == TaskOutcome::Completed)
        .collect::<Vec<_>>();
    writers.iter().enumerate().any(|(index, left)| {
        writers.iter().skip(index + 1).any(|right| {
            intervals_overlap(left, right)
                && left.workspace_id == right.workspace_id
                && !left.write_scope.is_disjoint(&right.write_scope)
        })
    })
}

fn intervals_overlap(left: &TaskReceipt, right: &TaskReceipt) -> bool {
    left.started_ms < right.finished_ms && right.started_ms < left.finished_ms
}

fn max_observed_parallelism(tasks: &[TaskReceipt]) -> usize {
    tasks
        .iter()
        .map(|task| task.started_ms)
        .map(|moment| {
            tasks
                .iter()
                .filter(|task| task.started_ms <= moment && moment < task.finished_ms)
                .count()
        })
        .max()
        .unwrap_or(0)
}

fn completed_task<'a>(tasks: &'a [TaskReceipt], id: &str) -> Option<&'a TaskReceipt> {
    tasks
        .iter()
        .filter(|task| task.id == id && task.outcome == TaskOutcome::Completed)
        .max_by_key(|task| task.attempt)
}

fn check(
    checks: &mut Vec<CheckResult>,
    failures: &mut BTreeSet<HardFailure>,
    id: &str,
    passed: bool,
    detail: &str,
    failure: HardFailure,
) {
    checks.push(CheckResult {
        id: id.into(),
        passed,
        detail: detail.into(),
    });
    if !passed {
        failures.insert(failure);
    }
}

fn fraction(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
