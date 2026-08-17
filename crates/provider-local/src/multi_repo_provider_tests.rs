use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agent_core::domain::{AgentEvent, ContentBlock, Role, RunOutcome, RunStatus, RunUsage};
use agent_core::error::{Error, Result as CoreResult};
use agent_core::ids::{ProviderId, RunId, SessionId};
use agent_core::provider::{
    ClientResponse, EventStream, PromptInput, ProviderCapabilities, Session, SessionEnvironment,
    SessionOptions,
};
use agent_orchestration::{
    BudgetConfig, ContractDecision, IntegrationCheck, ModelTier, MultiRepoCoordinator,
    RepositoryContractEdge, RepositoryId, ReviewDecision, SharedBudget, TaskId,
};
use base64::Engine;
use futures::stream;
use serde_json::json;

use super::*;
use crate::RepositorySelectionRequest;

#[derive(Default)]
pub(super) struct FakeState {
    pub(super) configs: Mutex<Vec<ProviderConfig>>,
    pub(super) active_writers: AtomicUsize,
    pub(super) maximum_writers: AtomicUsize,
    pub(super) cloud_attachments: AtomicUsize,
    pub(super) write_outside_lease: bool,
    pub(super) writer_paths: BTreeMap<String, String>,
    pub(super) fail_first_writer: Option<String>,
    pub(super) writer_attempts: Mutex<BTreeMap<String, usize>>,
    pub(super) initial_writer_barrier: Option<Arc<tokio::sync::Barrier>>,
}

pub(super) struct FakeProvider {
    pub(super) shared: Arc<FakeState>,
    pub(super) cwd: Option<PathBuf>,
}

#[async_trait]
impl Provider for FakeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("fake")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    async fn connect(&mut self, config: ProviderConfig) -> CoreResult<()> {
        self.shared.configs.lock().unwrap().push(config);
        Ok(())
    }

    async fn new_session(&mut self, options: SessionOptions) -> CoreResult<Session> {
        self.cwd = options.cwd.map(PathBuf::from);
        Ok(Session {
            id: SessionId::new(uuid::Uuid::new_v4().to_string()),
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: options.mode,
            collaboration_mode: options.collaboration_mode.unwrap_or_default(),
            environment: Some(SessionEnvironment::default()),
        })
    }

    async fn load_session(&mut self, _id: SessionId) -> CoreResult<Session> {
        Err(Error::Unsupported("fake provider cannot resume".into()))
    }

    async fn prompt(
        &mut self,
        _session: &SessionId,
        input: PromptInput,
    ) -> CoreResult<EventStream> {
        let text = input
            .blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut terminal_error = None;
        let final_message = if text.contains("brokered cloud repository writer") {
            self.shared
                .cloud_attachments
                .fetch_add(input.attachments.len(), Ordering::SeqCst);
            let patch = b"diff --git a/src/sdk.txt b/src/sdk.txt\n--- a/src/sdk.txt\n+++ b/src/sdk.txt\n@@ -1 +1 @@\n-v1\n+v2\n";
            serde_json::json!({
                "patch_base64": base64::engine::general_purpose::STANDARD.encode(patch)
            })
            .to_string()
        } else if text.contains("bounded, read-only repository reader") {
            r#"{"summary":"found version marker","evidence_refs":["src/file.txt:1"]}"#.to_string()
        } else if text.contains("independent, read-only reviewer") {
            r#"{"decision":"accept","rework_task_ids":[],"findings":[]}"#.to_string()
        } else {
            let cwd = self.cwd.as_ref().expect("fake session cwd");
            let active = self.shared.active_writers.fetch_add(1, Ordering::SeqCst) + 1;
            self.shared
                .maximum_writers
                .fetch_max(active, Ordering::SeqCst);
            let workspace_name = cwd.file_name().unwrap().to_string_lossy();
            let generic_writer = self
                .shared
                .writer_paths
                .iter()
                .find(|(task_id, _)| workspace_name.starts_with(&format!("{task_id}-")))
                .map(|(task_id, path)| (task_id.clone(), path.clone()));
            let writer_task_id = generic_writer
                .as_ref()
                .map(|(task_id, _)| task_id.clone())
                .or_else(|| {
                    ["api-writer", "sdk-writer"]
                        .into_iter()
                        .find(|task_id| workspace_name.starts_with(&format!("{task_id}-")))
                        .map(str::to_owned)
                });
            let generic_attempt = writer_task_id.as_ref().map(|task_id| {
                let mut attempts = self.shared.writer_attempts.lock().unwrap();
                let attempt = attempts.entry(task_id.clone()).or_default();
                *attempt += 1;
                *attempt
            });
            if generic_attempt == Some(1) {
                if let Some(barrier) = &self.shared.initial_writer_barrier {
                    barrier.wait().await;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            if let Some((task_id, path)) = generic_writer {
                let attempt = generic_attempt.expect("generic writer attempt");
                if self.shared.fail_first_writer.as_ref() == Some(&task_id) && attempt == 1 {
                    terminal_error = Some(format!("injected failure for {task_id}"));
                } else {
                    std::fs::write(cwd.join(path), "v2\n").unwrap();
                }
            } else if workspace_name.starts_with("api-writer-") {
                std::fs::write(cwd.join("src/api.txt"), "v2\n").unwrap();
                if self.shared.write_outside_lease {
                    std::fs::write(cwd.join("src/not-leased.txt"), "escape\n").unwrap();
                }
            } else {
                std::fs::write(cwd.join("src/sdk.txt"), "v2\n").unwrap();
            }
            self.shared.active_writers.fetch_sub(1, Ordering::SeqCst);
            "implemented in disposable clone".into()
        };
        let run = RunId::new("fake-run");
        Ok(Box::pin(stream::iter(vec![
            AgentEvent::MessageChunk {
                run: run.clone(),
                role: Role::Agent,
                delta: ContentBlock::text(final_message),
            },
            AgentEvent::RunFinished {
                run,
                outcome: RunOutcome {
                    status: if terminal_error.is_some() {
                        RunStatus::Failed
                    } else {
                        RunStatus::Done
                    },
                    stop_reason: None,
                    error: terminal_error,
                    failure_kind: None,
                    usage: Some(RunUsage {
                        input_tokens: 100,
                        output_tokens: 20,
                        cost_usd: Some(0.001),
                        ..Default::default()
                    }),
                    execution: None,
                },
            },
        ])))
    }

    async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> CoreResult<()> {
        Ok(())
    }

    async fn respond(&mut self, _session: &SessionId, _response: ClientResponse) -> CoreResult<()> {
        Ok(())
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed(root: &Path, path: &str) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join(path), "v1\n").unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Agent Test"]);
    git(root, &["config", "user.email", "test@invalid.local"]);
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "-m", "baseline"]);
}

pub(super) async fn selected(temp: &tempfile::TempDir) -> Arc<RepositorySelection> {
    let api = temp.path().join("api");
    let sdk = temp.path().join("sdk");
    std::fs::create_dir_all(&api).unwrap();
    std::fs::create_dir_all(&sdk).unwrap();
    seed(&api, "src/api.txt");
    seed(&sdk, "src/sdk.txt");
    std::fs::write(api.join("user-note.txt"), "keep me\n").unwrap();
    Arc::new(
        RepositorySelection::resolve(
            &LocalExecutor,
            vec![
                RepositorySelectionRequest {
                    repository_id: RepositoryId::new("api").unwrap(),
                    root: api,
                    allowed_changed_paths: BTreeSet::from(["src/api.txt".into()]),
                    cloud_eligible: false,
                },
                RepositorySelectionRequest {
                    repository_id: RepositoryId::new("sdk").unwrap(),
                    root: sdk,
                    allowed_changed_paths: BTreeSet::from(["src/sdk.txt".into()]),
                    cloud_eligible: true,
                },
            ],
        )
        .await
        .unwrap(),
    )
}

fn task(
    id: &str,
    role: MultiRepoTaskRole,
    repository: Option<&str>,
    dependencies: &[&str],
    allowed: &[&str],
) -> MultiRepoTask {
    MultiRepoTask {
        id: TaskId::new(id).unwrap(),
        role,
        repository_id: repository.map(|value| RepositoryId::new(value).unwrap()),
        dependencies: dependencies
            .iter()
            .map(|value| TaskId::new(*value).unwrap())
            .collect(),
        objective: format!("implement {id}"),
        harness: match role {
            MultiRepoTaskRole::Reader => "read",
            MultiRepoTaskRole::Writer => "writer",
            MultiRepoTaskRole::Reviewer => "review",
            MultiRepoTaskRole::Integrator => "integrate",
            _ => "planner",
        }
        .into(),
        harness_kind: HarnessKind::Local,
        model: match role {
            MultiRepoTaskRole::Reader => "cheap-model",
            MultiRepoTaskRole::Reviewer => "review-model",
            _ => "strong-model",
        }
        .into(),
        model_tier: match role {
            MultiRepoTaskRole::Reader => ModelTier::Cheap,
            MultiRepoTaskRole::Reviewer => ModelTier::Reviewer,
            _ => ModelTier::Strong,
        },
        budget_reservation: 1_000,
        allowed_changed_paths: allowed.iter().map(|value| (*value).into()).collect(),
    }
}

pub(super) fn plan(selection: &RepositorySelection) -> Arc<MultiRepoPlan> {
    Arc::new(MultiRepoPlan {
        repositories: selection.baselines(),
        contracts: vec![RepositoryContractEdge {
            id: "api-sdk".into(),
            producer: RepositoryId::new("api").unwrap(),
            consumers: BTreeSet::from([RepositoryId::new("sdk").unwrap()]),
            artifact: "version marker".into(),
            compatibility_rule: "both repositories use v2".into(),
        }],
        contract_decisions: vec![ContractDecision {
            edge_id: "api-sdk".into(),
            decided_by: TaskId::new("planner").unwrap(),
            artifact_sha256: "a".repeat(64),
            compatibility_rule: "both repositories use v2".into(),
        }],
        tasks: vec![
            task("planner", MultiRepoTaskRole::Planner, None, &[], &[]),
            task(
                "api-reader",
                MultiRepoTaskRole::Reader,
                Some("api"),
                &["planner"],
                &[],
            ),
            task(
                "sdk-reader",
                MultiRepoTaskRole::Reader,
                Some("sdk"),
                &["planner"],
                &[],
            ),
            task(
                "api-writer",
                MultiRepoTaskRole::Writer,
                Some("api"),
                &["planner", "api-reader"],
                &["src/api.txt"],
            ),
            task(
                "sdk-writer",
                MultiRepoTaskRole::Writer,
                Some("sdk"),
                &["planner", "sdk-reader"],
                &["src/sdk.txt"],
            ),
            task(
                "reviewer",
                MultiRepoTaskRole::Reviewer,
                None,
                &["api-writer", "sdk-writer"],
                &[],
            ),
            task(
                "integrator",
                MultiRepoTaskRole::Integrator,
                None,
                &["reviewer"],
                &[],
            ),
        ],
        integration_checks: vec![IntegrationCheck {
            id: "api-tests".into(),
            repository_id: RepositoryId::new("api").unwrap(),
            argv: vec!["git".into(), "diff".into(), "--check".into()],
            timeout_ms: if cfg!(windows) { 30_000 } else { 1_000 },
        }],
        max_parallel_writers: 2,
        requires_independent_review: true,
    })
}

fn runtime(
    temp: &tempfile::TempDir,
    selection: Arc<RepositorySelection>,
    plan: Arc<MultiRepoPlan>,
    shared: Arc<FakeState>,
) -> LocalMultiRepoRuntime {
    LocalMultiRepoRuntime::new(
        LocalMultiRepoRuntimeConfig {
            provider_config: ProviderConfig {
                auth_token: Some("test-only".into()),
                extra: json!({
                    "remote": {"ws_url":"ws://bad", "token":"bad", "cwd":"/bad"},
                    "mcp_servers": [{"name":"bad"}],
                    "memories": true,
                    "browser_enabled": true,
                    "orchestration": {}
                }),
                ..Default::default()
            },
            response_timeout: None,
            scratch_root: temp.path().join("scratch"),
            artifact_root: temp.path().join("artifacts"),
            selection,
            plan,
            integration_gate: None,
        },
        Arc::new(move || {
            Box::new(FakeProvider {
                shared: shared.clone(),
                cwd: None,
            }) as Box<dyn Provider>
        }),
        Arc::new(LocalExecutor),
    )
    .unwrap()
}

#[tokio::test]
async fn production_harness_path_is_parallel_isolated_reviewed_and_replayed() {
    let temp = tempfile::tempdir().unwrap();
    let selection = selected(&temp).await;
    let plan = plan(&selection);
    let shared = Arc::new(FakeState {
        initial_writer_barrier: Some(Arc::new(tokio::sync::Barrier::new(2))),
        ..Default::default()
    });
    let runtime = runtime(&temp, selection.clone(), plan.clone(), shared.clone());
    let integrator = Arc::new(runtime.integration_harness("integrate").unwrap());
    let mut coordinator = MultiRepoCoordinator::new(
        (*plan).clone(),
        SharedBudget::new(BudgetConfig {
            limit_weighted_tokens: Some(10_000),
            ..Default::default()
        })
        .unwrap(),
        integrator,
    )
    .unwrap();
    coordinator
        .register_reader(Arc::new(runtime.reader_harness("read").unwrap()))
        .unwrap();
    coordinator
        .register_writer(Arc::new(runtime.writer_harness("writer").unwrap()))
        .unwrap();
    coordinator
        .register_reviewer(Arc::new(runtime.reviewer_harness("review").unwrap()))
        .unwrap();
    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(result.passed(), "{result:?}");
    assert_eq!(result.change_packages.len(), 2);
    assert_eq!(result.reader_reports.len(), 2);
    assert!(result
        .tasks
        .iter()
        .filter(|task| task.role == MultiRepoTaskRole::Reader)
        .all(|task| task.model_tier == ModelTier::Cheap));
    assert!(shared.maximum_writers.load(Ordering::SeqCst) >= 2);
    assert_eq!(
        result.review.as_ref().unwrap().decision,
        ReviewDecision::Accept
    );
    let integration = result.integration.as_ref().unwrap();
    assert!(integration.fresh_workspace);
    assert_eq!(integration.check_receipts.len(), 1);
    assert_eq!(integration.check_receipts[0].exit_code, Some(0));
    assert!(integration.check_receipts[0].passed);
    assert_eq!(integration.check_receipts[0].stdout_sha256.len(), 64);
    assert_eq!(result.planning.plan_sha256.len(), 64);
    assert_eq!(result.budget.weighted_tokens_reserved, 0.0);

    for selected in selection.repositories().values() {
        let path = if selected.baseline.repository_id.as_str() == "api" {
            "src/api.txt"
        } else {
            "src/sdk.txt"
        };
        assert_eq!(
            std::fs::read_to_string(Path::new(&selected.baseline.checkout_root).join(path))
                .unwrap(),
            "v1\n"
        );
    }
    let api = &selection.repositories()[&RepositoryId::new("api").unwrap()];
    assert_eq!(
        std::fs::read_to_string(Path::new(&api.baseline.checkout_root).join("user-note.txt"))
            .unwrap(),
        "keep me\n"
    );

    let configs = shared.configs.lock().unwrap();
    assert_eq!(configs.len(), 5);
    for config in configs.iter() {
        assert_eq!(config.extra["isolated_writer"], true);
        assert_eq!(config.extra["memories"], false);
        assert_eq!(config.extra["project_knowledge"], false);
        assert_eq!(config.extra["browser_enabled"], false);
        assert_eq!(config.extra["mcp_servers"], json!([]));
        assert!(config.extra.get("remote").is_none());
        assert_eq!(config.extra["permissions"]["bash"], "deny");
    }
}

#[tokio::test]
async fn out_of_lease_provider_write_is_rejected_without_touching_primary() {
    let temp = tempfile::tempdir().unwrap();
    let selection = selected(&temp).await;
    let plan = plan(&selection);
    let shared = Arc::new(FakeState {
        write_outside_lease: true,
        ..Default::default()
    });
    let runtime = runtime(&temp, selection.clone(), plan, shared);
    let harness = runtime.writer_harness("writer").unwrap();
    let api_task = runtime
        .plan
        .tasks
        .iter()
        .find(|task| task.id == TaskId::new("api-writer").unwrap())
        .unwrap()
        .clone();
    let error = harness
        .run(api_task, 1, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(error.message.contains("outside its lease"), "{error:?}");
    selection
        .verify_primaries_unchanged(&LocalExecutor)
        .await
        .unwrap();
    let api = &selection.repositories()[&RepositoryId::new("api").unwrap()];
    assert!(!Path::new(&api.baseline.checkout_root)
        .join("src/not-leased.txt")
        .exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn six_parallel_writers_retry_one_preserve_five_and_apply_all_packages() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("large-parallel-repository");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let writer_paths = (0..6)
        .map(|index| (format!("writer-{index}"), format!("src/part-{index}.txt")))
        .collect::<BTreeMap<_, _>>();
    for path in writer_paths.values() {
        std::fs::write(root.join(path), "v1\n").unwrap();
    }
    git(&root, &["init", "--quiet"]);
    git(&root, &["config", "core.autocrlf", "false"]);
    git(&root, &["config", "user.name", "Agent Large Simulation"]);
    git(
        &root,
        &["config", "user.email", "large-simulation@invalid.local"],
    );
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "large baseline"]);
    std::fs::write(root.join("notes.user"), "preserve this user work\n").unwrap();

    let repository_id = RepositoryId::new("large").unwrap();
    let allowed_changed_paths = writer_paths.values().cloned().collect::<BTreeSet<_>>();
    let selection = Arc::new(
        RepositorySelection::resolve(
            &LocalExecutor,
            vec![RepositorySelectionRequest {
                repository_id: repository_id.clone(),
                root: root.clone(),
                allowed_changed_paths,
                cloud_eligible: false,
            }],
        )
        .await
        .unwrap(),
    );

    let writer_ids = writer_paths.keys().map(String::as_str).collect::<Vec<_>>();
    let mut tasks = vec![task("planner", MultiRepoTaskRole::Planner, None, &[], &[])];
    for (task_id, path) in &writer_paths {
        tasks.push(task(
            task_id,
            MultiRepoTaskRole::Writer,
            Some("large"),
            &["planner"],
            &[path],
        ));
    }
    tasks.push(task(
        "reviewer",
        MultiRepoTaskRole::Reviewer,
        None,
        &writer_ids,
        &[],
    ));
    tasks.push(task(
        "integrator",
        MultiRepoTaskRole::Integrator,
        None,
        &["reviewer"],
        &[],
    ));
    let plan = Arc::new(MultiRepoPlan {
        repositories: selection.baselines(),
        contracts: Vec::new(),
        contract_decisions: Vec::new(),
        tasks,
        integration_checks: vec![IntegrationCheck {
            id: "all-six-parts".into(),
            repository_id: repository_id.clone(),
            argv: vec!["git".into(), "diff".into(), "--check".into()],
            timeout_ms: if cfg!(windows) { 30_000 } else { 5_000 },
        }],
        max_parallel_writers: 6,
        requires_independent_review: true,
    });
    plan.validate().unwrap();

    let shared = Arc::new(FakeState {
        writer_paths: writer_paths.clone(),
        fail_first_writer: Some("writer-3".into()),
        initial_writer_barrier: Some(Arc::new(tokio::sync::Barrier::new(6))),
        ..Default::default()
    });
    let runtime = runtime(&temp, selection.clone(), plan.clone(), shared.clone());
    let integrator = Arc::new(runtime.integration_harness("integrate").unwrap());
    let mut coordinator = MultiRepoCoordinator::new(
        (*plan).clone(),
        SharedBudget::new(BudgetConfig {
            limit_weighted_tokens: Some(20_000),
            ..Default::default()
        })
        .unwrap(),
        integrator,
    )
    .unwrap();
    coordinator
        .register_writer(Arc::new(runtime.writer_harness("writer").unwrap()))
        .unwrap();
    coordinator
        .register_reviewer(Arc::new(runtime.reviewer_harness("review").unwrap()))
        .unwrap();

    let result = coordinator
        .run(CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(result.passed(), "{result:#?}");
    assert_eq!(result.change_packages.len(), 6);
    assert_eq!(result.recoveries.len(), 1);
    assert_eq!(
        result.recoveries[0].failed_task_id,
        TaskId::new("writer-3").unwrap()
    );
    assert_eq!(result.recoveries[0].preserved_package_sha256.len(), 5);
    assert_eq!(shared.maximum_writers.load(Ordering::SeqCst), 6);
    {
        let attempts = shared.writer_attempts.lock().unwrap();
        assert_eq!(attempts["writer-3"], 2);
        assert!(writer_paths
            .keys()
            .filter(|task_id| task_id.as_str() != "writer-3")
            .all(|task_id| attempts[task_id] == 1));
    }

    selection
        .verify_primaries_unchanged(&LocalExecutor)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("notes.user")).unwrap(),
        "preserve this user work\n"
    );
    let application = selection
        .apply_verified_packages(
            &LocalExecutor,
            &plan,
            &result.change_packages,
            &temp.path().join("scratch"),
        )
        .await
        .unwrap();
    assert!(application.head_unchanged);
    assert!(application.preexisting_changes_preserved);
    assert_eq!(application.task_ids.len(), 6);
    assert_eq!(application.changed_paths[&repository_id].len(), 6);
    for path in writer_paths.values() {
        assert_eq!(std::fs::read_to_string(root.join(path)).unwrap(), "v2\n");
    }
    assert_eq!(
        std::fs::read_to_string(root.join("notes.user")).unwrap(),
        "preserve this user work\n"
    );
}
