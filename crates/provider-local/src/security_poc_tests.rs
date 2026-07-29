use super::*;

fn candidate(outcome: SecurityPocOutcome) -> SecurityCandidate {
    SecurityCandidate {
        candidate_id: "candidate-1".into(),
        rule_id: "input-validation.security-boundary".into(),
        identity_anchor: "test-input-reaches-sensitive-sink".into(),
        identity_instance: None,
        title: "Unvalidated input crosses a security boundary".into(),
        summary: "The fixture input reaches a security-sensitive operation.".into(),
        category: "input-validation".into(),
        cwe: vec!["CWE-20".into()],
        severity: SecuritySeverity::High,
        confidence: SecurityConfidence::High,
        source: SecurityLocation {
            path: "src/input.rs".into(),
            line: Some(1),
            description: "attacker input".into(),
        },
        control: SecurityLocation {
            path: "src/control.rs".into(),
            line: Some(2),
            description: "missing control".into(),
        },
        sink: SecurityLocation {
            path: "src/sink.rs".into(),
            line: Some(3),
            description: "sensitive sink".into(),
        },
        impact: "crosses a security boundary".into(),
        remediation: "Validate and constrain the input before the sensitive operation.".into(),
        validation: SecurityValidation {
            disposition: SecurityDisposition::Reportable,
            evidence: "reproduction reaches the sink".into(),
            counterevidence: Vec::new(),
        },
        poc: SecurityPocEvidence {
            goal: "prove the vulnerable path and the safe control".into(),
            outcome,
            positive_receipt_id: Some("positive".into()),
            negative_receipt_id: Some("negative".into()),
            limitations: Vec::new(),
        },
        attack_path: Some(SecurityAttackPath {
            attacker: "remote user".into(),
            entrypoint: "POST /input".into(),
            preconditions: Vec::new(),
            path: vec!["input".into(), "sink".into()],
            likelihood: "high".into(),
        }),
    }
}

fn receipt(
    id: &str,
    control: SecurityPocControl,
    script: &str,
    passed: bool,
) -> SecurityPocReceipt {
    SecurityPocReceipt {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        receipt_id: id.into(),
        scan_id: "scan-1".into(),
        candidate_id: "candidate-1".into(),
        inventory_id: "inventory-1".into(),
        control,
        language: "python".into(),
        script_sha256: script.into(),
        expected_observation_sha256: "observation".into(),
        workspace_sha256: "workspace".into(),
        stdout_sha256: "stdout".into(),
        stderr_sha256: "stderr".into(),
        expected_exit_code: 0,
        exit_code: Some(if passed { 0 } else { 1 }),
        passed,
        containment: "managed_disposable".into(),
        artifact_path: format!(".clark/security-scans/scan-1/poc/{id}/receipt.json"),
        execution: None,
    }
}

fn ledger() -> SecurityPocLedger {
    let mut ledger = SecurityPocLedger::default();
    ledger
        .record(receipt(
            "positive",
            SecurityPocControl::Positive,
            "positive-script",
            true,
        ))
        .unwrap();
    ledger
        .record(receipt(
            "negative",
            SecurityPocControl::Negative,
            "negative-script",
            true,
        ))
        .unwrap();
    ledger
}

#[test]
fn reproduced_candidate_requires_host_issued_control_pair() {
    let candidate = candidate(SecurityPocOutcome::Reproduced);
    let error = SecurityPocLedger::default()
        .validate_candidate("scan-1", "inventory-1", &candidate)
        .unwrap_err();
    assert!(error.contains("unknown host-issued PoC receipt"));

    ledger()
        .validate_candidate("scan-1", "inventory-1", &candidate)
        .unwrap();
}

#[test]
fn controls_must_be_distinct_and_pass() {
    let mut duplicate = ledger();
    duplicate
        .record(receipt(
            "negative-same-script",
            SecurityPocControl::Negative,
            "positive-script",
            true,
        ))
        .unwrap();
    let mut candidate = candidate(SecurityPocOutcome::Reproduced);
    candidate.poc.negative_receipt_id = Some("negative-same-script".into());
    assert!(duplicate
        .validate_candidate("scan-1", "inventory-1", &candidate)
        .unwrap_err()
        .contains("must be distinct"));

    let mut failed = ledger();
    failed
        .record(receipt(
            "negative-failed",
            SecurityPocControl::Negative,
            "failed-script",
            false,
        ))
        .unwrap();
    candidate.poc.negative_receipt_id = Some("negative-failed".into());
    assert!(failed
        .validate_candidate("scan-1", "inventory-1", &candidate)
        .unwrap_err()
        .contains("requires passing"));
}

#[test]
fn blocked_or_unsafe_attempts_cannot_be_reportable() {
    for outcome in [
        SecurityPocOutcome::Blocked,
        SecurityPocOutcome::UnsafeToExecute,
    ] {
        let mut blocked = candidate(outcome);
        blocked.poc.positive_receipt_id = None;
        blocked.poc.negative_receipt_id = None;
        blocked.poc.limitations = vec!["requires a service unavailable in the offline lab".into()];
        assert!(SecurityPocLedger::default()
            .validate_candidate("scan-1", "inventory-1", &blocked)
            .unwrap_err()
            .contains("disposition and PoC outcome are inconsistent"));

        blocked.validation.disposition = SecurityDisposition::Deferred;
        SecurityPocLedger::default()
            .validate_candidate("scan-1", "inventory-1", &blocked)
            .unwrap();
    }
}

#[test]
fn receipt_cannot_be_reused_across_snapshots_or_candidates() {
    let candidate = candidate(SecurityPocOutcome::Reproduced);
    assert!(ledger()
        .validate_candidate("scan-1", "inventory-2", &candidate)
        .unwrap_err()
        .contains("does not match scan, inventory, candidate, and control"));
}
