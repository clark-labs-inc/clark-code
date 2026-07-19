use super::{resource, scenario, task};
use crate::model::{FaultInjection, FileFixture, ProjectSpec, Scenario, TaskRole};

pub(super) fn parallel_feature_recovery() -> Scenario {
    scenario(
        "large-parallel-feature-recovery",
        "large_parallel_writer",
        "Ship one cross-repository feature through eight isolated writers while shared environments prepare and one writer crashes",
        vec![
            project(
                "contract",
                &["src/api.contract", "src/events.contract"],
                false,
            ),
            project(
                "service",
                &["src/auth.logic", "src/storage.logic", "src/handler.logic"],
                false,
            ),
            project("clients", &["src/sdk.client", "src/cli.client"], true),
            project("delivery", &["src/rollout.plan"], true),
        ],
        vec![
            resource("generator", "code_generator", 900, 20_000, true),
            resource(
                "integration-environment",
                "ephemeral_integration_environment",
                1_200,
                20_000,
                true,
            ),
        ],
        vec![
            task(
                "inspect-boundaries",
                TaskRole::Inspect,
                &[],
                &[],
                &[],
                &[],
                350,
                8_000,
                false,
            ),
            task(
                "contract-api-writer",
                TaskRole::Implement,
                &["inspect-boundaries"],
                &[],
                &["api-contract-patch"],
                &["contract/src/api.contract"],
                1_500,
                16_000,
                false,
            ),
            task(
                "contract-events-writer",
                TaskRole::Implement,
                &["inspect-boundaries"],
                &[],
                &["events-contract-patch"],
                &["contract/src/events.contract"],
                1_450,
                16_000,
                false,
            ),
            task(
                "service-auth-writer",
                TaskRole::Implement,
                &["inspect-boundaries"],
                &[],
                &["auth-patch"],
                &["service/src/auth.logic"],
                1_600,
                18_000,
                false,
            ),
            task(
                "service-storage-writer",
                TaskRole::Implement,
                &["inspect-boundaries"],
                &[],
                &["storage-patch"],
                &["service/src/storage.logic"],
                1_550,
                18_000,
                false,
            ),
            task(
                "service-handler-writer",
                TaskRole::Implement,
                &[
                    "contract-api-writer",
                    "contract-events-writer",
                    "service-auth-writer",
                    "service-storage-writer",
                ],
                &[],
                &["handler-patch"],
                &["service/src/handler.logic"],
                1_800,
                22_000,
                false,
            ),
            task(
                "client-sdk-writer",
                TaskRole::Generate,
                &["contract-api-writer", "contract-events-writer"],
                &["generator"],
                &["sdk-patch"],
                &["clients/src/sdk.client"],
                1_600,
                18_000,
                true,
            ),
            task(
                "client-cli-writer",
                TaskRole::Implement,
                &["client-sdk-writer"],
                &[],
                &["cli-patch"],
                &["clients/src/cli.client"],
                1_300,
                16_000,
                true,
            ),
            task(
                "delivery-rollout-writer",
                TaskRole::Implement,
                &["service-handler-writer", "client-cli-writer"],
                &["integration-environment"],
                &["rollout-patch"],
                &["delivery/src/rollout.plan"],
                1_500,
                16_000,
                true,
            ),
            task(
                "verify-large-integration",
                TaskRole::Verify,
                &["delivery-rollout-writer"],
                &["integration-environment"],
                &[],
                &[],
                900,
                10_000,
                false,
            ),
        ],
        &[
            "api-contract-patch",
            "events-contract-patch",
            "auth-patch",
            "storage-patch",
            "handler-patch",
            "sdk-patch",
            "cli-patch",
            "rollout-patch",
        ],
        true,
        true,
        FaultInjection::WorkerCrashAfterArtifact,
    )
}

fn project(id: &str, paths: &[&str], cloud_eligible: bool) -> ProjectSpec {
    let mut initial_files = vec![FileFixture::new(
        "README.md",
        format!("# {id}\n\nLarge synthetic parallel-writer benchmark project.\n"),
    )];
    let mut solution_files = Vec::new();
    for path in paths {
        initial_files.push(FileFixture::new(path, format!("broken:{id}:{path}\n")));
        solution_files.push(FileFixture::new(path, format!("fixed:{id}:{path}\n")));
    }
    ProjectSpec {
        id: id.into(),
        initial_files,
        dirty_user_files: vec![FileFixture::new(
            "notes.user",
            format!("keep-large-user-note:{id}\n"),
        )],
        solution_files,
        allowed_changed_paths: paths.iter().map(|path| (*path).into()).collect(),
        cloud_eligible,
    }
}
