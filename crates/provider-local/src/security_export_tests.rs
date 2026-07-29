use super::*;
use crate::security::{
    finalize_security_scan, SecurityAttackPath, SecurityCandidate, SecurityConfidence,
    SecurityCoverage, SecurityCoverageStatus, SecurityDisposition, SecurityInventory,
    SecurityLocation, SecurityPocControl, SecurityPocEvidence, SecurityPocLedger,
    SecurityPocOutcome, SecurityPocReceipt, SecurityScanBundle, SecurityScanMode,
    SecurityScanPhase, SecuritySeverity, SecurityThreatModel, SecurityValidation,
    SECURITY_SCAN_CONTRACT_VERSION,
};

fn record() -> SecurityScanRecord {
    let inventory = SecurityInventory {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scope: ".".into(),
        inventory_id: "a".repeat(64),
        paths: vec!["src/auth.rs".into(), "src/routes.rs".into()],
    };
    let bundle = SecurityScanBundle {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scan_id: "export-test".into(),
        mode: SecurityScanMode::Standard,
        model: crate::SECURITY_MODEL.into(),
        scope: ".".into(),
        inventory_id: inventory.inventory_id.clone(),
        phase: SecurityScanPhase::Reporting,
        threat_model: SecurityThreatModel {
            assets: vec!["Tenant secrets".into()],
            trust_boundaries: vec!["Public API to service network".into()],
            attacker_inputs: vec!["Request destination".into()],
            invariants: vec!["Tenant input cannot select internal hosts".into()],
        },
        coverage: inventory
            .paths
            .iter()
            .map(|path| SecurityCoverage {
                path: path.clone(),
                status: SecurityCoverageStatus::Reviewed,
                reason: None,
            })
            .collect(),
        supporting_coverage: Vec::new(),
        diff_target: None,
        deep_run_id: None,
        candidates: vec![
            candidate(
                "reportable",
                "server-request-without-policy",
                SecurityDisposition::Reportable,
                SecurityPocOutcome::Reproduced,
                Some(SecurityAttackPath {
                    attacker: "Tenant user".into(),
                    entrypoint: "POST /fetch".into(),
                    preconditions: vec!["Tenant account".into()],
                    path: vec!["request destination".into(), "HTTP client".into()],
                    likelihood: "high".into(),
                }),
            ),
            candidate(
                "rejected",
                "server-request-with-allowlist",
                SecurityDisposition::Suppressed,
                SecurityPocOutcome::NotReproduced,
                None,
            ),
            candidate(
                "deferred",
                "server-request-runtime-unknown",
                SecurityDisposition::Deferred,
                SecurityPocOutcome::Blocked,
                None,
            ),
        ],
    };
    let mut ledger = SecurityPocLedger::default();
    for candidate in &bundle.candidates[..2] {
        for control in [SecurityPocControl::Positive, SecurityPocControl::Negative] {
            ledger
                .record(receipt(&bundle, candidate, control))
                .expect("receipt");
        }
    }
    let seal = finalize_security_scan(&bundle, &inventory, &ledger).expect("local seal");
    SecurityScanRecord {
        path: ".clark/security-scans/export-test/scan.json".into(),
        modified_at_ms: Some(10),
        bundle,
        seal: Some(seal),
        poc_receipts: Vec::new(),
    }
}

fn candidate(
    id: &str,
    anchor: &str,
    disposition: SecurityDisposition,
    outcome: SecurityPocOutcome,
    attack_path: Option<SecurityAttackPath>,
) -> SecurityCandidate {
    let has_controls = !matches!(
        outcome,
        SecurityPocOutcome::Blocked | SecurityPocOutcome::UnsafeToExecute
    );
    SecurityCandidate {
        candidate_id: id.into(),
        rule_id: "server-side-request-forgery.http-client".into(),
        identity_anchor: anchor.into(),
        identity_instance: None,
        title: format!("Candidate {id}"),
        summary: "A destination may cross the service network boundary.".into(),
        category: "server-side-request-forgery".into(),
        cwe: vec!["CWE-918".into()],
        severity: SecuritySeverity::High,
        confidence: SecurityConfidence::High,
        source: SecurityLocation {
            path: "src/routes.rs".into(),
            line: Some(10),
            description: "Tenant-controlled destination".into(),
        },
        control: SecurityLocation {
            path: "src/auth.rs".into(),
            line: Some(20),
            description: "Destination policy".into(),
        },
        sink: SecurityLocation {
            path: "src/routes.rs".into(),
            line: Some(30),
            description: "Server-side HTTP request".into(),
        },
        impact: "Tenant may reach internal services".into(),
        remediation: "Validate canonical destinations against an allowlist.".into(),
        validation: SecurityValidation {
            disposition,
            evidence: "The source-to-control-to-sink path was tested.".into(),
            counterevidence: vec!["Authentication is required.".into()],
        },
        poc: SecurityPocEvidence {
            goal: "Test the candidate and a safe control.".into(),
            outcome,
            positive_receipt_id: has_controls.then(|| format!("{id}-positive")),
            negative_receipt_id: has_controls.then(|| format!("{id}-negative")),
            limitations: if has_controls {
                Vec::new()
            } else {
                vec!["The required runtime is unavailable in the offline lab.".into()]
            },
        },
        attack_path,
    }
}

fn receipt(
    bundle: &SecurityScanBundle,
    candidate: &SecurityCandidate,
    control: SecurityPocControl,
) -> SecurityPocReceipt {
    let label = match control {
        SecurityPocControl::Positive => "positive",
        SecurityPocControl::Negative => "negative",
    };
    SecurityPocReceipt {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        receipt_id: format!("{}-{label}", candidate.candidate_id),
        scan_id: bundle.scan_id.clone(),
        candidate_id: candidate.candidate_id.clone(),
        inventory_id: bundle.inventory_id.clone(),
        control,
        language: "python".into(),
        script_sha256: sha256_hex(format!("{}-{label}", candidate.candidate_id).as_bytes()),
        expected_observation_sha256: sha256_hex(b"observation"),
        workspace_sha256: sha256_hex(b"workspace"),
        stdout_sha256: sha256_hex(b"stdout"),
        stderr_sha256: sha256_hex(b"stderr"),
        expected_exit_code: 0,
        exit_code: Some(0),
        passed: true,
        containment: "managed_disposable".into(),
        artifact_path: format!(
            ".clark/security-scans/export-test/poc/{}/{label}/receipt.json",
            candidate.candidate_id
        ),
        execution: None,
    }
}

#[test]
fn export_is_clark_owned_and_preserves_every_candidate_disposition() {
    let repository_id = Uuid::new_v4();
    let scan_id = Uuid::new_v4();
    let export =
        build_clark_security_cloud_export(&record(), repository_id, scan_id).expect("export");
    assert_eq!(
        export.manifest["documentType"],
        "clark-security.scan-manifest"
    );
    assert_eq!(export.findings["documentType"], "clark-security.findings");
    assert_eq!(export.coverage["documentType"], "clark-security.coverage");
    assert_eq!(export.findings["findings"].as_array().unwrap().len(), 1);
    assert_eq!(export.occurrences.len(), 3);
    assert_eq!(
        export
            .occurrences
            .iter()
            .map(|occurrence| occurrence.disposition.as_str())
            .collect::<Vec<_>>(),
        ["reported", "rejected", "deferred"]
    );
    assert_eq!(export.coverage_completeness, "partial");
    assert_eq!(
        export.findings["findings"][0]["rootCause"],
        export.occurrences[0].root_cause
    );
    assert_eq!(
        export.findings["findings"][0]["attackPath"],
        export.occurrences[0].attack_path
    );
    assert_eq!(
        export.findings["findings"][0]["provenance"],
        export.occurrences[0].provenance
    );
    assert_eq!(
        export.findings["findings"][0]["provenance"]["source"],
        "clark-security"
    );
}

#[test]
fn clark_finding_identity_survives_line_motion_but_occurrence_does_not() {
    let repository_id = Uuid::new_v4();
    let first =
        build_clark_security_cloud_export(&record(), repository_id, Uuid::from_u128(1)).unwrap();
    let mut moved = record();
    moved.bundle.candidates[0].source.line = Some(900);
    let inventory = SecurityInventory {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scope: ".".into(),
        inventory_id: moved.bundle.inventory_id.clone(),
        paths: moved
            .bundle
            .coverage
            .iter()
            .map(|coverage| coverage.path.clone())
            .collect(),
    };
    let mut ledger = SecurityPocLedger::default();
    for candidate in &moved.bundle.candidates[..2] {
        for control in [SecurityPocControl::Positive, SecurityPocControl::Negative] {
            ledger
                .record(receipt(&moved.bundle, candidate, control))
                .unwrap();
        }
    }
    moved.seal = Some(finalize_security_scan(&moved.bundle, &inventory, &ledger).unwrap());
    let second =
        build_clark_security_cloud_export(&moved, repository_id, Uuid::from_u128(2)).unwrap();
    assert_eq!(
        first.findings["findings"][0]["findingId"],
        second.findings["findings"][0]["findingId"]
    );
    assert_ne!(
        first.findings["findings"][0]["occurrenceId"],
        second.findings["findings"][0]["occurrenceId"]
    );
}

#[test]
fn modified_local_bundle_cannot_sync_under_an_old_seal() {
    let mut tampered = record();
    tampered.bundle.candidates[0].title = "Tampered after sealing".into();
    assert!(
        build_clark_security_cloud_export(&tampered, Uuid::new_v4(), Uuid::new_v4())
            .unwrap_err()
            .contains("stale or has been modified")
    );
}
