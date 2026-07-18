use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::{
    ChangePackageDescriptor, MultiRepoCoordinator, MultiRepoCoordinatorEvent, MultiRepoEventSink,
    MultiRepoRunResult, MultiRepoTask, MultiRepoTaskRole, PlanningReceipt, RecoveryReceipt,
    ReviewDecision, ReviewReceipt, TaskExecutionReceipt, TaskRunOutcome, WriterHarnessAttempt,
};
use crate::{TaskId, UsageCharge};

pub(super) struct FailedRunState {
    pub(super) tasks: Vec<TaskExecutionReceipt>,
    pub(super) reader_reports: Vec<super::ReaderReport>,
    pub(super) packages: BTreeMap<TaskId, ChangePackageDescriptor>,
    pub(super) recoveries: Vec<RecoveryReceipt>,
}

impl MultiRepoCoordinator {
    pub(super) fn validate_harnesses(&self) -> Result<(), String> {
        for task in self.plan.tasks.iter().filter(|task| {
            matches!(
                task.role,
                MultiRepoTaskRole::Reader | MultiRepoTaskRole::Writer
            )
        }) {
            let kind = match task.role {
                MultiRepoTaskRole::Reader => self
                    .readers
                    .get(&task.harness)
                    .ok_or_else(|| format!("unknown reader harness: {}", task.harness))?
                    .kind(),
                MultiRepoTaskRole::Writer => self
                    .writers
                    .get(&task.harness)
                    .ok_or_else(|| format!("unknown writer harness: {}", task.harness))?
                    .kind(),
                _ => unreachable!(),
            };
            if kind != task.harness_kind {
                return Err(format!("task harness kind mismatch for {}", task.id));
            }
        }
        if self.plan.requires_independent_review && self.reviewer.is_none() {
            return Err(
                "independent review is required but no review harness is registered".into(),
            );
        }
        Ok(())
    }

    pub(super) fn validate_reader_report(
        &self,
        task: &MultiRepoTask,
        report: &super::ReaderReport,
    ) -> Result<(), String> {
        if report.task_id != task.id
            || task.repository_id.as_ref() != Some(&report.repository_id)
            || report.summary.trim().is_empty()
            || report.evidence_refs.is_empty()
            || report
                .evidence_refs
                .iter()
                .any(|evidence| evidence.trim().is_empty())
        {
            return Err("reader report is incomplete or belongs to the wrong task".into());
        }
        Ok(())
    }

    pub(super) fn accept_package(
        &self,
        task: &MultiRepoTask,
        attempt: WriterHarnessAttempt,
        packages: &mut BTreeMap<TaskId, ChangePackageDescriptor>,
        events: &MultiRepoEventSink,
    ) -> Result<(), String> {
        self.plan.validate_change_package(&attempt.package)?;
        if attempt.package.task_id != task.id {
            return Err("writer harness returned a package for a different task".into());
        }
        events(MultiRepoCoordinatorEvent::PackageAccepted {
            task_id: task.id.clone(),
            patch_sha256: attempt.package.patch_sha256.clone(),
        });
        packages.insert(task.id.clone(), attempt.package);
        Ok(())
    }

    pub(super) fn validate_review(
        &self,
        review: &ReviewReceipt,
        packages: &BTreeMap<TaskId, ChangePackageDescriptor>,
    ) -> Result<(), String> {
        let reviewer = self
            .plan
            .tasks
            .iter()
            .find(|task| task.role == MultiRepoTaskRole::Reviewer)
            .expect("validated review plan");
        if review.reviewer_task_id != reviewer.id {
            return Err("review receipt names the wrong reviewer task".into());
        }
        let expected = packages
            .values()
            .map(|package| package.patch_sha256.clone())
            .collect::<BTreeSet<_>>();
        if review.package_sha256 != expected {
            return Err("review did not cover the exact current package set".into());
        }
        match review.decision {
            ReviewDecision::Accept if !review.rework_task_ids.is_empty() => {
                Err("accepted review cannot request rework".into())
            }
            ReviewDecision::Rework
                if review.rework_task_ids.is_empty()
                    || review.findings.is_empty()
                    || review
                        .rework_task_ids
                        .iter()
                        .any(|task| !packages.contains_key(task)) =>
            {
                Err("review rework must target existing writer tasks".into())
            }
            _ => Ok(()),
        }
    }

    pub(super) fn validate_integration(
        &self,
        integration: &super::IntegrationReceipt,
        packages: &BTreeMap<TaskId, ChangePackageDescriptor>,
    ) -> Result<(), String> {
        let expected_baselines = self
            .plan
            .repositories
            .iter()
            .map(|(id, repository)| (id.clone(), repository.head_oid.clone()))
            .collect::<BTreeMap<_, _>>();
        let expected_patches = packages
            .values()
            .map(|package| package.patch_sha256.clone())
            .collect::<BTreeSet<_>>();
        let expected_trees = packages
            .values()
            .map(|package| {
                (
                    package.repository_id.clone(),
                    package.result_tree_sha256.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let checks_valid = integration.check_receipts.len() == self.plan.integration_checks.len()
            && self.plan.integration_checks.iter().all(|check| {
                integration.check_receipts.iter().any(|receipt| {
                    receipt.id == check.id
                        && receipt.repository_id == check.repository_id
                        && receipt.argv == check.argv
                        && receipt.finished_ms >= receipt.started_ms
                        && receipt.exit_code == Some(0)
                        && receipt.passed
                        && receipt.error.is_none()
                        && valid_sha256(&receipt.stdout_sha256)
                        && valid_sha256(&receipt.stderr_sha256)
                        && integration.checks_run.contains(&receipt.id)
                })
            });
        if !integration.fresh_workspace
            || !integration.passed
            || integration.repository_baselines != expected_baselines
            || integration.repository_result_trees != expected_trees
            || integration
                .applied_patch_sha256
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                != expected_patches
            || integration.applied_patch_sha256.len() != expected_patches.len()
            || integration.checks_run.is_empty()
            || !checks_valid
        {
            return Err("integration receipt does not prove exact fresh replay".into());
        }
        Ok(())
    }

    pub(super) fn failed_result(
        &self,
        decomposition: super::super::DecompositionDecision,
        planning: PlanningReceipt,
        state: FailedRunState,
        error: String,
    ) -> MultiRepoRunResult {
        MultiRepoRunResult {
            decomposition,
            planning,
            tasks: state.tasks,
            reader_reports: state.reader_reports,
            change_packages: state.packages.into_values().collect(),
            recoveries: state.recoveries,
            review: None,
            integration: None,
            budget: self.budget.snapshot(),
            error: Some(error),
        }
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn planning_receipt(
    plan: &super::MultiRepoPlan,
    delegated: bool,
    validated_ms: u64,
) -> Result<PlanningReceipt, String> {
    let planner_task_id = plan
        .tasks
        .iter()
        .find(|task| task.role == MultiRepoTaskRole::Planner)
        .expect("validated plan has a planner")
        .id
        .clone();
    let serialized = serde_json::to_vec(plan)
        .map_err(|error| format!("failed to fingerprint validated plan: {error}"))?;
    Ok(PlanningReceipt {
        planner_task_id,
        plan_sha256: format!("{:x}", Sha256::digest(serialized)),
        repository_baselines: plan
            .repositories
            .iter()
            .map(|(id, repository)| (id.clone(), repository.head_oid.clone()))
            .collect(),
        delegated,
        validated_ms,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn task_receipt(
    task: &MultiRepoTask,
    attempt: u32,
    started_ms: u64,
    finished_ms: u64,
    outcome: TaskRunOutcome,
    usage: UsageCharge,
    error: Option<String>,
) -> TaskExecutionReceipt {
    TaskExecutionReceipt {
        task_id: task.id.clone(),
        role: task.role,
        repository_id: task.repository_id.clone(),
        harness: task.harness.clone(),
        model: task.model.clone(),
        model_tier: task.model_tier,
        attempt,
        started_ms,
        finished_ms,
        outcome,
        usage,
        error,
    }
}

pub(super) fn retry_task_id(task_id: &TaskId, attempt: u32) -> Result<TaskId, String> {
    let suffix = format!("-retry-{attempt}");
    let keep = 64usize.saturating_sub(suffix.len());
    TaskId::new(format!(
        "{}{}",
        &task_id.0[..task_id.0.len().min(keep)],
        suffix
    ))
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
