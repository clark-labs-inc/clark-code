use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use agent_orchestration::{
    ContractDecision, HarnessKind, IntegrationCheck, ModelTier, MultiRepoPlan, MultiRepoTask,
    MultiRepoTaskRole, RepositoryContractEdge, RepositoryId, TaskId,
};

use super::*;
use crate::exec::{Executor, LocalExecutor};

fn run_git(root: &Path, args: &[&str]) {
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

fn seed_repository(root: &Path, path: &str, content: &str) {
    std::fs::create_dir_all(root.join(Path::new(path).parent().unwrap())).unwrap();
    std::fs::write(root.join(path), content).unwrap();
    run_git(root, &["init", "--quiet"]);
    run_git(root, &["config", "user.name", "Clark Test"]);
    run_git(root, &["config", "user.email", "test@invalid.local"]);
    run_git(root, &["add", "--all"]);
    run_git(root, &["commit", "--quiet", "-m", "baseline"]);
}

fn task_id(value: &str) -> TaskId {
    TaskId::new(value).unwrap()
}

fn writer(repository_id: &str, path: &str) -> MultiRepoTask {
    MultiRepoTask {
        id: task_id(&format!("{repository_id}-writer")),
        role: MultiRepoTaskRole::Writer,
        repository_id: Some(RepositoryId::new(repository_id).unwrap()),
        dependencies: BTreeSet::from([task_id("planner")]),
        objective: format!("update {repository_id}"),
        harness: "local".into(),
        harness_kind: HarnessKind::Local,
        model: "strong".into(),
        model_tier: ModelTier::Strong,
        budget_reservation: 1_000,
        allowed_changed_paths: BTreeSet::from([path.into()]),
    }
}

fn global_task(value: &str, role: MultiRepoTaskRole, dependencies: &[&str]) -> MultiRepoTask {
    MultiRepoTask {
        id: task_id(value),
        role,
        repository_id: None,
        dependencies: dependencies.iter().map(|value| task_id(value)).collect(),
        objective: value.into(),
        harness: "local".into(),
        harness_kind: HarnessKind::Local,
        model: "strong".into(),
        model_tier: ModelTier::Strong,
        budget_reservation: 1_000,
        allowed_changed_paths: BTreeSet::new(),
    }
}

fn plan(selection: &RepositorySelection) -> MultiRepoPlan {
    MultiRepoPlan {
        repositories: selection.baselines(),
        contracts: vec![RepositoryContractEdge {
            id: "api-sdk".into(),
            producer: RepositoryId::new("api").unwrap(),
            consumers: BTreeSet::from([RepositoryId::new("sdk").unwrap()]),
            artifact: "text contract".into(),
            compatibility_rule: "both files use v2".into(),
        }],
        contract_decisions: vec![ContractDecision {
            edge_id: "api-sdk".into(),
            decided_by: task_id("planner"),
            artifact_sha256: "a".repeat(64),
            compatibility_rule: "both files use v2".into(),
        }],
        tasks: vec![
            global_task("planner", MultiRepoTaskRole::Planner, &[]),
            writer("api", "src/api.txt"),
            writer("sdk", "src/sdk.txt"),
            global_task(
                "integrator",
                MultiRepoTaskRole::Integrator,
                &["api-writer", "sdk-writer"],
            ),
        ],
        integration_checks: vec![IntegrationCheck {
            id: "api-tests".into(),
            repository_id: RepositoryId::new("api").unwrap(),
            argv: vec!["python3".into(), "-c".into(), "pass".into()],
            timeout_ms: 1_000,
        }],
        max_parallel_writers: 2,
        requires_independent_review: false,
    }
}

async fn selection(temp: &tempfile::TempDir) -> RepositorySelection {
    let api = temp.path().join("api");
    let sdk = temp.path().join("sdk");
    std::fs::create_dir_all(&api).unwrap();
    std::fs::create_dir_all(&sdk).unwrap();
    seed_repository(&api, "src/api.txt", "v1\n");
    seed_repository(&sdk, "src/sdk.txt", "v1\n");
    std::fs::create_dir_all(api.join("notes")).unwrap();
    std::fs::write(api.join("notes/local.txt"), "user work\n").unwrap();
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
                cloud_eligible: false,
            },
        ],
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn isolated_packages_replay_without_touching_dirty_primary_checkouts() {
    let temp = tempfile::tempdir().unwrap();
    let selection = selection(&temp).await;
    let plan = plan(&selection);
    plan.validate().unwrap();
    assert!(plan.decomposition_decision().unwrap().delegated);
    let scratch = temp.path().join("scratch");
    let artifacts = scratch.join("artifacts");
    let mut packages = Vec::new();
    for (repository, path) in [("api", "src/api.txt"), ("sdk", "src/sdk.txt")] {
        let workspace = IsolatedWriterWorkspace::create(
            &LocalExecutor,
            &selection,
            writer(repository, path),
            &scratch,
        )
        .await
        .unwrap();
        LocalExecutor
            .write(&workspace.root.join(path), b"v2\n")
            .await
            .unwrap();
        packages.push(
            workspace
                .package(
                    &LocalExecutor,
                    &plan,
                    &artifacts,
                    vec!["fixture check".into()],
                )
                .await
                .unwrap(),
        );
    }
    selection
        .verify_primaries_unchanged(&LocalExecutor)
        .await
        .unwrap();
    let primary_api = &selection.repositories()[&RepositoryId::new("api").unwrap()];
    assert_eq!(
        std::fs::read_to_string(Path::new(&primary_api.baseline.checkout_root).join("src/api.txt"))
            .unwrap(),
        "v1\n"
    );
    assert_eq!(
        std::fs::read_to_string(
            Path::new(&primary_api.baseline.checkout_root).join("notes/local.txt")
        )
        .unwrap(),
        "user work\n"
    );

    let integration =
        FreshIntegrationWorkspace::replay(&LocalExecutor, &selection, &plan, &packages, &scratch)
            .await
            .unwrap();
    assert!(integration.receipt().passed);
    assert_eq!(integration.receipt().applied_patch_sha256.len(), 2);
    for (repository, path) in [("api", "src/api.txt"), ("sdk", "src/sdk.txt")] {
        let root = &integration.repository_roots[&RepositoryId::new(repository).unwrap()];
        assert_eq!(LocalExecutor.read(&root.join(path)).await.unwrap(), b"v2\n");
    }
}

#[tokio::test]
async fn writer_package_fails_closed_on_an_out_of_scope_path() {
    let temp = tempfile::tempdir().unwrap();
    let selection = selection(&temp).await;
    let plan = plan(&selection);
    let scratch = temp.path().join("scratch");
    let workspace = IsolatedWriterWorkspace::create(
        &LocalExecutor,
        &selection,
        writer("api", "src/api.txt"),
        &scratch,
    )
    .await
    .unwrap();
    LocalExecutor
        .write(&workspace.root.join("src/escape.txt"), b"not leased\n")
        .await
        .unwrap();
    let error = workspace
        .package(
            &LocalExecutor,
            &plan,
            &scratch.join("artifacts"),
            Vec::new(),
        )
        .await
        .unwrap_err();
    assert!(error.contains("outside its lease"));
}

#[tokio::test]
async fn selection_rejects_the_same_repository_through_two_roots() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    seed_repository(&root, "src/lib.txt", "v1\n");
    let error = RepositorySelection::resolve(
        &LocalExecutor,
        vec![
            RepositorySelectionRequest {
                repository_id: RepositoryId::new("one").unwrap(),
                root: root.clone(),
                allowed_changed_paths: BTreeSet::from(["src/lib.txt".into()]),
                cloud_eligible: false,
            },
            RepositorySelectionRequest {
                repository_id: RepositoryId::new("two").unwrap(),
                root: root.join("src"),
                allowed_changed_paths: BTreeSet::from(["src/lib.txt".into()]),
                cloud_eligible: false,
            },
        ],
    )
    .await
    .unwrap_err();
    assert!(error.contains("overlap") || error.contains("same repository identity"));
}

#[test]
fn selection_baselines_serialize_without_private_workspace_contents() {
    let baseline = agent_orchestration::RepositoryBaseline {
        repository_id: RepositoryId::new("api").unwrap(),
        repository_fingerprint: "fingerprint".into(),
        checkout_root: "/tmp/api".into(),
        checkout_kind: agent_orchestration::CheckoutKind::Main,
        head_oid: "a".repeat(40),
        current_branch: Some("main".into()),
        dirty_tree_sha256: "b".repeat(64),
        allowed_changed_paths: BTreeSet::from(["src/api.rs".into()]),
        cloud_eligible: false,
    };
    let serialized = serde_json::to_string(&BTreeMap::from([(
        baseline.repository_id.clone(),
        baseline,
    )]))
    .unwrap();
    assert!(!serialized.contains("user work"));
}
