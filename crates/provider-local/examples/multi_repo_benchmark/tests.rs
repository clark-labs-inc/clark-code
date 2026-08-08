use super::*;
use model::{HardFailure, LaneKind};

fn lane(kind: LaneKind) -> LaneSpec {
    LaneSpec::catalog("strong", "cheap", "reviewer")
        .into_iter()
        .find(|lane| lane.kind == kind)
        .unwrap()
}

#[test]
fn production_adapter_passes_backend_gates_but_stays_red_without_real_ui_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = fixtures::catalog()
        .into_iter()
        .find(|scenario| scenario.id == "api-sdk-web")
        .unwrap();
    let lane = lane(LaneKind::MultiCheap);
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let result = run_current(&scenario, &lane, &workspace).unwrap();
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Scripted,
        CandidateKind::CurrentAgent,
        &scenario,
        0,
        &lane,
        &workspace,
        temp.path(),
        result,
    )
    .unwrap();
    assert_eq!(record.behavioral_correctness, 1.0);
    assert_eq!(record.replay_correctness, 1.0);
    assert!(!record
        .hard_failures
        .contains(&HardFailure::WriterIsolationMissing));
    assert!(!record
        .hard_failures
        .contains(&HardFailure::FreshIntegrationFailed));
    assert!(record
        .hard_failures
        .contains(&HardFailure::NonTechnicalDefaultFlowMissing));
}

#[test]
fn reference_adapter_passes_replay_and_conformance() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = fixtures::catalog()
        .into_iter()
        .find(|scenario| scenario.id == "api-sdk-web")
        .unwrap();
    let lane = lane(LaneKind::MultiDiverseReview);
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let result = run_reference(&scenario, &lane, &workspace, temp.path()).unwrap();
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Scripted,
        CandidateKind::Reference,
        &scenario,
        0,
        &lane,
        &workspace,
        temp.path(),
        result,
    )
    .unwrap();
    assert!(record.passed(), "{:?}", record.hard_failures);
}

#[test]
fn reference_retries_only_failed_work_and_preserves_dirty_file() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = fixtures::catalog()
        .into_iter()
        .find(|scenario| scenario.id == "targeted-child-recovery")
        .unwrap();
    let lane = lane(LaneKind::MultiCheap);
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let result = run_reference(&scenario, &lane, &workspace, temp.path()).unwrap();
    assert_eq!(result.recoveries.len(), 1);
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Scripted,
        CandidateKind::Reference,
        &scenario,
        0,
        &lane,
        &workspace,
        temp.path(),
        result,
    )
    .unwrap();
    assert!(record.passed(), "{:?}", record.hard_failures);
}

#[test]
fn public_manifest_contains_no_hidden_solution() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = fixtures::catalog().remove(0);
    let lane = lane(LaneKind::MultiCheap);
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let json = serde_json::to_string(&workspace.public_manifest(&scenario, &lane)).unwrap();
    assert!(!json.contains("req-42"));
    assert!(!json.contains("hidden_checks"));
    assert!(!json.contains("solution_files"));
    assert!(!json.contains("expected_delegate"));
}

#[test]
fn reference_does_not_delegate_the_sequential_anti_case() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = fixtures::catalog()
        .into_iter()
        .find(|scenario| scenario.id == "sequential-dependency-chain")
        .unwrap();
    let lane = lane(LaneKind::MultiStrong);
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let result = run_reference(&scenario, &lane, &workspace, temp.path()).unwrap();
    assert!(!result.delegated);
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Scripted,
        CandidateKind::Reference,
        &scenario,
        0,
        &lane,
        &workspace,
        temp.path(),
        result,
    )
    .unwrap();
    assert!(record.passed(), "{:?}", record.hard_failures);
}

#[test]
fn tampered_change_package_is_rejected_before_integration() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = fixtures::catalog()
        .into_iter()
        .find(|scenario| scenario.id == "api-sdk-web")
        .unwrap();
    let lane = lane(LaneKind::MultiCheap);
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let mut result = run_reference(&scenario, &lane, &workspace, temp.path()).unwrap();
    result.change_packages[0].patch_sha256 = "forged-digest".into();
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Scripted,
        CandidateKind::Reference,
        &scenario,
        0,
        &lane,
        &workspace,
        temp.path(),
        result,
    )
    .unwrap();
    assert!(record
        .hard_failures
        .contains(&HardFailure::InvalidChangePackage));
    assert_eq!(record.replay_correctness, 0.0);
}

#[test]
fn expert_only_or_jargon_heavy_flow_fails_the_default_experience_gate() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = fixtures::catalog().remove(0);
    let lane = lane(LaneKind::MultiCheap);
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let mut result = run_reference(&scenario, &lane, &workspace, temp.path()).unwrap();
    let interaction = result.interaction.as_mut().unwrap();
    interaction.model_choice_required = true;
    interaction
        .exposed_internal_terms
        .push("choose a worktree and agent model".into());
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Scripted,
        CandidateKind::Reference,
        &scenario,
        0,
        &lane,
        &workspace,
        temp.path(),
        result,
    )
    .unwrap();
    assert!(record
        .hard_failures
        .contains(&HardFailure::NonTechnicalDefaultFlowMissing));
}

#[test]
fn forged_or_missing_host_plan_receipt_fails_conformance() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = fixtures::catalog().remove(0);
    let lane = lane(LaneKind::MultiCheap);
    let workspace = SeededWorkspace::seed(temp.path(), &scenario).unwrap();
    let mut result = run_reference(&scenario, &lane, &workspace, temp.path()).unwrap();
    result.planning.as_mut().unwrap().plan_sha256 = "self-reported".into();
    let record = grader::grade(
        "test".into(),
        EvidenceLevel::Scripted,
        CandidateKind::Reference,
        &scenario,
        0,
        &lane,
        &workspace,
        temp.path(),
        result,
    )
    .unwrap();
    assert!(record
        .hard_failures
        .contains(&HardFailure::AuthoritativePlanningReceiptMissing));
}
