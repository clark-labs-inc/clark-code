use provider_local::security::{
    finalize_security_scan, SecurityAttackPath, SecurityCandidate, SecurityConfidence,
    SecurityCoverage, SecurityCoverageStatus, SecurityDisposition, SecurityInventory,
    SecurityLocation, SecurityPocControl, SecurityPocEvidence, SecurityPocLedger,
    SecurityPocOutcome, SecurityPocReceipt, SecurityScanBundle, SecurityScanMode,
    SecurityScanPhase, SecuritySeverity, SecurityThreatModel, SecurityValidation,
    SECURITY_SCAN_CONTRACT_VERSION,
};

const SECURITY_SIMULATION_MODEL: &str = "security-model";

fn main() {
    let inventory = SecurityInventory {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scope: ".".into(),
        inventory_id: "fixture-snapshot".into(),
        paths: vec!["src/auth.rs".into(), "src/redirect.rs".into()],
    };
    let complete = fixture(&inventory);
    let receipts = poc_ledger(&complete);
    let seal = finalize_security_scan(&complete, &inventory, &receipts)
        .expect("complete simulated scan must seal");

    let mut missing_coverage = complete.clone();
    missing_coverage.coverage.pop();
    let coverage_error = finalize_security_scan(&missing_coverage, &inventory, &receipts)
        .expect_err("partial coverage must fail");

    let mut missing_attack_path = complete;
    missing_attack_path.candidates[0].attack_path = None;
    let attack_path_error = finalize_security_scan(&missing_attack_path, &inventory, &receipts)
        .expect_err("reportable finding without attack path must fail");

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "simulation": "agent-security-contract-v2",
            "sealedFindingCount": seal.findings.len(),
            "findingId": seal.findings[0].finding_id,
            "negativeControls": {
                "missingCoverage": coverage_error,
                "missingAttackPath": attack_path_error
            }
        }))
        .expect("simulation receipt")
    );
}

fn fixture(inventory: &SecurityInventory) -> SecurityScanBundle {
    SecurityScanBundle {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scan_id: "simulation".into(),
        mode: SecurityScanMode::Standard,
        model: SECURITY_SIMULATION_MODEL.into(),
        scope: inventory.scope.clone(),
        inventory_id: inventory.inventory_id.clone(),
        phase: SecurityScanPhase::Reporting,
        threat_model: SecurityThreatModel {
            assets: vec!["Internal service credentials".into()],
            trust_boundaries: vec!["Tenant request to service network".into()],
            attacker_inputs: vec!["Redirect destination".into()],
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
        candidates: vec![SecurityCandidate {
            candidate_id: "ssrf-redirect".into(),
            rule_id: "server-side-request-forgery.http-client".into(),
            identity_anchor: "redirect-destination-without-network-policy".into(),
            identity_instance: None,
            title: "Redirect destination reaches an internal HTTP client".into(),
            summary: "A tenant-controlled destination crosses the service network boundary.".into(),
            category: "server-side-request-forgery".into(),
            cwe: vec!["CWE-918".into()],
            severity: SecuritySeverity::High,
            confidence: SecurityConfidence::High,
            source: SecurityLocation {
                path: "src/redirect.rs".into(),
                line: Some(8),
                description: "Tenant-supplied redirect destination".into(),
            },
            control: SecurityLocation {
                path: "src/auth.rs".into(),
                line: Some(14),
                description: "User authentication without destination authorization".into(),
            },
            sink: SecurityLocation {
                path: "src/redirect.rs".into(),
                line: Some(18),
                description: "Server-side HTTP request".into(),
            },
            impact: "Tenant can query internal network services".into(),
            remediation: "Allowlist outbound destinations after canonical URL parsing.".into(),
            validation: SecurityValidation {
                disposition: SecurityDisposition::Reportable,
                evidence: "Source trace reaches the request builder unchanged".into(),
                counterevidence: vec!["Requires an authenticated tenant".into()],
            },
            poc: SecurityPocEvidence {
                goal: "Demonstrate internal destination reachability and a public-host control"
                    .into(),
                outcome: SecurityPocOutcome::Reproduced,
                positive_receipt_id: Some("simulation-positive".into()),
                negative_receipt_id: Some("simulation-negative".into()),
                limitations: Vec::new(),
            },
            attack_path: Some(SecurityAttackPath {
                attacker: "Authenticated tenant".into(),
                entrypoint: "POST /redirect".into(),
                preconditions: vec!["Tenant account".into()],
                path: vec![
                    "destination JSON field".into(),
                    "request builder".into(),
                    "internal HTTP client".into(),
                ],
                likelihood: "medium: normal tenant access is sufficient".into(),
            }),
        }],
    }
}

fn poc_ledger(bundle: &SecurityScanBundle) -> SecurityPocLedger {
    let mut ledger = SecurityPocLedger::default();
    for (id, control, script) in [
        (
            "simulation-positive",
            SecurityPocControl::Positive,
            "positive-script",
        ),
        (
            "simulation-negative",
            SecurityPocControl::Negative,
            "negative-script",
        ),
    ] {
        ledger
            .record(SecurityPocReceipt {
                contract_version: SECURITY_SCAN_CONTRACT_VERSION,
                receipt_id: id.into(),
                scan_id: bundle.scan_id.clone(),
                candidate_id: bundle.candidates[0].candidate_id.clone(),
                inventory_id: bundle.inventory_id.clone(),
                control,
                language: "python".into(),
                script_sha256: script.into(),
                expected_observation_sha256: format!("{script}-observation"),
                workspace_sha256: "fixture-workspace".into(),
                stdout_sha256: format!("{script}-stdout"),
                stderr_sha256: format!("{script}-stderr"),
                expected_exit_code: 0,
                exit_code: Some(0),
                passed: true,
                containment: "managed_disposable".into(),
                artifact_path: format!(".agent/security-scans/simulation/poc/{id}.json"),
                execution: None,
            })
            .unwrap();
    }
    ledger
}
