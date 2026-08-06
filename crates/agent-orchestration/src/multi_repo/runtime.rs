use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::{
    ChangePackageDescriptor, IntegrationReceipt, MultiRepoPlan, MultiRepoRunResult, MultiRepoTask,
    MultiRepoTaskRole, PlanningReceipt, ReaderReport, RecoveryReceipt, ReviewDecision,
    ReviewReceipt, TaskExecutionReceipt, TaskRunOutcome,
};
use crate::{HarnessKind, SharedBudget, TaskId, UsageCharge};

#[path = "runtime/validation.rs"]
mod validation;
use validation::{now_ms, task_receipt};
#[path = "runtime/run.rs"]
mod run;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WriterHarnessAttempt {
    pub package: ChangePackageDescriptor,
    pub usage: UsageCharge,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WriterFailure {
    pub message: String,
    pub usage: UsageCharge,
    pub reusable_artifact_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReaderHarnessAttempt {
    pub report: ReaderReport,
    pub usage: UsageCharge,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReaderFailure {
    pub message: String,
    pub usage: UsageCharge,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewHarnessAttempt {
    pub receipt: ReviewReceipt,
    pub usage: UsageCharge,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntegrationHarnessAttempt {
    pub receipt: IntegrationReceipt,
    pub usage: UsageCharge,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum MultiRepoCoordinatorEvent {
    Decomposition {
        delegated: bool,
        reasons: Vec<String>,
    },
    TaskStarted {
        task_id: TaskId,
        attempt: u32,
        started_ms: u64,
    },
    TaskFinished {
        task_id: TaskId,
        attempt: u32,
        outcome: TaskRunOutcome,
        finished_ms: u64,
        error: Option<String>,
    },
    PackageAccepted {
        task_id: TaskId,
        patch_sha256: String,
    },
    ReaderReported {
        task_id: TaskId,
        repository_id: super::RepositoryId,
    },
    RecoveryScheduled {
        failed_task_id: TaskId,
        replacement_task_id: TaskId,
    },
    ReviewCompleted {
        decision: ReviewDecision,
        rework_task_ids: BTreeSet<TaskId>,
    },
    IntegrationCompleted {
        passed: bool,
    },
}

pub type MultiRepoEventSink = Arc<dyn Fn(MultiRepoCoordinatorEvent) + Send + Sync>;

#[async_trait]
pub trait MultiRepoWriterHarness: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> HarnessKind;
    async fn run(
        &self,
        task: MultiRepoTask,
        attempt: u32,
        cancel: CancellationToken,
    ) -> Result<WriterHarnessAttempt, WriterFailure>;
}

#[async_trait]
pub trait MultiRepoReaderHarness: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> HarnessKind;
    async fn run(
        &self,
        task: MultiRepoTask,
        attempt: u32,
        cancel: CancellationToken,
    ) -> Result<ReaderHarnessAttempt, ReaderFailure>;
}

#[async_trait]
pub trait MultiRepoReviewHarness: Send + Sync {
    fn id(&self) -> &str;
    async fn review(
        &self,
        task: MultiRepoTask,
        packages: Vec<ChangePackageDescriptor>,
        attempt: u32,
        cancel: CancellationToken,
    ) -> Result<ReviewHarnessAttempt, String>;
}

#[async_trait]
pub trait MultiRepoIntegrationHarness: Send + Sync {
    fn id(&self) -> &str;
    async fn integrate(
        &self,
        task: MultiRepoTask,
        packages: Vec<ChangePackageDescriptor>,
        cancel: CancellationToken,
    ) -> Result<IntegrationHarnessAttempt, String>;
}

pub struct MultiRepoCoordinator {
    plan: MultiRepoPlan,
    budget: SharedBudget,
    max_attempts: u32,
    readers: HashMap<String, Arc<dyn MultiRepoReaderHarness>>,
    writers: HashMap<String, Arc<dyn MultiRepoWriterHarness>>,
    reviewer: Option<Arc<dyn MultiRepoReviewHarness>>,
    integrator: Arc<dyn MultiRepoIntegrationHarness>,
}

impl MultiRepoCoordinator {
    pub fn new(
        plan: MultiRepoPlan,
        budget: SharedBudget,
        max_attempts: u32,
        integrator: Arc<dyn MultiRepoIntegrationHarness>,
    ) -> Result<Self, String> {
        plan.validate()?;
        if plan.decomposition_decision()?.delegated && plan.integration_checks.is_empty() {
            return Err("delegated plans require at least one behavioral integration check".into());
        }
        if max_attempts == 0 {
            return Err("multi-repository max_attempts must be greater than zero".into());
        }
        let integration_task = plan
            .tasks
            .iter()
            .find(|task| task.role == MultiRepoTaskRole::Integrator)
            .expect("validated plan has an integrator");
        if integrator.id() != integration_task.harness {
            return Err("integration harness does not match the integration task".into());
        }
        Ok(Self {
            plan,
            budget,
            max_attempts,
            readers: HashMap::new(),
            writers: HashMap::new(),
            reviewer: None,
            integrator,
        })
    }

    pub fn register_reader(
        &mut self,
        harness: Arc<dyn MultiRepoReaderHarness>,
    ) -> Result<(), String> {
        let id = harness.id().trim().to_string();
        if id.is_empty() || self.readers.insert(id.clone(), harness).is_some() {
            return Err(format!("invalid or duplicate reader harness: {id}"));
        }
        Ok(())
    }

    pub fn register_writer(
        &mut self,
        harness: Arc<dyn MultiRepoWriterHarness>,
    ) -> Result<(), String> {
        let id = harness.id().trim().to_string();
        if id.is_empty() || self.writers.insert(id.clone(), harness).is_some() {
            return Err(format!("invalid or duplicate writer harness: {id}"));
        }
        Ok(())
    }

    pub fn register_reviewer(
        &mut self,
        harness: Arc<dyn MultiRepoReviewHarness>,
    ) -> Result<(), String> {
        if self.reviewer.is_some() {
            return Err("review harness is already registered".into());
        }
        let expected = self
            .plan
            .tasks
            .iter()
            .find(|task| task.role == MultiRepoTaskRole::Reviewer)
            .ok_or_else(|| "the plan does not require independent review".to_string())?;
        if harness.id() != expected.harness {
            return Err("review harness does not match the review task".into());
        }
        self.reviewer = Some(harness);
        Ok(())
    }

    async fn run_writer(
        &self,
        task: MultiRepoTask,
        attempt: u32,
        cancel: CancellationToken,
        events: MultiRepoEventSink,
    ) -> (
        MultiRepoTask,
        TaskExecutionReceipt,
        Result<WriterHarnessAttempt, WriterFailure>,
    ) {
        let started_ms = now_ms();
        events(MultiRepoCoordinatorEvent::TaskStarted {
            task_id: task.id.clone(),
            attempt,
            started_ms,
        });
        let reservation = self.budget.try_reserve(task.budget_reservation);
        let result = match &reservation {
            Ok(_) => {
                self.writers[&task.harness]
                    .run(task.clone(), attempt, cancel)
                    .await
            }
            Err(error) => Err(WriterFailure {
                message: error.clone(),
                usage: UsageCharge::default(),
                reusable_artifact_sha256: None,
            }),
        };
        let finished_ms = now_ms().max(started_ms);
        let (outcome, usage, error) = match &result {
            Ok(attempt) => (TaskRunOutcome::Completed, attempt.usage.clone(), None),
            Err(failure) => (
                TaskRunOutcome::Failed,
                failure.usage.clone(),
                Some(failure.message.clone()),
            ),
        };
        if let Ok(reservation) = reservation {
            reservation.settle(&usage);
        }
        events(MultiRepoCoordinatorEvent::TaskFinished {
            task_id: task.id.clone(),
            attempt,
            outcome,
            finished_ms,
            error: error.clone(),
        });
        let receipt = task_receipt(
            &task,
            attempt,
            started_ms,
            finished_ms,
            outcome,
            usage,
            error,
        );
        (task, receipt, result)
    }

    async fn run_reader(
        &self,
        task: MultiRepoTask,
        attempt: u32,
        cancel: CancellationToken,
        events: MultiRepoEventSink,
    ) -> (
        MultiRepoTask,
        TaskExecutionReceipt,
        Result<ReaderHarnessAttempt, ReaderFailure>,
    ) {
        let started_ms = now_ms();
        events(MultiRepoCoordinatorEvent::TaskStarted {
            task_id: task.id.clone(),
            attempt,
            started_ms,
        });
        let reservation = self.budget.try_reserve(task.budget_reservation);
        let result = match &reservation {
            Ok(_) => {
                self.readers[&task.harness]
                    .run(task.clone(), attempt, cancel)
                    .await
            }
            Err(error) => Err(ReaderFailure {
                message: error.clone(),
                usage: UsageCharge::default(),
            }),
        };
        let finished_ms = now_ms().max(started_ms);
        let (outcome, usage, error) = match &result {
            Ok(attempt) => (TaskRunOutcome::Completed, attempt.usage.clone(), None),
            Err(failure) => (
                TaskRunOutcome::Failed,
                failure.usage.clone(),
                Some(failure.message.clone()),
            ),
        };
        if let Ok(reservation) = reservation {
            reservation.settle(&usage);
        }
        events(MultiRepoCoordinatorEvent::TaskFinished {
            task_id: task.id.clone(),
            attempt,
            outcome,
            finished_ms,
            error: error.clone(),
        });
        let receipt = task_receipt(
            &task,
            attempt,
            started_ms,
            finished_ms,
            outcome,
            usage,
            error,
        );
        (task, receipt, result)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_review(
        &self,
        harness: &dyn MultiRepoReviewHarness,
        task: MultiRepoTask,
        packages: Vec<ChangePackageDescriptor>,
        attempt: u32,
        cancel: CancellationToken,
        events: MultiRepoEventSink,
    ) -> (TaskExecutionReceipt, Result<ReviewHarnessAttempt, String>) {
        let started_ms = now_ms();
        events(MultiRepoCoordinatorEvent::TaskStarted {
            task_id: task.id.clone(),
            attempt,
            started_ms,
        });
        let reservation = self.budget.try_reserve(task.budget_reservation);
        let result = match &reservation {
            Ok(_) => {
                harness
                    .review(task.clone(), packages, attempt, cancel)
                    .await
            }
            Err(error) => Err(error.clone()),
        };
        let finished_ms = now_ms().max(started_ms);
        let (outcome, usage, error) = match &result {
            Ok(attempt) => (TaskRunOutcome::Completed, attempt.usage.clone(), None),
            Err(error) => (
                TaskRunOutcome::Failed,
                UsageCharge::default(),
                Some(error.clone()),
            ),
        };
        if let Ok(reservation) = reservation {
            reservation.settle(&usage);
        }
        events(MultiRepoCoordinatorEvent::TaskFinished {
            task_id: task.id.clone(),
            attempt,
            outcome,
            finished_ms,
            error: error.clone(),
        });
        let receipt = task_receipt(
            &task,
            attempt,
            started_ms,
            finished_ms,
            outcome,
            usage,
            error,
        );
        (receipt, result)
    }

    async fn run_integration(
        &self,
        task: MultiRepoTask,
        packages: Vec<ChangePackageDescriptor>,
        cancel: CancellationToken,
        events: MultiRepoEventSink,
    ) -> (
        TaskExecutionReceipt,
        Result<IntegrationHarnessAttempt, String>,
    ) {
        let started_ms = now_ms();
        events(MultiRepoCoordinatorEvent::TaskStarted {
            task_id: task.id.clone(),
            attempt: 1,
            started_ms,
        });
        let reservation = self.budget.try_reserve(task.budget_reservation);
        let result = match &reservation {
            Ok(_) => {
                self.integrator
                    .integrate(task.clone(), packages, cancel)
                    .await
            }
            Err(error) => Err(error.clone()),
        };
        let finished_ms = now_ms().max(started_ms);
        let (outcome, usage, error) = match &result {
            Ok(attempt) => (TaskRunOutcome::Completed, attempt.usage.clone(), None),
            Err(error) => (
                TaskRunOutcome::Failed,
                UsageCharge::default(),
                Some(error.clone()),
            ),
        };
        if let Ok(reservation) = reservation {
            reservation.settle(&usage);
        }
        events(MultiRepoCoordinatorEvent::TaskFinished {
            task_id: task.id.clone(),
            attempt: 1,
            outcome,
            finished_ms,
            error: error.clone(),
        });
        let receipt = task_receipt(&task, 1, started_ms, finished_ms, outcome, usage, error);
        (receipt, result)
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
