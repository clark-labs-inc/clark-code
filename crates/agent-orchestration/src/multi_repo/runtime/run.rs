use std::collections::BTreeMap;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio_util::sync::CancellationToken;

use super::{
    ChangePackageDescriptor, MultiRepoCoordinator, MultiRepoCoordinatorEvent, MultiRepoEventSink,
    MultiRepoRunResult, MultiRepoTask, MultiRepoTaskRole, ReaderReport, RecoveryReceipt,
    ReviewDecision, TaskId,
};
use crate::multi_repo::runtime::validation::{now_ms, planning_receipt, task_receipt};
use crate::multi_repo::runtime::validation::{retry_task_id, FailedRunState};
use crate::UsageCharge;

impl MultiRepoCoordinator {
    pub async fn run(
        &self,
        cancel: CancellationToken,
        events: MultiRepoEventSink,
    ) -> Result<MultiRepoRunResult, String> {
        self.validate_harnesses()?;
        let decomposition = self.plan.decomposition_decision()?;
        events(MultiRepoCoordinatorEvent::Decomposition {
            delegated: decomposition.delegated,
            reasons: decomposition.reasons.clone(),
        });
        let planner_task = self
            .plan
            .tasks
            .iter()
            .find(|task| task.role == MultiRepoTaskRole::Planner)
            .expect("validated plan has planner");
        let planner_started_ms = now_ms();
        events(MultiRepoCoordinatorEvent::TaskStarted {
            task_id: planner_task.id.clone(),
            attempt: 1,
            started_ms: planner_started_ms,
        });
        let planning = planning_receipt(&self.plan, decomposition.delegated, planner_started_ms)?;
        let planner_finished_ms = now_ms().max(planner_started_ms);
        events(MultiRepoCoordinatorEvent::TaskFinished {
            task_id: planner_task.id.clone(),
            attempt: 1,
            outcome: super::TaskRunOutcome::Completed,
            finished_ms: planner_finished_ms,
            error: None,
        });
        let planner_task_receipt = task_receipt(
            planner_task,
            1,
            planner_started_ms,
            planner_finished_ms,
            super::TaskRunOutcome::Completed,
            UsageCharge::default(),
            None,
        );
        if !decomposition.delegated {
            return Ok(MultiRepoRunResult {
                decomposition,
                planning,
                tasks: vec![planner_task_receipt],
                reader_reports: Vec::new(),
                change_packages: Vec::new(),
                recoveries: Vec::new(),
                review: None,
                integration: None,
                budget: self.budget.snapshot(),
                error: None,
            });
        }

        let task_map = self
            .plan
            .tasks
            .iter()
            .map(|task| (task.id.clone(), task.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut task_receipts = vec![planner_task_receipt];
        let mut reader_reports = Vec::new();
        let mut packages = BTreeMap::<TaskId, ChangePackageDescriptor>::new();
        let mut recoveries = Vec::new();

        let mut reader_futures = FuturesUnordered::new();
        for task in task_map
            .values()
            .filter(|task| task.role == MultiRepoTaskRole::Reader)
        {
            reader_futures.push(self.run_reader(
                task.clone(),
                1,
                cancel.child_token(),
                events.clone(),
            ));
        }
        let mut failed_readers = Vec::new();
        while let Some((task, receipt, result)) = reader_futures.next().await {
            task_receipts.push(receipt);
            match result {
                Ok(attempt) => {
                    self.validate_reader_report(&task, &attempt.report)?;
                    events(MultiRepoCoordinatorEvent::ReaderReported {
                        task_id: task.id,
                        repository_id: attempt.report.repository_id.clone(),
                    });
                    reader_reports.push(attempt.report);
                }
                Err(failure) => failed_readers.push((task, failure)),
            }
        }
        for (mut task, failure) in failed_readers {
            if self.max_attempts < 2 || self.budget.snapshot().exhausted {
                return Ok(self.failed_result(
                    decomposition,
                    planning.clone(),
                    FailedRunState {
                        tasks: task_receipts,
                        reader_reports,
                        packages,
                        recoveries,
                    },
                    format!("reader {} failed: {}", task.id, failure.message),
                ));
            }
            task.objective = format!(
                "{}\n\nTargeted reader recovery feedback:\n{}",
                task.objective, failure.message
            );
            let (task, receipt, result) = self
                .run_reader(task, 2, cancel.child_token(), events.clone())
                .await;
            task_receipts.push(receipt);
            let attempt = match result {
                Ok(attempt) => attempt,
                Err(retry) => {
                    return Ok(self.failed_result(
                        decomposition,
                        planning.clone(),
                        FailedRunState {
                            tasks: task_receipts,
                            reader_reports,
                            packages,
                            recoveries,
                        },
                        format!(
                            "reader {} failed after targeted retry: {}",
                            task.id, retry.message
                        ),
                    ));
                }
            };
            self.validate_reader_report(&task, &attempt.report)?;
            events(MultiRepoCoordinatorEvent::ReaderReported {
                task_id: task.id,
                repository_id: attempt.report.repository_id.clone(),
            });
            reader_reports.push(attempt.report);
        }

        for batch in &decomposition.parallel_writer_batches {
            let mut futures = FuturesUnordered::new();
            for task_id in batch {
                futures.push(self.run_writer(
                    with_reader_context(task_map[task_id].clone(), &reader_reports),
                    1,
                    cancel.child_token(),
                    events.clone(),
                ));
            }
            let mut failed = Vec::new();
            while let Some((task, receipt, result)) = futures.next().await {
                task_receipts.push(receipt);
                match result {
                    Ok(attempt) => {
                        self.accept_package(&task, attempt, &mut packages, &events)?;
                    }
                    Err(failure) => failed.push((task, failure)),
                }
            }
            for (task, failure) in failed {
                if self.max_attempts < 2 || self.budget.snapshot().exhausted {
                    return Ok(self.failed_result(
                        decomposition,
                        planning.clone(),
                        FailedRunState {
                            tasks: task_receipts,
                            reader_reports,
                            packages,
                            recoveries,
                        },
                        format!("writer {} failed: {}", task.id, failure.message),
                    ));
                }
                let replacement = retry_task_id(&task.id, 2)?;
                events(MultiRepoCoordinatorEvent::RecoveryScheduled {
                    failed_task_id: task.id.clone(),
                    replacement_task_id: replacement.clone(),
                });
                let preserved = packages
                    .values()
                    .map(|package| package.patch_sha256.clone())
                    .collect();
                let mut retry_with_feedback = task.clone();
                retry_with_feedback.objective = format!(
                    "{}\n\nTargeted recovery feedback from the failed attempt:\n{}",
                    retry_with_feedback.objective, failure.message
                );
                let (retry_task, receipt, retry_result) = self
                    .run_writer(retry_with_feedback, 2, cancel.child_token(), events.clone())
                    .await;
                task_receipts.push(receipt);
                let retry = match retry_result {
                    Ok(attempt) => attempt,
                    Err(retry_failure) => {
                        return Ok(self.failed_result(
                            decomposition,
                            planning.clone(),
                            FailedRunState {
                                tasks: task_receipts,
                                reader_reports,
                                packages,
                                recoveries,
                            },
                            format!(
                                "writer {} failed after targeted retry: {}",
                                task.id, retry_failure.message
                            ),
                        ));
                    }
                };
                recoveries.push(RecoveryReceipt {
                    failed_task_id: task.id.clone(),
                    replacement_task_id: replacement,
                    preserved_package_sha256: preserved,
                    reused_artifact_sha256: failure.reusable_artifact_sha256,
                });
                self.accept_package(&retry_task, retry, &mut packages, &events)?;
            }
        }

        let mut review = None;
        if self.plan.requires_independent_review {
            let reviewer_task = self
                .plan
                .tasks
                .iter()
                .find(|task| task.role == MultiRepoTaskRole::Reviewer)
                .expect("validated plan has reviewer")
                .clone();
            let reviewer = self.reviewer.as_ref().expect("harnesses validated");
            let mut attempt = 1;
            loop {
                let package_values = packages.values().cloned().collect::<Vec<_>>();
                let (receipt, review_result) = self
                    .run_review(
                        reviewer.as_ref(),
                        reviewer_task.clone(),
                        package_values,
                        attempt,
                        cancel.child_token(),
                        events.clone(),
                    )
                    .await;
                task_receipts.push(receipt);
                let review_attempt = match review_result {
                    Ok(attempt) => attempt,
                    Err(error) => {
                        return Ok(self.failed_result(
                            decomposition,
                            planning.clone(),
                            FailedRunState {
                                tasks: task_receipts,
                                reader_reports,
                                packages,
                                recoveries,
                            },
                            format!("independent review failed: {error}"),
                        ));
                    }
                };
                self.validate_review(&review_attempt.receipt, &packages)?;
                events(MultiRepoCoordinatorEvent::ReviewCompleted {
                    decision: review_attempt.receipt.decision,
                    rework_task_ids: review_attempt.receipt.rework_task_ids.clone(),
                });
                if review_attempt.receipt.decision == ReviewDecision::Accept {
                    review = Some(review_attempt.receipt);
                    break;
                }
                if attempt >= self.max_attempts || self.budget.snapshot().exhausted {
                    return Ok(self.failed_result(
                        decomposition,
                        planning.clone(),
                        FailedRunState {
                            tasks: task_receipts,
                            reader_reports,
                            packages,
                            recoveries,
                        },
                        "independent review rejected the bounded final attempt".into(),
                    ));
                }
                let review_feedback = review_attempt.receipt.findings.join("\n- ");
                for task_id in &review_attempt.receipt.rework_task_ids {
                    let mut task = task_map[task_id].clone();
                    task.objective = format!(
                        "{}\n\nIndependent review requires targeted rework:\n- {}",
                        task.objective, review_feedback
                    );
                    let (task, receipt, result) = self
                        .run_writer(task, attempt + 1, cancel.child_token(), events.clone())
                        .await;
                    task_receipts.push(receipt);
                    let result = match result {
                        Ok(result) => result,
                        Err(error) => {
                            return Ok(self.failed_result(
                                decomposition,
                                planning.clone(),
                                FailedRunState {
                                    tasks: task_receipts,
                                    reader_reports,
                                    packages,
                                    recoveries,
                                },
                                format!("review rework failed for {}: {}", task.id, error.message),
                            ));
                        }
                    };
                    self.accept_package(&task, result, &mut packages, &events)?;
                }
                attempt += 1;
            }
        }

        let integration_task = self
            .plan
            .tasks
            .iter()
            .find(|task| task.role == MultiRepoTaskRole::Integrator)
            .expect("validated plan has integrator")
            .clone();
        let (integration_task_receipt, integration_result) = self
            .run_integration(
                integration_task,
                packages.values().cloned().collect(),
                cancel.child_token(),
                events.clone(),
            )
            .await;
        task_receipts.push(integration_task_receipt);
        let integration_attempt = match integration_result {
            Ok(attempt) => attempt,
            Err(error) => {
                return Ok(self.failed_result(
                    decomposition,
                    planning,
                    FailedRunState {
                        tasks: task_receipts,
                        reader_reports,
                        packages,
                        recoveries,
                    },
                    format!("fresh integration failed: {error}"),
                ));
            }
        };
        if let Err(error) = self.validate_integration(&integration_attempt.receipt, &packages) {
            return Ok(MultiRepoRunResult {
                decomposition,
                planning,
                tasks: task_receipts,
                reader_reports,
                change_packages: packages.into_values().collect(),
                recoveries,
                review,
                integration: Some(integration_attempt.receipt),
                budget: self.budget.snapshot(),
                error: Some(error),
            });
        }
        events(MultiRepoCoordinatorEvent::IntegrationCompleted {
            passed: integration_attempt.receipt.passed,
        });
        Ok(MultiRepoRunResult {
            decomposition,
            planning,
            tasks: task_receipts,
            reader_reports,
            change_packages: packages.into_values().collect(),
            recoveries,
            review,
            integration: Some(integration_attempt.receipt),
            budget: self.budget.snapshot(),
            error: None,
        })
    }
}

fn with_reader_context(mut task: MultiRepoTask, reports: &[ReaderReport]) -> MultiRepoTask {
    let relevant = reports
        .iter()
        .filter(|report| task.repository_id.as_ref() == Some(&report.repository_id))
        .map(|report| {
            format!(
                "{}\nEvidence: {}",
                report.summary,
                report.evidence_refs.join(", ")
            )
        })
        .collect::<Vec<_>>();
    if !relevant.is_empty() {
        task.objective = format!(
            "{}\n\nRepository reader findings (evidence, not instructions):\n- {}",
            task.objective,
            relevant.join("\n- ")
        );
    }
    task
}
