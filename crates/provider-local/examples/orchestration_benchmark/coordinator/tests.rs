use super::*;

fn lane(kind: LaneKind) -> LaneSpec {
    LaneSpec::catalog("scripted-strong", "scripted-cheap")
        .into_iter()
        .find(|lane| lane.kind == kind)
        .unwrap()
}

#[tokio::test]
async fn single_and_multi_lanes_share_record_schema_and_pass_clean_fixture() {
    let scenario = scenarios::find("independent-modules-1").unwrap();
    let artifacts = tempfile::tempdir().unwrap();
    let options = ScriptedRunOptions {
        artifact_root: artifacts.path().to_path_buf(),
        repetition: 1,
    };
    let single = run_scripted(&scenario, &lane(LaneKind::Single), &options)
        .await
        .unwrap();
    let multi = run_scripted(&scenario, &lane(LaneKind::CheapSubagents), &options)
        .await
        .unwrap();
    assert!(single.passed(), "{single:#?}");
    assert!(multi.passed(), "{multi:#?}");
    assert!(!single.trigger.actual_delegate);
    assert!(multi.trigger.actual_delegate);
    assert!(multi.attempts.len() > single.attempts.len());
}

#[tokio::test]
async fn injected_crash_is_retried_without_losing_correctness() {
    let scenario = scenarios::find("worker-crash-1").unwrap();
    let artifacts = tempfile::tempdir().unwrap();
    let record = run_scripted(
        &scenario,
        &lane(LaneKind::ReaderWriter),
        &ScriptedRunOptions {
            artifact_root: artifacts.path().to_path_buf(),
            repetition: 1,
        },
    )
    .await
    .unwrap();
    assert!(record.passed(), "{record:#?}");
    assert_eq!(record.metrics.recovered_failures, 1);
    assert!(record
        .attempts
        .iter()
        .any(|attempt| attempt.status == AgentStatus::Errored));
}

#[tokio::test]
async fn permission_escalation_is_refused_then_retried_narrowly() {
    let scenario = scenarios::find("permission-escalation-1").unwrap();
    let artifacts = tempfile::tempdir().unwrap();
    let record = run_scripted(
        &scenario,
        &lane(LaneKind::ReaderWriter),
        &ScriptedRunOptions {
            artifact_root: artifacts.path().to_path_buf(),
            repetition: 1,
        },
    )
    .await
    .unwrap();
    assert!(record.passed(), "{record:#?}");
    assert!(record.hard_failures.is_empty());
    assert_eq!(record.metrics.recovered_failures, 1);
}

#[tokio::test]
async fn reviewer_rejects_a_wrong_report_then_accepts_rework() {
    let scenario = scenarios::find("reviewer-bug-1").unwrap();
    let artifacts = tempfile::tempdir().unwrap();
    let options = ScriptedRunOptions {
        artifact_root: artifacts.path().to_path_buf(),
        repetition: 1,
    };
    let single = run_scripted(&scenario, &lane(LaneKind::Single), &options)
        .await
        .unwrap();
    let reviewed = run_scripted(&scenario, &lane(LaneKind::Reviewed), &options)
        .await
        .unwrap();
    assert!(!single.passed());
    assert!(reviewed.passed(), "{reviewed:#?}");
    assert_eq!(reviewed.metrics.review_catches, 1);
    assert_eq!(
        reviewed
            .handoffs
            .iter()
            .filter(|handoff| handoff.task_id == "implement")
            .count(),
        2
    );
}

#[tokio::test]
async fn cloud_lane_only_assigns_cloud_provider_to_eligible_reader() {
    let scenario = scenarios::find("clark-cloud-1").unwrap();
    let artifacts = tempfile::tempdir().unwrap();
    let record = run_scripted(
        &scenario,
        &lane(LaneKind::ClarkCloud),
        &ScriptedRunOptions {
            artifact_root: artifacts.path().to_path_buf(),
            repetition: 1,
        },
    )
    .await
    .unwrap();
    assert!(record.passed(), "{record:#?}");
    assert!(record
        .attempts
        .iter()
        .any(|attempt| attempt.provider == "scripted-clark-cloud"));
    assert_eq!(record.trigger.cloud_agent_assignment_score, 1.0);
}
