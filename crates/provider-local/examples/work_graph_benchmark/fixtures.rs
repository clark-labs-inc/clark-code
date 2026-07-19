use std::collections::BTreeSet;

use super::model::{
    FaultInjection, FileFixture, ProjectSpec, ResourceSpec, Scenario, TaskRole, TaskSpec,
};

#[path = "fixtures_large.rs"]
mod large;

pub fn catalog() -> Vec<Scenario> {
    vec![
        toolchain_bootstrap(),
        generated_contract_pipeline(),
        service_migration_health(),
        reusable_build_cache(),
        remote_compute_integration(),
        resource_recovery(),
        resource_lease_expiry(),
        worker_recovery(),
        baseline_drift(),
        large::parallel_feature_recovery(),
        sequential_anti_case(),
    ]
}

fn toolchain_bootstrap() -> Scenario {
    scenario(
        "toolchain-bootstrap-fix",
        "resource_and_code",
        "Prepare a toolchain while diagnosing and fixing a compiler regression",
        vec![project("compiler", false, false)],
        vec![resource("toolchain", "build_toolchain", 900, 4_000, true)],
        vec![
            task(
                "inspect-config",
                TaskRole::Inspect,
                &[],
                &[],
                &[],
                &[],
                260,
                12_000,
                false,
            ),
            task(
                "build-toolchain",
                TaskRole::Provision,
                &[],
                &["toolchain"],
                &[],
                &[],
                100,
                4_000,
                false,
            ),
            task(
                "implement-fix",
                TaskRole::Implement,
                &["inspect-config"],
                &["toolchain"],
                &["compiler-patch"],
                &["compiler/state.txt"],
                620,
                28_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["implement-fix"],
                &["toolchain"],
                &[],
                &[],
                280,
                9_000,
                false,
            ),
        ],
        &["compiler-patch"],
        true,
        false,
        FaultInjection::None,
    )
}

fn generated_contract_pipeline() -> Scenario {
    scenario(
        "generated-contract-pipeline",
        "artifact_dependency",
        "Change a public contract, regenerate its client, and update the consumer",
        vec![
            project("contract", false, false),
            project("generator", false, false),
            project("consumer", false, false),
        ],
        vec![resource(
            "generator-runtime",
            "code_generator",
            420,
            3_000,
            true,
        )],
        vec![
            task(
                "inspect-contract",
                TaskRole::Inspect,
                &[],
                &[],
                &[],
                &[],
                220,
                9_000,
                false,
            ),
            task(
                "change-contract",
                TaskRole::Implement,
                &["inspect-contract"],
                &[],
                &["contract-patch"],
                &["contract/state.txt"],
                440,
                22_000,
                false,
            ),
            task(
                "generate-client",
                TaskRole::Generate,
                &["change-contract"],
                &["generator-runtime"],
                &["generated-client"],
                &["generator/state.txt"],
                360,
                16_000,
                false,
            ),
            task(
                "update-consumer",
                TaskRole::Implement,
                &["generate-client"],
                &[],
                &["consumer-patch"],
                &["consumer/state.txt"],
                480,
                24_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["update-consumer"],
                &[],
                &[],
                &[],
                300,
                10_000,
                false,
            ),
        ],
        &["contract-patch", "generated-client", "consumer-patch"],
        true,
        true,
        FaultInjection::None,
    )
}

fn service_migration_health() -> Scenario {
    scenario(
        "service-migration-health",
        "live_service",
        "Prepare a database, apply a migration, update a service, and prove it is healthy",
        vec![
            project("schema", false, false),
            project("service", false, false),
        ],
        vec![resource("database", "ephemeral_database", 700, 3_000, true)],
        vec![
            task(
                "migration",
                TaskRole::Implement,
                &[],
                &[],
                &["migration-patch"],
                &["schema/state.txt"],
                520,
                25_000,
                false,
            ),
            task(
                "prepare-db",
                TaskRole::Provision,
                &[],
                &["database"],
                &[],
                &[],
                100,
                4_000,
                false,
            ),
            task(
                "update-service",
                TaskRole::Implement,
                &["migration"],
                &["database"],
                &["service-patch"],
                &["service/state.txt"],
                560,
                28_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["update-service"],
                &["database"],
                &[],
                &[],
                260,
                8_000,
                false,
            ),
        ],
        &["migration-patch", "service-patch"],
        true,
        false,
        FaultInjection::None,
    )
}

fn reusable_build_cache() -> Scenario {
    scenario(
        "reusable-build-cache",
        "resource_reuse",
        "Fix two independent packages using one prepared build cache, then integrate them",
        vec![
            project("package-a", false, false),
            project("package-b", false, false),
        ],
        vec![resource("build-cache", "build_cache", 640, 4_000, true)],
        vec![
            task(
                "prepare-cache",
                TaskRole::Provision,
                &[],
                &["build-cache"],
                &[],
                &[],
                90,
                3_000,
                false,
            ),
            task(
                "fix-package-a",
                TaskRole::Implement,
                &[],
                &["build-cache"],
                &["package-a-patch"],
                &["package-a/state.txt"],
                540,
                25_000,
                false,
            ),
            task(
                "fix-package-b",
                TaskRole::Implement,
                &[],
                &["build-cache"],
                &["package-b-patch"],
                &["package-b/state.txt"],
                580,
                27_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["fix-package-a", "fix-package-b"],
                &["build-cache"],
                &[],
                &[],
                300,
                10_000,
                false,
            ),
        ],
        &["package-a-patch", "package-b-patch"],
        true,
        false,
        FaultInjection::None,
    )
}

fn remote_compute_integration() -> Scenario {
    scenario(
        "remote-compute-integration",
        "mixed_environment",
        "Use a remote compute worker for validation while implementing the local client",
        vec![
            project("kernel", true, false),
            project("client", false, false),
        ],
        vec![resource(
            "compute-worker",
            "remote_compute",
            1_100,
            4_500,
            true,
        )],
        vec![
            task(
                "inspect-client",
                TaskRole::Inspect,
                &[],
                &[],
                &[],
                &[],
                260,
                9_000,
                false,
            ),
            task(
                "validate-kernel",
                TaskRole::Implement,
                &[],
                &["compute-worker"],
                &["kernel-patch"],
                &["kernel/state.txt"],
                660,
                29_000,
                true,
            ),
            task(
                "update-client",
                TaskRole::Implement,
                &["inspect-client"],
                &[],
                &["client-patch"],
                &["client/state.txt"],
                520,
                26_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["validate-kernel", "update-client"],
                &[],
                &[],
                &[],
                320,
                11_000,
                false,
            ),
        ],
        &["kernel-patch", "client-patch"],
        true,
        true,
        FaultInjection::None,
    )
}

fn resource_recovery() -> Scenario {
    scenario(
        "targeted-resource-recovery",
        "recovery",
        "Keep a completed code change when the verification environment fails and restarts",
        vec![
            project("engine", false, true),
            project("adapter", false, false),
        ],
        vec![resource("test-service", "test_service", 600, 3_500, true)],
        vec![
            task(
                "fix-engine",
                TaskRole::Implement,
                &[],
                &[],
                &["engine-patch"],
                &["engine/state.txt"],
                520,
                25_000,
                false,
            ),
            task(
                "fix-adapter",
                TaskRole::Implement,
                &[],
                &[],
                &["adapter-patch"],
                &["adapter/state.txt"],
                500,
                24_000,
                false,
            ),
            task(
                "prepare-tests",
                TaskRole::Provision,
                &[],
                &["test-service"],
                &[],
                &[],
                100,
                4_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["fix-engine", "fix-adapter"],
                &["test-service"],
                &[],
                &[],
                300,
                10_000,
                false,
            ),
        ],
        &["engine-patch", "adapter-patch"],
        true,
        false,
        FaultInjection::ResourceProvisionFailure,
    )
}

fn resource_lease_expiry() -> Scenario {
    scenario(
        "resource-lease-expiry",
        "resource_lifecycle",
        "Refresh an expired test lease without repeating completed diagnostic work",
        vec![project("runner", false, true)],
        vec![resource("test-lease", "leased_test_host", 260, 700, true)],
        vec![
            task(
                "diagnose-runner",
                TaskRole::Inspect,
                &[],
                &[],
                &["diagnostic-report"],
                &[],
                620,
                15_000,
                false,
            ),
            task(
                "fix-runner",
                TaskRole::Implement,
                &["diagnose-runner"],
                &["test-lease"],
                &["runner-patch"],
                &["runner/state.txt"],
                360,
                20_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["fix-runner"],
                &["test-lease"],
                &[],
                &[],
                160,
                6_000,
                false,
            ),
        ],
        &["diagnostic-report", "runner-patch"],
        true,
        false,
        FaultInjection::ResourceExpiry,
    )
}

fn worker_recovery() -> Scenario {
    scenario(
        "targeted-worker-recovery",
        "recovery",
        "Recover one failed writer without restarting an independent completed writer",
        vec![
            project("library", false, false),
            project("cli", false, true),
        ],
        Vec::new(),
        vec![
            task(
                "fix-library",
                TaskRole::Implement,
                &[],
                &[],
                &["library-patch"],
                &["library/state.txt"],
                520,
                25_000,
                false,
            ),
            task(
                "fix-cli",
                TaskRole::Implement,
                &[],
                &[],
                &["cli-patch"],
                &["cli/state.txt"],
                500,
                24_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["fix-library", "fix-cli"],
                &[],
                &[],
                &[],
                280,
                9_000,
                false,
            ),
        ],
        &["library-patch", "cli-patch"],
        true,
        false,
        FaultInjection::WorkerCrashAfterArtifact,
    )
}

fn baseline_drift() -> Scenario {
    scenario(
        "baseline-drift-invalidation",
        "artifact_invalidation",
        "Reject generated output from an obsolete source baseline and redo only dependent work",
        vec![
            project("schema", false, false),
            project("consumer", false, false),
        ],
        Vec::new(),
        vec![
            task(
                "change-schema",
                TaskRole::Implement,
                &[],
                &[],
                &["schema-patch"],
                &["schema/state.txt"],
                440,
                22_000,
                false,
            ),
            task(
                "update-consumer",
                TaskRole::Implement,
                &["change-schema"],
                &[],
                &["consumer-patch"],
                &["consumer/state.txt"],
                520,
                25_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["update-consumer"],
                &[],
                &[],
                &[],
                260,
                8_000,
                false,
            ),
        ],
        &["schema-patch", "consumer-patch"],
        true,
        false,
        FaultInjection::SourceBaselineDrift,
    )
}

fn sequential_anti_case() -> Scenario {
    scenario(
        "sequential-small-fix",
        "anti_case",
        "Make a small two-step change whose second edit depends on the exact first result",
        vec![project("application", false, false)],
        Vec::new(),
        vec![
            task(
                "change-core",
                TaskRole::Implement,
                &[],
                &[],
                &["core-patch"],
                &["application/state.txt"],
                300,
                16_000,
                false,
            ),
            task(
                "adjust-caller",
                TaskRole::Implement,
                &["change-core"],
                &[],
                &["caller-patch"],
                &["application/state.txt"],
                280,
                15_000,
                false,
            ),
            task(
                "verify-integration",
                TaskRole::Verify,
                &["adjust-caller"],
                &[],
                &[],
                &[],
                180,
                6_000,
                false,
            ),
        ],
        &["core-patch", "caller-patch"],
        false,
        false,
        FaultInjection::None,
    )
}

#[allow(clippy::too_many_arguments)]
fn scenario(
    id: &str,
    family: &str,
    title: &str,
    projects: Vec<ProjectSpec>,
    resources: Vec<ResourceSpec>,
    tasks: Vec<TaskSpec>,
    final_artifacts: &[&str],
    expected_delegate: bool,
    requires_independent_review: bool,
    fault: FaultInjection,
) -> Scenario {
    Scenario {
        id: id.into(),
        family: family.into(),
        title: title.into(),
        prompt: format!(
            "{title}. Preserve user changes, prove the final result, and avoid unnecessary setup."
        ),
        projects,
        tasks,
        resources,
        final_artifacts: strings(final_artifacts),
        expected_delegate,
        requires_independent_review,
        fault,
    }
}

fn project(id: &str, cloud_eligible: bool, dirty_notes: bool) -> ProjectSpec {
    ProjectSpec {
        id: id.into(),
        initial_files: vec![
            FileFixture::new(
                "README.md",
                format!("# {id}\n\nSynthetic benchmark project.\n"),
            ),
            FileFixture::new("state.txt", format!("broken:{id}\n")),
        ],
        dirty_user_files: if dirty_notes {
            vec![FileFixture::new(
                "notes.user",
                format!("keep-user-note:{id}\n"),
            )]
        } else {
            Vec::new()
        },
        solution_files: vec![FileFixture::new("state.txt", format!("fixed:{id}\n"))],
        allowed_changed_paths: strings(&["state.txt"]),
        cloud_eligible,
    }
}

fn resource(id: &str, kind: &str, provision_ms: u64, ttl_ms: u64, reusable: bool) -> ResourceSpec {
    ResourceSpec {
        id: id.into(),
        kind: kind.into(),
        provision_ms,
        ttl_ms,
        reusable,
    }
}

#[allow(clippy::too_many_arguments)]
fn task(
    id: &str,
    role: TaskRole,
    dependencies: &[&str],
    resources: &[&str],
    outputs: &[&str],
    write_scope: &[&str],
    duration_ms: u64,
    token_estimate: u64,
    cloud_eligible: bool,
) -> TaskSpec {
    TaskSpec {
        id: id.into(),
        role,
        dependencies: dependencies.iter().map(|value| (*value).into()).collect(),
        resources: resources.iter().map(|value| (*value).into()).collect(),
        outputs: outputs.iter().map(|value| (*value).into()).collect(),
        write_scope: strings(write_scope),
        duration_ms,
        token_estimate,
        cloud_eligible,
    }
}

fn strings(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).into()).collect()
}
