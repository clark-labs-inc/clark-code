use super::*;
use crate::tools::{ReadTracker, ToolCtx};
use crate::{background::BackgroundTasks, exec::LocalExecutor, loop_state::SessionState};
use agent_core::domain::{AgentEvent, RunStatus, RunUsage};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use agent_orchestration::{CheckoutKind, RepositoryBaseline};
use futures::StreamExt;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

#[path = "orchestration_writer_large_paid_tests.rs"]
mod large_paid;

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

fn seed_synthetic_project(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(
        root.join("src/math_ops.py"),
        "def add(a, b):\n    return 0\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/greetings.py"),
        "def greet(name):\n    return \"\"\n",
    )
    .unwrap();
    std::fs::write(root.join("src/__init__.py"), "").unwrap();
    std::fs::write(
        root.join("tests/test_contract.py"),
        "import unittest\nfrom src.math_ops import add\nfrom src.greetings import greet\n\nclass ContractTest(unittest.TestCase):\n    def test_add(self):\n        self.assertEqual(add(2, 3), 5)\n\n    def test_greet(self):\n        self.assertEqual(greet('Ada'), 'Hello, Ada!')\n",
    )
    .unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Clark Paid Eval"]);
    git(root, &["config", "user.email", "eval@invalid.local"]);
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "-m", "synthetic baseline"]);
}

fn paid_config() -> (String, String, String) {
    let api_key = std::env::var("CLARK_CODE_API_KEY")
        .or_else(|_| std::env::var("CLARK_API_KEY"))
        .expect("CLARK_CODE_API_KEY or CLARK_API_KEY must be set");
    let model =
        std::env::var("CLARK_PAID_EVAL_MODEL").unwrap_or_else(|_| "clark-code:minimax_m3".into());
    let base_url = std::env::var("CLARK_PAID_EVAL_BASE_URL")
        .unwrap_or_else(|_| crate::config::DEFAULT_BASE_URL.into());
    (api_key, model, base_url)
}

fn config() -> OrchestrationToolsConfig {
    OrchestrationToolsConfig {
        policy: crate::orchestration::OrchestrationConfig {
            enabled: true,
            max_agents: 3,
            ..Default::default()
        },
        base_url: "https://example.invalid/v1".into(),
        api_key: None,
        headers: HashMap::new(),
        root_model: "strong".into(),
        reasoning_effort: None,
    }
}

fn absolute_test_root() -> &'static str {
    if cfg!(windows) {
        "C:/clark-test/workspace"
    } else {
        "/tmp/workspace"
    }
}

#[test]
fn writer_schema_orders_contract_before_environment_and_file_leases() {
    let schema = tools(config())[0].parameters();
    let properties = schema["properties"].as_object().unwrap();
    assert_eq!(
        properties.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "objective",
            "integration_checks",
            "resources",
            "workstreams",
            "independent_review"
        ]
    );
    let resources = properties["resources"]["items"]["properties"]
        .as_object()
        .unwrap();
    assert_eq!(
        resources.keys().map(String::as_str).collect::<Vec<_>>(),
        ["id", "workdir", "command", "output_contains", "timeout_ms"]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        properties["workstreams"]["items"]["additionalProperties"],
        false
    );
}

#[test]
fn builds_a_parallel_single_repository_plan_with_exact_leases() {
    let repository_id = RepositoryId::new("workspace").unwrap();
    let selection = RepositorySelection::from_test_baselines(BTreeMap::from([(
        repository_id.clone(),
        RepositoryBaseline {
            repository_id: repository_id.clone(),
            repository_fingerprint: "fingerprint".into(),
            checkout_root: absolute_test_root().into(),
            checkout_kind: CheckoutKind::Main,
            head_oid: "a".repeat(40),
            current_branch: Some("main".into()),
            dirty_tree_sha256: "b".repeat(64),
            allowed_changed_paths: BTreeSet::from(["src/a.rs".into(), "src/b.rs".into()]),
            cloud_eligible: false,
        },
    )]));
    let plan = build_plan(
        &SharedState {
            config: config(),
            pending: Mutex::new(HashMap::new()),
        },
        DelegateArgs {
            objective: "implement both halves".into(),
            workstreams: vec![
                WorkstreamArgs {
                    id: "a-writer".into(),
                    objective: "implement a".into(),
                    paths: BTreeSet::from(["src/a.rs".into()]),
                    dependencies: BTreeSet::new(),
                },
                WorkstreamArgs {
                    id: "b-writer".into(),
                    objective: "implement b".into(),
                    paths: BTreeSet::from(["src/b.rs".into()]),
                    dependencies: BTreeSet::new(),
                },
            ],
            resources: Vec::new(),
            integration_checks: vec![CheckArgs {
                id: "tests".into(),
                argv: vec!["cargo".into(), "test".into()],
                timeout_ms: 1_000,
            }],
            independent_review: false,
        },
        &selection,
        repository_id,
    )
    .unwrap();
    assert!(plan.decomposition_decision().unwrap().delegated);
    let writer_reservations = plan
        .tasks
        .iter()
        .filter(|task| task.role == MultiRepoTaskRole::Writer)
        .map(|task| task.budget_reservation)
        .sum::<u64>();
    assert!(writer_reservations <= config().policy.token_budget * 9 / 10);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "paid live-model evaluation; run only with explicit user authorization"]
async fn paid_single_repo_workstreams_complete_and_apply() {
    let (api_key, model, base_url) = paid_config();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("synthetic-python-project");
    seed_synthetic_project(&root);

    let shared = Arc::new(SharedState {
        config: OrchestrationToolsConfig {
            policy: crate::orchestration::OrchestrationConfig {
                enabled: true,
                max_agents: 2,
                max_attempts: 1,
                token_budget: 60_000,
                ..Default::default()
            },
            base_url,
            api_key: Some(api_key),
            headers: HashMap::new(),
            root_model: model.clone(),
            reasoning_effort: Some("low".into()),
        },
        pending: Mutex::new(HashMap::new()),
    });
    let ctx = ToolCtx {
        sandbox: Arc::new(crate::sandbox::Sandbox::new(&root).unwrap()),
        executor: Arc::new(LocalExecutor),
        reads: Arc::new(Mutex::new(ReadTracker::default())),
        cancel: tokio_util::sync::CancellationToken::new(),
        background: Arc::new(BackgroundTasks::default()),
        session: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
        progress: None,
        agent_progress: None,
        call_progress: None,
    };
    let started = Instant::now();
    let outcome = run_workstreams(
        &shared,
        DelegateArgs {
            objective: "Make the two public Python functions satisfy the existing contract tests without changing tests or adding dependencies.".into(),
            workstreams: vec![
                WorkstreamArgs {
                    id: "math-writer".into(),
                    objective: "Implement add(a, b) correctly in src/math_ops.py.".into(),
                    paths: BTreeSet::from(["src/math_ops.py".into()]),
                    dependencies: BTreeSet::new(),
                },
                WorkstreamArgs {
                    id: "greeting-writer".into(),
                    objective: "Implement greet(name) in src/greetings.py so it returns exactly Hello, {name}!".into(),
                    paths: BTreeSet::from(["src/greetings.py".into()]),
                    dependencies: BTreeSet::new(),
                },
            ],
            resources: vec![resources::ResourceArgs {
                id: "verification-environment".into(),
                command: "printf ENVIRONMENT_READY; sleep 30".into(),
                output_contains: Some("ENVIRONMENT_READY".into()),
                workdir: None,
                timeout_ms: 2_000,
            }],
            integration_checks: vec![CheckArgs {
                id: "python-contract".into(),
                argv: vec![
                    "python3".into(),
                    "-m".into(),
                    "unittest".into(),
                    "discover".into(),
                    "-s".into(),
                    "tests".into(),
                ],
                timeout_ms: 30_000,
            }],
            independent_review: false,
        },
        &ctx,
    )
    .await
    .unwrap();
    assert!(!outcome.is_error, "{}", outcome.content);
    let body: Value = serde_json::from_str(&outcome.content).unwrap();
    let run_id = body["run_id"].as_str().unwrap();
    let pending = shared.pending.lock().unwrap().get(run_id).cloned().unwrap();
    let receipt = pending
        .selection
        .apply_verified_packages(
            ctx.executor.as_ref(),
            &pending.plan,
            &pending.result.change_packages,
            &pending.scratch_root,
        )
        .await
        .unwrap();
    let verification = Command::new("python3")
        .current_dir(&root)
        .args(["-m", "unittest", "discover", "-s", "tests"])
        .output()
        .unwrap();
    assert!(
        verification.status.success(),
        "{}",
        String::from_utf8_lossy(&verification.stderr)
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "model": model,
            "passed": true,
            "wall_ms": started.elapsed().as_millis(),
            "task_receipts": pending.result.tasks,
            "budget": pending.result.budget,
            "application": receipt
        }))
        .unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "paid live-model evaluation; run only with explicit user authorization"]
async fn paid_single_agent_control_completes_the_same_contract() {
    let (api_key, model, base_url) = paid_config();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("synthetic-python-control");
    seed_synthetic_project(&root);
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            cwd: Some(root.to_string_lossy().into_owned()),
            auth_token: Some(api_key),
            extra: json!({
                "base_url": base_url,
                "model": model,
                "reasoning_effort": "low",
                "temperature": 0.0,
                "max_iterations": 96,
                "permissions": {
                    "write_file": "allow",
                    "edit_file": "allow",
                    "apply_patch": "allow",
                    "bash": "allow",
                    "bash_input": "allow",
                    "bash_kill": "allow"
                },
                "orchestration": false,
                "research": false,
                "memories": false,
                "project_knowledge": false,
                "browser_enabled": false,
                "mcp_servers": []
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(root.to_string_lossy().into_owned()),
            mode: Some("auto".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let started = Instant::now();
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text(
                "Make the two public Python functions satisfy the existing contract tests without changing tests or adding dependencies. Implement add(a, b) in src/math_ops.py and greet(name) in src/greetings.py so greet('Ada') returns exactly Hello, Ada! Run the existing unittest suite and finish only when it passes.",
            ),
        )
        .await
        .unwrap();
    let mut usage = RunUsage::default();
    let mut status = None;
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::RunFinished { outcome, .. } => {
                usage = outcome.usage.unwrap_or_default();
                status = Some(outcome.status);
            }
            AgentEvent::PermissionRequest { request } => {
                panic!(
                    "unexpected permission request in paid control: {}",
                    request.title
                )
            }
            _ => {}
        }
    }
    assert_eq!(status, Some(RunStatus::Done));
    let verification = Command::new("python3")
        .current_dir(&root)
        .args(["-m", "unittest", "discover", "-s", "tests"])
        .output()
        .unwrap();
    assert!(
        verification.status.success(),
        "{}",
        String::from_utf8_lossy(&verification.stderr)
    );
    let changed = Command::new("git")
        .current_dir(&root)
        .args(["diff", "--name-only"])
        .output()
        .unwrap();
    let changed = String::from_utf8(changed.stdout).unwrap();
    assert_eq!(
        changed.lines().collect::<BTreeSet<_>>(),
        BTreeSet::from(["src/greetings.py", "src/math_ops.py"])
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "model": model,
            "passed": true,
            "wall_ms": started.elapsed().as_millis(),
            "usage": usage,
            "changed_paths": changed.lines().collect::<Vec<_>>()
        }))
        .unwrap()
    );
}
