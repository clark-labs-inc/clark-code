use std::path::Path;
use std::process::Command;

use provider_local::security::{
    collect_security_diff_inventory, collect_security_inventory, finalize_security_diff,
    SecurityAttackPath, SecurityCandidate, SecurityConfidence, SecurityCoverage,
    SecurityCoverageStatus, SecurityDiffKind, SecurityDisposition, SecurityLocation,
    SecurityPocControl, SecurityPocEvidence, SecurityPocLedger, SecurityPocOutcome,
    SecurityPocReceipt, SecurityScanBundle, SecurityScanMode, SecurityScanPhase, SecuritySeverity,
    SecurityThreatModel, SecurityValidation, SECURITY_SCAN_CONTRACT_VERSION,
};
use provider_local::{LocalExecutor, SECURITY_MODEL};

#[tokio::main]
async fn main() {
    let temp = tempfile::tempdir().expect("temporary repository");
    let root = temp.path();
    git(root, &["init", "-q"]);
    std::fs::create_dir(root.join("src")).expect("source directory");
    std::fs::write(
        root.join("src/auth.rs"),
        "pub fn authenticated() -> bool { true }\n",
    )
    .expect("auth fixture");
    std::fs::write(
        root.join("src/proxy.rs"),
        "pub fn destination() -> &'static str { \"public.example\" }\n",
    )
    .expect("proxy fixture");
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "baseline"]);

    std::fs::write(
        root.join("src/proxy.rs"),
        "pub fn destination(input: &str) -> &str { input }\n",
    )
    .expect("changed proxy fixture");

    let inventory = collect_security_inventory(&LocalExecutor, root, root)
        .await
        .expect("repository inventory");
    let diff = collect_security_diff_inventory(
        &LocalExecutor,
        root,
        root,
        SecurityDiffKind::WorkingTree,
        "HEAD",
        None,
    )
    .await
    .expect("working-tree diff inventory");
    let complete = fixture(&inventory.inventory_id, &diff.target);
    let receipts = poc_ledger(&complete);
    let seal = finalize_security_diff(&complete, &inventory, &diff, &receipts)
        .expect("complete simulated diff must seal");

    let mut missing_changed_file = complete.clone();
    missing_changed_file.coverage.clear();
    let coverage_error =
        finalize_security_diff(&missing_changed_file, &inventory, &diff, &receipts)
            .expect_err("partial changed-file coverage must fail");

    std::fs::write(
        root.join("src/proxy.rs"),
        "pub fn destination(input: &str) -> String { input.trim().to_owned() }\n",
    )
    .expect("stale target mutation");
    let changed_diff = collect_security_diff_inventory(
        &LocalExecutor,
        root,
        root,
        SecurityDiffKind::WorkingTree,
        "HEAD",
        None,
    )
    .await
    .expect("changed working-tree inventory");
    let stale_error = finalize_security_diff(&complete, &inventory, &changed_diff, &receipts)
        .expect_err("stale exact diff must fail");

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "simulation": "clark-security-diff-contract-v2",
            "resolvedBase": diff.resolved_base,
            "resolvedHead": diff.resolved_head,
            "diffTargetId": diff.target.target_id,
            "changedFiles": diff.changed_files,
            "sealedFindingCount": seal.findings.len(),
            "findingId": seal.findings[0].finding_id,
            "negativeControls": {
                "missingChangedFile": coverage_error,
                "staleDiffTarget": stale_error
            }
        }))
        .expect("simulation receipt")
    );
}

fn fixture(
    inventory_id: &str,
    target: &provider_local::security::SecurityDiffTarget,
) -> SecurityScanBundle {
    SecurityScanBundle {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scan_id: "diff-simulation".into(),
        mode: SecurityScanMode::Diff,
        model: SECURITY_MODEL.into(),
        scope: ".".into(),
        inventory_id: inventory_id.into(),
        phase: SecurityScanPhase::Reporting,
        threat_model: SecurityThreatModel {
            assets: vec!["Internal service credentials".into()],
            trust_boundaries: vec!["Tenant input to service network".into()],
            attacker_inputs: vec!["Proxy destination".into()],
            invariants: vec!["Tenant input cannot select internal hosts".into()],
        },
        coverage: vec![SecurityCoverage {
            path: "src/proxy.rs".into(),
            status: SecurityCoverageStatus::Reviewed,
            reason: None,
        }],
        supporting_coverage: vec![SecurityCoverage {
            path: "src/auth.rs".into(),
            status: SecurityCoverageStatus::Reviewed,
            reason: None,
        }],
        diff_target: Some(target.clone()),
        deep_run_id: None,
        candidates: vec![SecurityCandidate {
            candidate_id: "changed-destination-policy".into(),
            rule_id: "server-side-request-forgery.http-client".into(),
            identity_anchor: "proxy-destination-without-network-policy".into(),
            identity_instance: None,
            title: "Changed proxy destination reaches a server-side request".into(),
            summary: "A tenant-controlled destination now crosses the service network boundary."
                .into(),
            category: "server-side-request-forgery".into(),
            cwe: vec!["CWE-918".into()],
            severity: SecuritySeverity::High,
            confidence: SecurityConfidence::High,
            source: SecurityLocation {
                path: "src/proxy.rs".into(),
                line: Some(1),
                description: "Changed function now accepts a caller-controlled destination".into(),
            },
            control: SecurityLocation {
                path: "src/auth.rs".into(),
                line: Some(1),
                description: "Authentication does not constrain the destination".into(),
            },
            sink: SecurityLocation {
                path: "src/proxy.rs".into(),
                line: Some(1),
                description: "Destination is returned to the server-side request path".into(),
            },
            impact: "Authenticated tenant can select an internal network destination".into(),
            remediation: "Allowlist outbound destinations after canonical URL parsing.".into(),
            validation: SecurityValidation {
                disposition: SecurityDisposition::Reportable,
                evidence: "Exact patch replaces a constant destination with caller input".into(),
                counterevidence: vec!["The route remains authenticated".into()],
            },
            poc: SecurityPocEvidence {
                goal: "Exercise the caller-controlled destination and a fixed public control"
                    .into(),
                outcome: SecurityPocOutcome::Reproduced,
                positive_receipt_id: Some("diff-positive".into()),
                negative_receipt_id: Some("diff-negative".into()),
                limitations: Vec::new(),
            },
            attack_path: Some(SecurityAttackPath {
                attacker: "Authenticated tenant".into(),
                entrypoint: "Proxy destination parameter".into(),
                preconditions: vec!["Tenant account".into()],
                path: vec![
                    "attacker destination".into(),
                    "changed destination function".into(),
                    "server-side request".into(),
                ],
                likelihood: "medium: ordinary authenticated access is sufficient".into(),
            }),
        }],
    }
}

fn poc_ledger(bundle: &SecurityScanBundle) -> SecurityPocLedger {
    let mut ledger = SecurityPocLedger::default();
    for (id, control, script) in [
        (
            "diff-positive",
            SecurityPocControl::Positive,
            "positive-script",
        ),
        (
            "diff-negative",
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
                artifact_path: format!(".clark/security-scans/diff-simulation/poc/{id}.json"),
                execution: None,
            })
            .unwrap();
    }
    ledger
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Clark Security Simulation")
        .env("GIT_AUTHOR_EMAIL", "security@example.com")
        .env("GIT_COMMITTER_NAME", "Clark Security Simulation")
        .env("GIT_COMMITTER_EMAIL", "security@example.com")
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
