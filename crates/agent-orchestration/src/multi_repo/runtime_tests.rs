use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::{
    BudgetConfig, CheckoutKind, ContractDecision, IntegrationCheck, IntegrationCheckReceipt,
    ModelTier, RepositoryBaseline,
};

use super::*;

fn id(value: &str) -> TaskId {
    TaskId::new(value).unwrap()
}

fn repo(value: &str, path: &str) -> RepositoryBaseline {
    RepositoryBaseline {
        repository_id: super::super::RepositoryId::new(value).unwrap(),
        repository_fingerprint: format!("fingerprint-{value}"),
        checkout_root: format!("/tmp/runtime/{value}"),
        checkout_kind: CheckoutKind::Main,
        head_oid: "a".repeat(40),
        current_branch: Some("main".into()),
        dirty_tree_sha256: "b".repeat(64),
        allowed_changed_paths: BTreeSet::from([path.into()]),
        cloud_eligible: false,
    }
}

fn task(
    value: &str,
    role: MultiRepoTaskRole,
    repository: Option<&str>,
    dependencies: &[&str],
    allowed: &[&str],
) -> MultiRepoTask {
    MultiRepoTask {
        id: id(value),
        role,
        repository_id: repository.map(|value| super::super::RepositoryId::new(value).unwrap()),
        dependencies: dependencies.iter().map(|value| id(value)).collect(),
        objective: value.into(),
        harness: match role {
            MultiRepoTaskRole::Reviewer => "review",
            MultiRepoTaskRole::Integrator => "integrate",
            _ => "local",
        }
        .into(),
        harness_kind: HarnessKind::Local,
        model: match role {
            MultiRepoTaskRole::Reviewer => "reviewer",
            _ => "strong",
        }
        .into(),
        model_tier: match role {
            MultiRepoTaskRole::Reviewer => ModelTier::Reviewer,
            _ => ModelTier::Strong,
        },
        budget_reservation: 1_000,
        allowed_changed_paths: allowed.iter().map(|path| (*path).into()).collect(),
    }
}

fn plan(review: bool, sequential: bool) -> MultiRepoPlan {
    let api = repo("api", "src/api.rs");
    let sdk = repo("sdk", "src/sdk.rs");
    let mut sdk_dependencies = vec!["planner"];
    if sequential {
        sdk_dependencies.push("api-writer");
    }
    let mut tasks = vec![
        task("planner", MultiRepoTaskRole::Planner, None, &[], &[]),
        task(
            "api-writer",
            MultiRepoTaskRole::Writer,
            Some("api"),
            &["planner"],
            &["src/api.rs"],
        ),
        task(
            "sdk-writer",
            MultiRepoTaskRole::Writer,
            Some("sdk"),
            &sdk_dependencies,
            &["src/sdk.rs"],
        ),
    ];
    if review {
        tasks.push(task(
            "reviewer",
            MultiRepoTaskRole::Reviewer,
            None,
            &["api-writer", "sdk-writer"],
            &[],
        ));
    }
    tasks.push(task(
        "integrator",
        MultiRepoTaskRole::Integrator,
        None,
        if review {
            &["reviewer"]
        } else {
            &["api-writer", "sdk-writer"]
        },
        &[],
    ));
    MultiRepoPlan {
        repositories: BTreeMap::from([
            (api.repository_id.clone(), api),
            (sdk.repository_id.clone(), sdk),
        ]),
        contracts: vec![super::super::RepositoryContractEdge {
            id: "edge".into(),
            producer: super::super::RepositoryId::new("api").unwrap(),
            consumers: BTreeSet::from([super::super::RepositoryId::new("sdk").unwrap()]),
            artifact: "api".into(),
            compatibility_rule: "stable".into(),
        }],
        contract_decisions: vec![ContractDecision {
            edge_id: "edge".into(),
            decided_by: id("planner"),
            artifact_sha256: "c".repeat(64),
            compatibility_rule: "stable".into(),
        }],
        tasks,
        integration_checks: vec![IntegrationCheck {
            id: "fresh-replay-check".into(),
            repository_id: super::super::RepositoryId::new("api").unwrap(),
            argv: vec!["python3".into(), "-c".into(), "pass".into()],
            timeout_ms: 1_000,
        }],
        max_parallel_writers: 2,
        requires_independent_review: review,
    }
}

fn plan_with_cheap_readers() -> MultiRepoPlan {
    let mut plan = plan(false, false);
    for (reader_id, repository, writer_id) in [
        ("api-reader", "api", "api-writer"),
        ("sdk-reader", "sdk", "sdk-writer"),
    ] {
        let mut reader = task(
            reader_id,
            MultiRepoTaskRole::Reader,
            Some(repository),
            &["planner"],
            &[],
        );
        reader.harness = "read".into();
        reader.model = "cheap".into();
        reader.model_tier = ModelTier::Cheap;
        plan.tasks.push(reader);
        plan.tasks
            .iter_mut()
            .find(|task| task.id == id(writer_id))
            .unwrap()
            .dependencies
            .insert(id(reader_id));
    }
    plan
}

struct FakeWriter {
    fail_first: Option<TaskId>,
    attempts: Mutex<BTreeMap<TaskId, u32>>,
    saw_rework_feedback: std::sync::atomic::AtomicBool,
    saw_reader_context: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl MultiRepoWriterHarness for FakeWriter {
    fn id(&self) -> &str {
        "local"
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::Local
    }

    async fn run(
        &self,
        task: MultiRepoTask,
        attempt: u32,
        _cancel: CancellationToken,
    ) -> Result<WriterHarnessAttempt, WriterFailure> {
        tokio::time::sleep(Duration::from_millis(10)).await;
        self.attempts
            .lock()
            .unwrap()
            .insert(task.id.clone(), attempt);
        if attempt > 1
            && task
                .objective
                .contains("Independent review requires targeted rework")
        {
            self.saw_rework_feedback.store(true, Ordering::SeqCst);
        }
        if task.objective.contains("Repository reader findings") {
            self.saw_reader_context.store(true, Ordering::SeqCst);
        }
        if attempt == 1 && self.fail_first.as_ref() == Some(&task.id) {
            return Err(WriterFailure {
                message: "injected crash".into(),
                usage: UsageCharge {
                    input_tokens: 10,
                    ..Default::default()
                },
                reusable_artifact_sha256: Some("f".repeat(64)),
            });
        }
        let repository_id = task.repository_id.clone().unwrap();
        let marker = if repository_id.as_str() == "api" {
            if attempt == 1 {
                'd'
            } else {
                'e'
            }
        } else {
            'f'
        };
        Ok(WriterHarnessAttempt {
            package: super::super::ChangePackageDescriptor {
                task_id: task.id,
                repository_id,
                base_head_oid: "a".repeat(40),
                changed_paths: task.allowed_changed_paths,
                patch_sha256: marker.to_string().repeat(64),
                result_tree_sha256: marker.to_ascii_uppercase().to_string().repeat(64),
                artifact_path: format!("/tmp/{marker}.patch"),
                isolation: super::super::IsolationKind::LocalEphemeralClone,
                checks_run: vec!["test".into()],
            },
            usage: UsageCharge {
                input_tokens: 100,
                output_tokens: 10,
                ..Default::default()
            },
        })
    }
}

struct FakeReader;

#[async_trait]
impl MultiRepoReaderHarness for FakeReader {
    fn id(&self) -> &str {
        "read"
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::Local
    }

    async fn run(
        &self,
        task: MultiRepoTask,
        _attempt: u32,
        _cancel: CancellationToken,
    ) -> Result<ReaderHarnessAttempt, ReaderFailure> {
        Ok(ReaderHarnessAttempt {
            report: ReaderReport {
                task_id: task.id,
                repository_id: task.repository_id.unwrap(),
                summary: "focused repository evidence".into(),
                evidence_refs: vec!["src/file.rs:1".into()],
            },
            usage: UsageCharge {
                input_tokens: 10,
                output_tokens: 2,
                ..Default::default()
            },
        })
    }
}

struct FakeIntegrator {
    baselines: BTreeMap<super::super::RepositoryId, String>,
    checks: Vec<IntegrationCheck>,
    valid: bool,
}

#[async_trait]
impl MultiRepoIntegrationHarness for FakeIntegrator {
    fn id(&self) -> &str {
        "integrate"
    }

    async fn integrate(
        &self,
        _task: MultiRepoTask,
        packages: Vec<super::super::ChangePackageDescriptor>,
        _cancel: CancellationToken,
    ) -> Result<IntegrationHarnessAttempt, String> {
        Ok(IntegrationHarnessAttempt {
            receipt: IntegrationReceipt {
                fresh_workspace: true,
                repository_baselines: self.baselines.clone(),
                repository_result_trees: packages
                    .iter()
                    .map(|package| {
                        (
                            package.repository_id.clone(),
                            package.result_tree_sha256.clone(),
                        )
                    })
                    .collect(),
                applied_patch_sha256: packages
                    .iter()
                    .map(|package| package.patch_sha256.clone())
                    .collect(),
                checks_run: self
                    .checks
                    .iter()
                    .map(|check| check.id.clone())
                    .chain(std::iter::once("fresh replay".into()))
                    .collect(),
                check_receipts: self
                    .checks
                    .iter()
                    .map(|check| IntegrationCheckReceipt {
                        id: check.id.clone(),
                        repository_id: check.repository_id.clone(),
                        argv: check.argv.clone(),
                        started_ms: 1,
                        finished_ms: 2,
                        exit_code: Some(0),
                        stdout_sha256: "a".repeat(64),
                        stderr_sha256: "b".repeat(64),
                        passed: self.valid,
                        error: None,
                    })
                    .collect(),
                passed: self.valid,
            },
            usage: UsageCharge {
                input_tokens: 20,
                ..Default::default()
            },
        })
    }
}

struct FakeReviewer {
    calls: AtomicUsize,
    rework_once: bool,
}

#[async_trait]
impl MultiRepoReviewHarness for FakeReviewer {
    fn id(&self) -> &str {
        "review"
    }

    async fn review(
        &self,
        task: MultiRepoTask,
        packages: Vec<super::super::ChangePackageDescriptor>,
        _attempt: u32,
        _cancel: CancellationToken,
    ) -> Result<ReviewHarnessAttempt, String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let rework = self.rework_once && call == 0;
        Ok(ReviewHarnessAttempt {
            receipt: ReviewReceipt {
                reviewer_task_id: task.id,
                package_sha256: packages
                    .iter()
                    .map(|package| package.patch_sha256.clone())
                    .collect(),
                decision: if rework {
                    ReviewDecision::Rework
                } else {
                    ReviewDecision::Accept
                },
                rework_task_ids: if rework {
                    BTreeSet::from([id("api-writer")])
                } else {
                    BTreeSet::new()
                },
                findings: if rework {
                    vec!["api needs correction".into()]
                } else {
                    Vec::new()
                },
            },
            usage: UsageCharge {
                input_tokens: 30,
                ..Default::default()
            },
        })
    }
}

fn coordinator(
    plan: MultiRepoPlan,
    writer: Arc<FakeWriter>,
    valid_integration: bool,
) -> MultiRepoCoordinator {
    let checks = plan.integration_checks.clone();
    let baselines = plan
        .repositories
        .iter()
        .map(|(id, repository)| (id.clone(), repository.head_oid.clone()))
        .collect();
    let budget = SharedBudget::new(BudgetConfig {
        limit_weighted_tokens: 100_000,
        ..Default::default()
    })
    .unwrap();
    let mut coordinator = MultiRepoCoordinator::new(
        plan,
        budget,
        2,
        Arc::new(FakeIntegrator {
            baselines,
            checks,
            valid: valid_integration,
        }),
    )
    .unwrap();
    if coordinator
        .plan
        .tasks
        .iter()
        .any(|task| task.role == MultiRepoTaskRole::Reader)
    {
        coordinator.register_reader(Arc::new(FakeReader)).unwrap();
    }
    coordinator.register_writer(writer).unwrap();
    coordinator
}

fn writer_harness(fail_first: Option<&str>) -> Arc<FakeWriter> {
    Arc::new(FakeWriter {
        fail_first: fail_first.map(id),
        attempts: Mutex::new(BTreeMap::new()),
        saw_rework_feedback: std::sync::atomic::AtomicBool::new(false),
        saw_reader_context: std::sync::atomic::AtomicBool::new(false),
    })
}

#[tokio::test]
async fn independent_writers_run_and_integrate_with_authoritative_receipts() {
    let coordinator = coordinator(plan(false, false), writer_harness(None), true);
    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(result.passed());
    assert_eq!(result.change_packages.len(), 2);
    assert_eq!(
        result
            .tasks
            .iter()
            .filter(|receipt| receipt.role == MultiRepoTaskRole::Writer)
            .count(),
        2
    );
    assert!(result.budget.weighted_tokens_used > 0.0);
}

#[tokio::test]
async fn cheap_reader_reports_are_validated_and_fed_to_strong_writers() {
    let writer = writer_harness(None);
    let coordinator = coordinator(plan_with_cheap_readers(), writer.clone(), true);
    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(result.passed());
    assert_eq!(result.reader_reports.len(), 2);
    assert!(result
        .tasks
        .iter()
        .filter(|task| task.role == MultiRepoTaskRole::Reader)
        .all(|task| {
            task.model_tier == ModelTier::Cheap && task.outcome == TaskRunOutcome::Completed
        }));
    assert!(writer.saw_reader_context.load(Ordering::SeqCst));
}

#[tokio::test]
async fn one_failed_writer_is_retried_without_restarting_its_sibling() {
    let writer = writer_harness(Some("api-writer"));
    let coordinator = coordinator(plan(false, false), writer.clone(), true);
    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(result.passed());
    assert_eq!(result.recoveries.len(), 1);
    assert_eq!(writer.attempts.lock().unwrap()[&id("api-writer")], 2);
    assert_eq!(writer.attempts.lock().unwrap()[&id("sdk-writer")], 1);
    assert_eq!(result.recoveries[0].preserved_package_sha256.len(), 1);
}

#[tokio::test]
async fn sequential_anti_case_declines_without_starting_harnesses() {
    let writer = writer_harness(None);
    let coordinator = coordinator(plan(false, true), writer.clone(), true);
    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(!result.decomposition.delegated);
    assert_eq!(result.tasks.len(), 1);
    assert_eq!(result.tasks[0].role, MultiRepoTaskRole::Planner);
    assert_eq!(result.planning.planner_task_id, id("planner"));
    assert_eq!(result.planning.plan_sha256.len(), 64);
    assert!(!result.planning.delegated);
    assert!(writer.attempts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn independent_review_reworks_only_the_targeted_package() {
    let writer = writer_harness(None);
    let mut coordinator = coordinator(plan(true, false), writer.clone(), true);
    coordinator
        .register_reviewer(Arc::new(FakeReviewer {
            calls: AtomicUsize::new(0),
            rework_once: true,
        }))
        .unwrap();
    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(result.passed());
    assert_eq!(writer.attempts.lock().unwrap()[&id("api-writer")], 2);
    assert_eq!(writer.attempts.lock().unwrap()[&id("sdk-writer")], 1);
    assert!(writer.saw_rework_feedback.load(Ordering::SeqCst));
    assert_eq!(result.review.unwrap().decision, ReviewDecision::Accept);
}

#[tokio::test]
async fn integration_claim_is_rejected_unless_fresh_replay_passed() {
    let coordinator = coordinator(plan(false, false), writer_harness(None), false);
    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert_eq!(
        result.error.as_deref(),
        Some("integration receipt does not prove exact fresh replay")
    );
    assert!(!result.integration.unwrap().passed);
    assert!(result
        .tasks
        .iter()
        .any(|task| task.role == MultiRepoTaskRole::Integrator));
}
