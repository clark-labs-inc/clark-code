use super::*;
use model::{CandidateKind, EvidenceLevel, HardFailure, LaneKind, TaskRole};

fn lane(kind: LaneKind) -> LaneSpec {
    LaneSpec::catalog("strong", "cheap", "reviewer")
        .into_iter()
        .find(|lane| lane.kind == kind)
        .unwrap()
}

fn scenario(id: &str) -> model::Scenario {
    fixtures::catalog()
        .into_iter()
        .find(|scenario| scenario.id == id)
        .unwrap()
}

fn grade_reference(
    scenario: &model::Scenario,
    lane: &LaneSpec,
) -> (tempfile::TempDir, model::RunRecord) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = SeededWorkspace::seed(temp.path(), scenario).unwrap();
    let result = simulator::run_reference(scenario, lane, &workspace).unwrap();
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Simulation,
        CandidateKind::Reference,
        scenario,
        0,
        lane,
        &workspace,
        result,
    )
    .unwrap();
    (temp, record)
}

#[test]
fn every_reference_work_graph_lane_passes_every_generic_scenario() {
    for scenario in fixtures::catalog() {
        for kind in [
            LaneKind::WorkGraphStrong,
            LaneKind::WorkGraphCheapSupport,
            LaneKind::WorkGraphDiverseReview,
            LaneKind::WorkGraphCloud,
        ] {
            let (_temp, record) = grade_reference(&scenario, &lane(kind));
            assert!(
                record.passed(),
                "{} / {:?}: {:?}",
                scenario.id,
                kind,
                record.hard_failures
            );
        }
    }
}

#[test]
fn naive_parallelism_is_a_negative_control_not_a_reference_oracle() {
    let scenario = scenario("reusable-build-cache");
    let (_temp, record) = grade_reference(&scenario, &lane(LaneKind::NaiveParallel));
    assert_eq!(record.behavioral_correctness, 1.0);
    assert!(record
        .hard_failures
        .contains(&HardFailure::DependencyOrderViolation));
    assert!(record
        .hard_failures
        .contains(&HardFailure::ResourceLifecycleViolation));
    assert!(record
        .hard_failures
        .contains(&HardFailure::DuplicateResourceSetup));
    assert!(record
        .hard_failures
        .contains(&HardFailure::RawProcessHandoff));
    assert!(record
        .hard_failures
        .contains(&HardFailure::BudgetOversubscribed));
}

#[test]
fn large_parallel_writer_case_is_wide_bounded_and_recovers_only_the_failed_writer() {
    let scenario = scenario("large-parallel-feature-recovery");
    assert_eq!(scenario.projects.len(), 4);
    assert_eq!(
        scenario
            .tasks
            .iter()
            .filter(|task| task.role.writes())
            .count(),
        8
    );
    let graph_lane = lane(LaneKind::WorkGraphStrong);
    let (_temp, record) = grade_reference(&scenario, &graph_lane);
    assert!(record.passed(), "{:?}", record.hard_failures);

    let writer_tasks = record
        .result
        .tasks
        .iter()
        .filter(|task| task.role.writes())
        .collect::<Vec<_>>();
    let peak_writers = writer_tasks
        .iter()
        .map(|task| task.started_ms)
        .map(|moment| {
            writer_tasks
                .iter()
                .filter(|task| task.started_ms <= moment && moment < task.finished_ms)
                .count()
        })
        .max()
        .unwrap();
    assert_eq!(peak_writers, graph_lane.max_parallel_tasks);

    let failed_id = "delivery-rollout-writer";
    assert_eq!(
        writer_tasks
            .iter()
            .filter(|task| task.id == failed_id)
            .count(),
        2
    );
    assert!(writer_tasks
        .iter()
        .filter(|task| task.id != failed_id)
        .all(|task| task.attempt == 1));
    assert_eq!(record.result.recoveries.len(), 1);
    assert_eq!(
        record.result.recoveries[0].restarted_subjects,
        vec![failed_id]
    );
    assert_eq!(record.result.recoveries[0].preserved_artifact_shas.len(), 7);

    let environment_ready = record
        .result
        .resources
        .iter()
        .find(|resource| resource.resource_id == "integration-environment")
        .unwrap()
        .ready_ms
        .unwrap();
    assert!(writer_tasks
        .iter()
        .any(|task| task.started_ms < environment_ready));
    assert_eq!(record.result.usage.model_polling_tokens, 0);

    let (_naive_temp, naive) = grade_reference(&scenario, &lane(LaneKind::NaiveParallel));
    assert!(naive
        .hard_failures
        .contains(&HardFailure::ParallelismLimitExceeded));
}

#[test]
fn large_parallel_writer_case_has_a_material_simulated_wall_time_advantage() {
    let mut scenario = scenario("large-parallel-feature-recovery");
    scenario.fault = model::FaultInjection::None;
    let (_single_temp, single) = grade_reference(&scenario, &lane(LaneKind::EqualBudgetSingle));
    let (_graph_temp, graph) = grade_reference(&scenario, &lane(LaneKind::WorkGraphStrong));
    assert!(single.passed(), "{:?}", single.hard_failures);
    assert!(graph.passed(), "{:?}", graph.hard_failures);
    assert!(
        graph.result.usage.wall_ms * 100 <= single.result.usage.wall_ms * 70,
        "graph={}ms single={}ms",
        graph.result.usage.wall_ms,
        single.result.usage.wall_ms
    );
    assert!(graph.total_tokens() > single.total_tokens());
}

#[test]
fn baseline_drift_is_rejected_and_only_dependent_work_is_recovered() {
    let scenario = scenario("baseline-drift-invalidation");
    let (_temp, record) = grade_reference(&scenario, &lane(LaneKind::WorkGraphStrong));
    assert!(record.passed(), "{:?}", record.hard_failures);
    let stale = record
        .result
        .artifacts
        .iter()
        .find(|artifact| artifact.stale)
        .unwrap();
    assert!(stale.rejected);
    assert!(stale.consumed_by.is_empty());
    assert_eq!(record.result.recoveries[0].restarted_subjects.len(), 1);
}

#[test]
fn resource_failure_preserves_completed_code_artifacts() {
    let scenario = scenario("targeted-resource-recovery");
    let (_temp, record) = grade_reference(&scenario, &lane(LaneKind::WorkGraphStrong));
    assert!(record.passed(), "{:?}", record.hard_failures);
    let recovery = record
        .result
        .recoveries
        .iter()
        .find(|recovery| recovery.reason.contains("resource failed"))
        .unwrap();
    assert_eq!(recovery.restarted_subjects, vec!["test-service"]);
    assert_eq!(recovery.preserved_artifact_shas.len(), 2);
}

#[test]
fn expired_resource_is_replaced_without_repeating_diagnosis() {
    let scenario = scenario("resource-lease-expiry");
    let (_temp, record) = grade_reference(&scenario, &lane(LaneKind::WorkGraphStrong));
    assert!(record.passed(), "{:?}", record.hard_failures);
    let recovery = record
        .result
        .recoveries
        .iter()
        .find(|recovery| recovery.reason.contains("lease expired"))
        .unwrap();
    assert_eq!(recovery.restarted_subjects, vec!["test-lease"]);
    assert_eq!(recovery.preserved_artifact_shas.len(), 1);
}

#[test]
fn work_graph_declines_the_strictly_sequential_anti_case() {
    let scenario = scenario("sequential-small-fix");
    let (_temp, record) = grade_reference(&scenario, &lane(LaneKind::WorkGraphStrong));
    assert!(record.passed(), "{:?}", record.hard_failures);
    assert!(!record.result.delegated);
}

#[test]
fn cheap_lane_routes_support_work_cheaply_but_keeps_writers_strong() {
    let scenario = scenario("toolchain-bootstrap-fix");
    let (_temp, record) = grade_reference(&scenario, &lane(LaneKind::WorkGraphCheapSupport));
    assert!(record.passed(), "{:?}", record.hard_failures);
    assert!(record
        .result
        .tasks
        .iter()
        .filter(|task| matches!(task.role, TaskRole::Inspect | TaskRole::Provision))
        .all(|task| task.model == "cheap"));
    assert!(record
        .result
        .tasks
        .iter()
        .filter(|task| task.role.writes())
        .all(|task| task.model == "strong"));
}

#[test]
fn cloud_lane_routes_only_cloud_eligible_work_to_the_cloud_harness() {
    let scenario = scenario("remote-compute-integration");
    let (_temp, record) = grade_reference(&scenario, &lane(LaneKind::WorkGraphCloud));
    assert!(record.passed(), "{:?}", record.hard_failures);
    let kernel = record
        .result
        .tasks
        .iter()
        .find(|task| task.id == "validate-kernel")
        .unwrap();
    let client = record
        .result
        .tasks
        .iter()
        .find(|task| task.id == "update-client")
        .unwrap();
    assert_eq!(kernel.harness, "clark-cloud");
    assert_eq!(client.harness, "local");
}

#[test]
fn current_clark_baseline_stays_red_even_when_hidden_code_checks_pass() {
    let scenario = scenario("toolchain-bootstrap-fix");
    let lane = lane(LaneKind::WorkGraphStrong);
    let temp = tempfile::tempdir().unwrap();
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let result = simulator::run_current(&scenario, &lane, &workspace).unwrap();
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Simulation,
        CandidateKind::ClarkCurrent,
        &scenario,
        0,
        &lane,
        &workspace,
        result,
    )
    .unwrap();
    assert_eq!(record.behavioral_correctness, 1.0);
    assert!(record
        .hard_failures
        .contains(&HardFailure::AuthoritativePlanMissing));
    assert!(record
        .hard_failures
        .contains(&HardFailure::ProductionTraceMissing));
    assert!(record
        .hard_failures
        .contains(&HardFailure::UnverifiedCompletion));
    assert!(record
        .hard_failures
        .contains(&HardFailure::NonTechnicalDefaultFlowMissing));
}

#[test]
fn public_manifest_contains_no_oracle_graph_fault_or_solution() {
    let scenario = scenario("targeted-resource-recovery");
    let lane = lane(LaneKind::WorkGraphStrong);
    let temp = tempfile::tempdir().unwrap();
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let json = serde_json::to_string(&workspace.public_manifest(&scenario, &lane)).unwrap();
    assert!(!json.contains("fixed:"));
    assert!(!json.contains("fix-engine"));
    assert!(!json.contains("resource_provision_failure"));
    assert!(!json.contains("expected_delegate"));
}

#[test]
fn tampered_artifact_integrity_and_model_polling_are_hard_failures() {
    let scenario = scenario("generated-contract-pipeline");
    let lane = lane(LaneKind::WorkGraphStrong);
    let temp = tempfile::tempdir().unwrap();
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let mut result = simulator::run_reference(&scenario, &lane, &workspace).unwrap();
    result.artifacts[0].integrity_sha256 = "forged".into();
    result.usage.model_polling_tokens = 500;
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Simulation,
        CandidateKind::Reference,
        &scenario,
        0,
        &lane,
        &workspace,
        result,
    )
    .unwrap();
    assert!(record
        .hard_failures
        .contains(&HardFailure::ArtifactLineageInvalid));
    assert!(record
        .hard_failures
        .contains(&HardFailure::ModelPollingDuringWait));
}
