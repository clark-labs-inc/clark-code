use super::*;

pub(super) fn poc_ledger(inventory_id: &str) -> SecurityPocLedger {
    let mut ledger = SecurityPocLedger::default();
    for (receipt_id, control, script_sha256) in [
        (
            "poc-positive",
            SecurityPocControl::Positive,
            "positive-script",
        ),
        (
            "poc-negative",
            SecurityPocControl::Negative,
            "negative-script",
        ),
    ] {
        ledger
            .record(SecurityPocReceipt {
                contract_version: SECURITY_SCAN_CONTRACT_VERSION,
                receipt_id: receipt_id.into(),
                scan_id: "scan-1".into(),
                candidate_id: "candidate-1".into(),
                inventory_id: inventory_id.into(),
                control,
                language: "python".into(),
                script_sha256: script_sha256.into(),
                expected_observation_sha256: "observation".into(),
                workspace_sha256: "workspace".into(),
                stdout_sha256: "stdout".into(),
                stderr_sha256: "stderr".into(),
                expected_exit_code: 0,
                exit_code: Some(0),
                passed: true,
                containment: "managed_disposable".into(),
                artifact_path: format!(".agent/security-scans/scan-1/poc/{receipt_id}.json"),
                execution: None,
            })
            .unwrap();
    }
    ledger
}

pub(super) fn reproduced_poc() -> SecurityPocEvidence {
    SecurityPocEvidence {
        goal: "Demonstrate the vulnerable flow and its safe negative control".into(),
        outcome: SecurityPocOutcome::Reproduced,
        positive_receipt_id: Some("poc-positive".into()),
        negative_receipt_id: Some("poc-negative".into()),
        limitations: Vec::new(),
    }
}
