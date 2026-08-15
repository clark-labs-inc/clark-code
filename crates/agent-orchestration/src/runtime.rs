use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::contract::{
    AgentPath, AgentStatus, DeliveryMode, Message, OrchestrationId, ReadOnlyTask, ReportDecision,
    StructuredReport,
};
use crate::control::{ControlPlane, ControlSnapshot};
use crate::harness::{AttemptContext, HarnessAttempt, HarnessError, HarnessEvent, ReadOnlyHarness};
use crate::policy::{AdmissionDecision, AdmissionPolicy, AdmissionRequest};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FanOutRequest {
    pub id: OrchestrationId,
    pub admission: AdmissionRequest,
    pub tasks: Vec<ReadOnlyTask>,
    pub parent_context: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanOutResult {
    pub admission: AdmissionDecision,
    pub control: ControlSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CoordinatorEvent {
    Queued {
        path: AgentPath,
        label: String,
    },
    Running {
        path: AgentPath,
        attempt: u32,
    },
    Harness {
        path: AgentPath,
        detail: HarnessEvent,
    },
    Reported {
        path: AgentPath,
        report: StructuredReport,
    },
    Accepted {
        path: AgentPath,
        attempt: u32,
    },
    ReworkRequested {
        path: AgentPath,
        attempt: u32,
        feedback: String,
    },
    Interrupted {
        path: AgentPath,
    },
    Failed {
        path: AgentPath,
        error: String,
    },
}

pub type CoordinatorEventSink = Arc<dyn Fn(CoordinatorEvent) + Send + Sync>;

#[derive(Debug, Error)]
pub enum CoordinatorError {
    #[error("fan-out admission rejected: {0}")]
    Admission(String),
    #[error("invalid fan-out contract: {0}")]
    InvalidContract(String),
    #[error("control-plane error: {0}")]
    Control(String),
    #[error(transparent)]
    Harness(#[from] HarnessError),
}

pub struct Coordinator {
    policy: AdmissionPolicy,
    control: ControlPlane,
    harnesses: HashMap<String, Arc<dyn ReadOnlyHarness>>,
    active: Arc<Mutex<BTreeMap<AgentPath, CancellationToken>>>,
}

impl Coordinator {
    pub fn new(policy: AdmissionPolicy, control: ControlPlane) -> Self {
        Self {
            policy,
            control,
            harnesses: HashMap::new(),
            active: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn register_harness(&mut self, harness: Arc<dyn ReadOnlyHarness>) -> Result<(), String> {
        let id = harness.id().trim().to_string();
        if id.is_empty() {
            return Err("harness id must not be empty".to_string());
        }
        if self.harnesses.insert(id.clone(), harness).is_some() {
            return Err(format!("duplicate harness id: {id}"));
        }
        Ok(())
    }

    pub async fn run_fanout(
        &self,
        request: FanOutRequest,
        events: CoordinatorEventSink,
    ) -> Result<FanOutResult, CoordinatorError> {
        let admission = self.policy.evaluate(&request.admission);
        if !admission.admitted {
            let details = admission
                .rejections
                .iter()
                .map(|rejection| format!("{}: {}", rejection.code, rejection.detail))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(CoordinatorError::Admission(details));
        }
        self.validate_contract(&request)?;

        // Reserve the entire fan-out before committing any child. Dropping this
        // vector rolls every slot/path back if a later reservation fails.
        let mut reservations = Vec::with_capacity(request.tasks.len());
        for task in request.tasks.iter().cloned() {
            reservations.push(
                self.control
                    .reserve_spawn(&AgentPath::root(), task)
                    .map_err(CoordinatorError::Control)?,
            );
        }
        let mut records = Vec::with_capacity(reservations.len());
        for reservation in reservations {
            let record = reservation
                .commit(Uuid::new_v4().to_string())
                .map_err(CoordinatorError::Control)?;
            events(CoordinatorEvent::Queued {
                path: record.path.clone(),
                label: record.task.objective.clone(),
            });
            records.push(record);
        }

        let mut attempts = FuturesUnordered::new();
        for record in records {
            let harness = self
                .harnesses
                .get(&record.task.harness)
                .expect("contract validation checked harness")
                .clone();
            attempts.push(self.run_attempt(
                request.id.clone(),
                record.path,
                record.task,
                record.attempt,
                request.parent_context.clone(),
                None,
                harness,
                events.clone(),
            ));
        }
        while attempts.next().await.is_some() {}

        Ok(FanOutResult {
            admission,
            control: self.control.snapshot(),
        })
    }

    pub fn accept(
        &self,
        path: &AgentPath,
        events: &CoordinatorEventSink,
    ) -> Result<u32, CoordinatorError> {
        let attempt = self
            .control
            .decide(path, ReportDecision::Accept)
            .map_err(CoordinatorError::Control)?;
        events(CoordinatorEvent::Accepted {
            path: path.clone(),
            attempt,
        });
        Ok(attempt)
    }

    pub async fn rework(
        &self,
        orchestration_id: OrchestrationId,
        path: &AgentPath,
        parent_context: String,
        feedback: String,
        events: CoordinatorEventSink,
    ) -> Result<(), CoordinatorError> {
        if feedback.trim().is_empty() {
            return Err(CoordinatorError::InvalidContract(
                "rework feedback must not be empty".to_string(),
            ));
        }
        if self.control.snapshot().budget.exhausted {
            return Err(CoordinatorError::Control(
                "shared orchestration budget is exhausted".to_string(),
            ));
        }
        let attempt = self
            .control
            .decide(path, ReportDecision::Rework)
            .map_err(CoordinatorError::Control)?;
        let record = self
            .control
            .agent(path)
            .ok_or_else(|| CoordinatorError::Control(format!("unknown agent: {path}")))?;
        self.control
            .send_message(Message {
                sender: AgentPath::root(),
                target: path.clone(),
                body: feedback.clone(),
                mode: DeliveryMode::TriggerTurn,
            })
            .map_err(CoordinatorError::Control)?;
        events(CoordinatorEvent::ReworkRequested {
            path: path.clone(),
            attempt,
            feedback: feedback.clone(),
        });
        let harness = self
            .harnesses
            .get(&record.task.harness)
            .ok_or_else(|| {
                CoordinatorError::InvalidContract(format!(
                    "unknown harness: {}",
                    record.task.harness
                ))
            })?
            .clone();
        self.run_attempt(
            orchestration_id,
            path.clone(),
            record.task,
            attempt,
            parent_context,
            Some(feedback),
            harness,
            events,
        )
        .await;
        Ok(())
    }

    pub fn interrupt(
        &self,
        path: &AgentPath,
        events: &CoordinatorEventSink,
    ) -> Result<AgentStatus, CoordinatorError> {
        let previous = self
            .control
            .agent(path)
            .ok_or_else(|| CoordinatorError::Control(format!("unknown agent: {path}")))?
            .status;
        if let Some(cancel) = self.active.lock().expect("active lock").get(path) {
            cancel.cancel();
        }
        self.control
            .set_status(path, AgentStatus::Interrupted)
            .map_err(CoordinatorError::Control)?;
        events(CoordinatorEvent::Interrupted { path: path.clone() });
        Ok(previous)
    }

    pub fn snapshot(&self) -> ControlSnapshot {
        self.control.snapshot()
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_attempt(
        &self,
        orchestration_id: OrchestrationId,
        path: AgentPath,
        task: ReadOnlyTask,
        attempt: u32,
        parent_context: String,
        feedback: Option<String>,
        harness: Arc<dyn ReadOnlyHarness>,
        events: CoordinatorEventSink,
    ) {
        let cancel = CancellationToken::new();
        self.active
            .lock()
            .expect("active lock")
            .insert(path.clone(), cancel.clone());
        let _ = self.control.set_status(&path, AgentStatus::Running);
        events(CoordinatorEvent::Running {
            path: path.clone(),
            attempt,
        });
        let event_path = path.clone();
        let forwarded_events = events.clone();
        let harness_events = Arc::new(move |detail| {
            forwarded_events(CoordinatorEvent::Harness {
                path: event_path.clone(),
                detail,
            });
        });
        let result = harness
            .run(
                AttemptContext {
                    orchestration_id,
                    agent_path: path.clone(),
                    task: task.clone(),
                    attempt,
                    parent_context,
                    feedback,
                    cancel,
                },
                harness_events,
            )
            .await;
        self.active.lock().expect("active lock").remove(&path);
        match result {
            Ok(attempt_result) => {
                self.finish_attempt(&path, &task, attempt, attempt_result, &events);
            }
            Err(error) => self.fail_attempt(&path, error.to_string(), &events),
        }
    }

    fn finish_attempt(
        &self,
        path: &AgentPath,
        task: &ReadOnlyTask,
        attempt: u32,
        result: HarnessAttempt,
        events: &CoordinatorEventSink,
    ) {
        self.control.budget().record(&result.usage);
        if result.observed_write {
            self.fail_attempt(
                path,
                "read-only harness observed a workspace mutation".to_string(),
                events,
            );
            return;
        }
        let Some(report) = result.report else {
            self.fail_attempt(
                path,
                "agent finished without a structured result report".to_string(),
                events,
            );
            return;
        };
        if let Err(error) = report.validate_read_only(task, attempt) {
            self.fail_attempt(path, error, events);
            return;
        }
        match self.control.report(path, report.clone()) {
            Ok(_) => events(CoordinatorEvent::Reported {
                path: path.clone(),
                report,
            }),
            Err(error) => self.fail_attempt(path, error, events),
        }
    }

    fn fail_attempt(&self, path: &AgentPath, error: String, events: &CoordinatorEventSink) {
        let _ = self.control.fail(path, error.clone());
        events(CoordinatorEvent::Failed {
            path: path.clone(),
            error,
        });
    }

    fn validate_contract(&self, request: &FanOutRequest) -> Result<(), CoordinatorError> {
        let estimated_ids = request
            .admission
            .workstreams
            .iter()
            .map(|workstream| workstream.task_id.clone())
            .collect::<BTreeSet<_>>();
        let task_ids = request
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect::<BTreeSet<_>>();
        if estimated_ids != task_ids || task_ids.len() != request.tasks.len() {
            return Err(CoordinatorError::InvalidContract(
                "tasks and admitted workstream estimates must match one-to-one".to_string(),
            ));
        }
        for task in &request.tasks {
            if task.objective.trim().is_empty()
                || task.scopes.is_empty()
                || task.acceptance.is_empty()
            {
                return Err(CoordinatorError::InvalidContract(format!(
                    "task {} is not concrete and self-contained",
                    task.id
                )));
            }
            let harness = self.harnesses.get(&task.harness).ok_or_else(|| {
                CoordinatorError::InvalidContract(format!("unknown harness: {}", task.harness))
            })?;
            let estimate = request
                .admission
                .workstreams
                .iter()
                .find(|workstream| workstream.task_id == task.id)
                .expect("id sets matched above");
            if harness.kind() != estimate.harness_kind {
                return Err(CoordinatorError::InvalidContract(format!(
                    "harness kind mismatch for task {}",
                    task.id
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use async_trait::async_trait;

    use crate::budget::{BudgetConfig, SharedBudget, UsageCharge};
    use crate::contract::{AgentRole, HarnessKind, ReportStatus, TaskId};
    use crate::harness::{HarnessAttempt, HarnessEventSink};
    use crate::policy::{
        Authorization, ModelRate, OrchestrationPurpose, RiskSignals, WorkstreamEstimate,
    };

    use super::*;

    struct FakeHarness {
        writes: bool,
        reports: bool,
    }

    #[async_trait]
    impl ReadOnlyHarness for FakeHarness {
        fn id(&self) -> &str {
            "local"
        }

        fn kind(&self) -> HarnessKind {
            HarnessKind::Local
        }

        async fn run(
            &self,
            context: AttemptContext,
            _events: HarnessEventSink,
        ) -> Result<HarnessAttempt, HarnessError> {
            Ok(HarnessAttempt {
                provider: "fake".to_string(),
                model: "fake".to_string(),
                final_message: "done".to_string(),
                report: self.reports.then(|| StructuredReport {
                    task_id: context.task.id,
                    attempt: context.attempt,
                    status: ReportStatus::Reported,
                    summary: "evidence".to_string(),
                    changed_paths: BTreeSet::new(),
                    commands: vec![],
                    tests: vec![],
                    claims: vec![crate::contract::ClaimEvidence {
                        claim: "evidence".to_string(),
                        evidence_ref: "src/lib.rs:1".to_string(),
                    }],
                    unresolved: vec![],
                }),
                usage: UsageCharge {
                    input_tokens: 1_000,
                    ..Default::default()
                },
                observed_write: self.writes,
            })
        }
    }

    fn task(id: &str, scope: &str) -> ReadOnlyTask {
        ReadOnlyTask {
            id: TaskId::new(id).unwrap(),
            role: AgentRole::Explorer,
            objective: format!("inspect {scope}"),
            scopes: BTreeSet::from([scope.to_string()]),
            acceptance: vec!["cite evidence".to_string()],
            harness: "local".to_string(),
        }
    }

    fn request(tasks: Vec<ReadOnlyTask>) -> FanOutRequest {
        let workstreams = tasks
            .iter()
            .map(|task| WorkstreamEstimate {
                task_id: task.id.clone(),
                scopes: task.scopes.clone(),
                estimated_context_tokens: 25_000,
                estimated_output_tokens: 1_000,
                harness_kind: HarnessKind::Local,
                model: "fake".to_string(),
                model_rate: None,
            })
            .collect();
        FanOutRequest {
            id: OrchestrationId::new("fanout-1").unwrap(),
            admission: AdmissionRequest {
                authorization: Authorization::UserRequested,
                purpose: OrchestrationPurpose::Explore,
                workstreams,
                root_model: "fake".to_string(),
                root_model_rate: ModelRate {
                    input_per_million_usd: 1.0,
                    output_per_million_usd: 1.0,
                },
                root_estimated_output_tokens: 2_000,
                risk: RiskSignals::default(),
                external_research_required: false,
            },
            tasks,
            parent_context: "overall task".to_string(),
        }
    }

    fn coordinator(harness: FakeHarness) -> Coordinator {
        let control =
            ControlPlane::new(3, 1, SharedBudget::new(BudgetConfig::default()).unwrap()).unwrap();
        let mut coordinator = Coordinator::new(AdmissionPolicy::default(), control);
        coordinator.register_harness(Arc::new(harness)).unwrap();
        coordinator
    }

    #[tokio::test]
    async fn reports_require_parent_acceptance() {
        let coordinator = coordinator(FakeHarness {
            writes: false,
            reports: true,
        });
        let result = coordinator
            .run_fanout(
                request(vec![task("api", "src/api"), task("ui", "src/ui")]),
                Arc::new(|_| {}),
            )
            .await
            .unwrap();
        assert!(result.control.agents.values().all(|agent| {
            agent.status == AgentStatus::Completed && agent.report_status == ReportStatus::Reported
        }));
        let path = AgentPath::parse("/root/api").unwrap();
        let sink: CoordinatorEventSink = Arc::new(|_| {});
        coordinator.accept(&path, &sink).unwrap();
        assert_eq!(
            coordinator.snapshot().agents[&path].report_status,
            ReportStatus::Accepted
        );
    }

    #[tokio::test]
    async fn writes_and_missing_reports_fail_closed() {
        for harness in [
            FakeHarness {
                writes: true,
                reports: true,
            },
            FakeHarness {
                writes: false,
                reports: false,
            },
        ] {
            let coordinator = coordinator(harness);
            let result = coordinator
                .run_fanout(
                    request(vec![task("api", "src/api"), task("ui", "src/ui")]),
                    Arc::new(|_| {}),
                )
                .await
                .unwrap();
            assert!(result.control.agents.values().all(|agent| {
                agent.status == AgentStatus::Errored && agent.report_status == ReportStatus::Failed
            }));
        }
    }
}
