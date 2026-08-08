use super::*;

fn id(value: &str) -> TaskId {
    TaskId::new(value).unwrap()
}

fn repository(value: &str, root: &str, path: &str) -> RepositoryBaseline {
    let repository_id = RepositoryId::new(value).unwrap();
    RepositoryBaseline {
        repository_id,
        repository_fingerprint: format!("fingerprint-{value}"),
        checkout_root: root.into(),
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
    repository_id: Option<&str>,
    dependencies: &[&str],
    allowed: &[&str],
) -> MultiRepoTask {
    let model_tier = match role {
        MultiRepoTaskRole::Reviewer => ModelTier::Reviewer,
        MultiRepoTaskRole::Reader => ModelTier::Cheap,
        _ => ModelTier::Strong,
    };
    MultiRepoTask {
        id: id(value),
        role,
        repository_id: repository_id.map(|value| RepositoryId::new(value).unwrap()),
        dependencies: dependencies.iter().map(|value| id(value)).collect(),
        objective: format!("perform {value}"),
        harness: "local".into(),
        harness_kind: HarnessKind::Local,
        model: match model_tier {
            ModelTier::Cheap => "cheap",
            ModelTier::Strong => "strong",
            ModelTier::Reviewer => "reviewer",
        }
        .into(),
        model_tier,
        budget_reservation: 1_000,
        allowed_changed_paths: allowed.iter().map(|value| (*value).into()).collect(),
    }
}

fn valid_plan() -> MultiRepoPlan {
    let api = repository("api", "/tmp/agent-bench/api", "src/api.rs");
    let sdk = repository("sdk", "/tmp/agent-bench/sdk", "src/sdk.rs");
    MultiRepoPlan {
        repositories: BTreeMap::from([
            (api.repository_id.clone(), api),
            (sdk.repository_id.clone(), sdk),
        ]),
        contracts: vec![RepositoryContractEdge {
            id: "api-contract".into(),
            producer: RepositoryId::new("api").unwrap(),
            consumers: BTreeSet::from([RepositoryId::new("sdk").unwrap()]),
            artifact: "request envelope".into(),
            compatibility_rule: "request_id remains stable".into(),
        }],
        contract_decisions: vec![ContractDecision {
            edge_id: "api-contract".into(),
            decided_by: id("planner"),
            artifact_sha256: "c".repeat(64),
            compatibility_rule: "request_id remains stable".into(),
        }],
        tasks: vec![
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
                &["planner"],
                &["src/sdk.rs"],
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
            argv: vec!["python3".into(), "-c".into(), "pass".into()],
            timeout_ms: 1_000,
        }],
        max_parallel_writers: 2,
        requires_independent_review: true,
    }
}

fn single_repository_plan() -> MultiRepoPlan {
    let app = RepositoryBaseline {
        allowed_changed_paths: BTreeSet::from(["src/engine.rs".into(), "tests/engine.rs".into()]),
        ..repository("app", "/tmp/agent-bench/app", "src/engine.rs")
    };
    MultiRepoPlan {
        repositories: BTreeMap::from([(app.repository_id.clone(), app)]),
        contracts: Vec::new(),
        contract_decisions: Vec::new(),
        tasks: vec![
            task("planner", MultiRepoTaskRole::Planner, None, &[], &[]),
            task(
                "engine-writer",
                MultiRepoTaskRole::Writer,
                Some("app"),
                &["planner"],
                &["src/engine.rs"],
            ),
            task(
                "test-writer",
                MultiRepoTaskRole::Writer,
                Some("app"),
                &["planner"],
                &["tests/engine.rs"],
            ),
            task(
                "integrator",
                MultiRepoTaskRole::Integrator,
                None,
                &["engine-writer", "test-writer"],
                &[],
            ),
        ],
        integration_checks: vec![IntegrationCheck {
            id: "app-tests".into(),
            repository_id: RepositoryId::new("app").unwrap(),
            argv: vec!["cargo".into(), "test".into()],
            timeout_ms: 120_000,
        }],
        max_parallel_writers: 2,
        requires_independent_review: false,
    }
}

#[test]
fn independent_repository_writers_are_selected_for_parallel_delegation() {
    let plan = valid_plan();
    let decision = plan.decomposition_decision().unwrap();
    assert!(decision.delegated);
    assert_eq!(
        decision.parallel_writer_batches,
        vec![vec![id("api-writer"), id("sdk-writer")]]
    );
}

#[test]
fn independent_path_leases_in_one_repository_are_parallelizable() {
    let plan = single_repository_plan();
    plan.validate().unwrap();
    let decision = plan.decomposition_decision().unwrap();
    assert!(decision.delegated);
    assert_eq!(
        decision.parallel_writer_batches,
        vec![vec![id("engine-writer"), id("test-writer")]]
    );
}

#[test]
fn one_repository_writer_leases_must_be_disjoint_and_cover_scope() {
    let mut overlapping = single_repository_plan();
    overlapping
        .tasks
        .iter_mut()
        .find(|task| task.id == id("test-writer"))
        .unwrap()
        .allowed_changed_paths
        .insert("src/engine.rs".into());
    assert_eq!(
        overlapping.validate().unwrap_err(),
        "writer leases overlap within repository app"
    );

    let mut incomplete = single_repository_plan();
    incomplete.tasks.retain(|task| task.id != id("test-writer"));
    incomplete
        .tasks
        .iter_mut()
        .find(|task| task.id == id("integrator"))
        .unwrap()
        .dependencies
        .remove(&id("test-writer"));
    assert_eq!(
        incomplete.validate().unwrap_err(),
        "writer leases must exactly cover repository app change scope"
    );
}

#[test]
fn sequential_writer_chain_declines_parallel_delegation() {
    let mut plan = valid_plan();
    plan.tasks
        .iter_mut()
        .find(|task| task.id == id("sdk-writer"))
        .unwrap()
        .dependencies
        .insert(id("api-writer"));
    let decision = plan.decomposition_decision().unwrap();
    assert!(!decision.delegated);
    assert_eq!(decision.parallel_writer_batches.len(), 2);
    assert!(decision
        .parallel_writer_batches
        .iter()
        .all(|batch| batch.len() == 1));
}

#[test]
fn writer_lease_is_strong_scoped_and_repository_bound() {
    let mut plan = valid_plan();
    let writer = plan
        .tasks
        .iter_mut()
        .find(|task| task.id == id("api-writer"))
        .unwrap();
    writer.model_tier = ModelTier::Cheap;
    assert_eq!(
        plan.validate().unwrap_err(),
        "writer tasks require a strong model tier"
    );

    let mut plan = valid_plan();
    plan.tasks
        .iter_mut()
        .find(|task| task.id == id("api-writer"))
        .unwrap()
        .allowed_changed_paths
        .insert("../sdk/src/sdk.rs".into());
    assert_eq!(
        plan.validate().unwrap_err(),
        "writer lease exceeds its repository path scope"
    );
}

#[test]
fn checkout_selection_rejects_duplicate_identity_and_nested_roots() {
    let mut plan = valid_plan();
    let api_fingerprint = plan.repositories[&RepositoryId::new("api").unwrap()]
        .repository_fingerprint
        .clone();
    plan.repositories
        .get_mut(&RepositoryId::new("sdk").unwrap())
        .unwrap()
        .repository_fingerprint = api_fingerprint;
    assert_eq!(
        plan.validate().unwrap_err(),
        "each selected repository must have a unique stable fingerprint"
    );

    let mut plan = valid_plan();
    plan.repositories
        .get_mut(&RepositoryId::new("sdk").unwrap())
        .unwrap()
        .checkout_root = "/tmp/agent-bench/api/nested".into();
    assert_eq!(
        plan.validate().unwrap_err(),
        "selected repository roots must be disjoint checkout boundaries"
    );
}

#[test]
fn package_descriptor_is_pinned_content_addressed_and_isolated() {
    let plan = valid_plan();
    let mut package = ChangePackageDescriptor {
        task_id: id("api-writer"),
        repository_id: RepositoryId::new("api").unwrap(),
        base_head_oid: "a".repeat(40),
        changed_paths: BTreeSet::from(["src/api.rs".into()]),
        patch_sha256: "d".repeat(64),
        result_tree_sha256: "e".repeat(64),
        artifact_path: "/tmp/artifacts/d.patch".into(),
        isolation: IsolationKind::LocalEphemeralClone,
        checks_run: vec!["cargo test".into()],
    };
    plan.validate_change_package(&package).unwrap();
    package.base_head_oid = "f".repeat(40);
    assert_eq!(
        plan.validate_change_package(&package).unwrap_err(),
        "change package baseline does not match the selected checkout"
    );
}

#[test]
fn repository_result_digest_covers_every_same_repository_package() {
    let first = ChangePackageDescriptor {
        task_id: id("engine-writer"),
        repository_id: RepositoryId::new("app").unwrap(),
        base_head_oid: "a".repeat(40),
        changed_paths: BTreeSet::from(["src/engine.rs".into()]),
        patch_sha256: "b".repeat(64),
        result_tree_sha256: "c".repeat(64),
        artifact_path: "/tmp/first.patch".into(),
        isolation: IsolationKind::LocalEphemeralClone,
        checks_run: Vec::new(),
    };
    let mut second = first.clone();
    second.task_id = id("test-writer");
    second.patch_sha256 = "d".repeat(64);
    second.result_tree_sha256 = "e".repeat(64);
    assert_eq!(
        repository_result_tree_sha256([&first]),
        Some(first.result_tree_sha256.clone())
    );
    assert_ne!(
        repository_result_tree_sha256([&first, &second]),
        repository_result_tree_sha256([&first])
    );
    assert_eq!(
        repository_result_tree_sha256([&second, &first]),
        repository_result_tree_sha256([&first, &second])
    );
}

#[test]
fn every_contract_requires_a_planner_decision_before_writers_run() {
    let mut plan = valid_plan();
    plan.contract_decisions.clear();
    assert!(plan
        .validate()
        .unwrap_err()
        .contains("requires one planner-approved exact decision"));
}
