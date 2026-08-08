use super::tests::{bundle, inventory, poc_ledger};
use super::*;

fn checkpointed_deep() -> (SecurityDeepLedger, String) {
    let mut ledger = SecurityDeepLedger::default();
    let run_id = ledger
        .begin("scan-1", &inventory().inventory_id)
        .unwrap()
        .run_id;
    for (index, focus) in [
        "entrypoint and trust boundary census",
        "authorization and tenant isolation challenge",
        "parser and sink contradiction pass",
    ]
    .into_iter()
    .enumerate()
    {
        let orchestration_id = format!("fanout-{index}");
        ledger.record_orchestration(
            &orchestration_id,
            focus,
            vec![SecurityDeepTaskReceipt {
                task_id: format!("task-{index}"),
                attempt: 1,
                claim_count: 1,
            }],
        );
        ledger
            .checkpoint(&run_id, &orchestration_id, vec!["candidate-1".into()])
            .unwrap();
    }
    (ledger, run_id)
}

fn deep_bundle(run_id: String) -> SecurityScanBundle {
    let mut bundle = bundle();
    bundle.mode = SecurityScanMode::Deep;
    bundle.deep_run_id = Some(run_id);
    bundle
}

#[test]
fn deep_scan_requires_accepted_independent_passes_and_saturation() {
    let (ledger, run_id) = checkpointed_deep();
    let status = ledger.status().unwrap();
    assert!(status.saturated);
    assert_eq!(status.passes.len(), 3);
    assert_eq!(
        status.passes[0].novel_candidate_ids.as_deref(),
        Some(["candidate-1".to_string()].as_slice())
    );
    assert!(status.passes[1]
        .novel_candidate_ids
        .as_ref()
        .unwrap()
        .is_empty());
    assert!(status.passes[2]
        .novel_candidate_ids
        .as_ref()
        .unwrap()
        .is_empty());

    let seal = finalize_security_deep(
        &deep_bundle(run_id.clone()),
        &inventory(),
        &ledger,
        &poc_ledger(),
    )
    .unwrap();
    assert_eq!(seal.deep_run_id.as_deref(), Some(run_id.as_str()));
    assert_eq!(seal.deep_passes, Some(3));
}

#[test]
fn deep_scan_rejects_unsealed_or_mismatched_candidate_reduction() {
    let mut ledger = SecurityDeepLedger::default();
    let run_id = ledger
        .begin("scan-1", &inventory().inventory_id)
        .unwrap()
        .run_id;
    ledger.record_orchestration(
        "fanout-only",
        "single pass",
        vec![SecurityDeepTaskReceipt {
            task_id: "task-only".into(),
            attempt: 1,
            claim_count: 0,
        }],
    );
    ledger
        .checkpoint(&run_id, "fanout-only", vec!["candidate-1".into()])
        .unwrap();
    assert!(
        finalize_security_deep(&deep_bundle(run_id), &inventory(), &ledger, &poc_ledger(),)
            .unwrap_err()
            .contains("at least 3")
    );

    let (ledger, run_id) = checkpointed_deep();
    let mut missing = deep_bundle(run_id);
    missing.candidates.clear();
    assert!(
        finalize_security_deep(&missing, &inventory(), &ledger, &poc_ledger())
            .unwrap_err()
            .contains("candidate reduction does not match")
    );
}
