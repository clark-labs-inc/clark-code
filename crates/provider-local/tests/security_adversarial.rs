//! Adversarial acceptance simulation for the deterministic Security contract.
//!
//! The corpus is intentionally vulnerable and lives under `harness/fixtures`.
//! These tests prove coverage and evidence invariants independently of any
//! model's ability to discover the seeded issues.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use provider_local::security::{
    collect_security_diff_inventory, collect_security_inventory, finalize_security_diff,
    finalize_security_scan, SecurityAttackPath, SecurityCandidate, SecurityConfidence,
    SecurityCoverage, SecurityCoverageStatus, SecurityDiffInventory, SecurityDiffKind,
    SecurityDisposition, SecurityInventory, SecurityLocation, SecurityPocControl,
    SecurityPocEvidence, SecurityPocLedger, SecurityPocOutcome, SecurityPocReceipt,
    SecurityScanBundle, SecurityScanMode, SecurityScanPhase, SecuritySeverity, SecurityThreatModel,
    SecurityValidation, SECURITY_SCAN_CONTRACT_VERSION,
};
use provider_local::{LocalExecutor, SECURITY_MODEL};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Oracle {
    expected_findings: Vec<OracleFinding>,
    safe_control_paths: Vec<String>,
    excluded_paths: Vec<String>,
}

#[derive(Deserialize)]
struct OracleFinding {
    id: String,
    path: String,
    cwe: String,
    severity: SecuritySeverity,
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../harness/fixtures/security-vulnerable-repo")
}

fn oracle() -> Oracle {
    serde_json::from_str(include_str!(
        "../../../harness/security-vulnerable-oracle.json"
    ))
    .expect("valid adversarial Security oracle")
}

fn candidate(id: &str, path: &str, cwe: &str, severity: SecuritySeverity) -> SecurityCandidate {
    let (positive, negative) = poc_receipt_ids(id);
    let slug = id.to_ascii_lowercase();
    SecurityCandidate {
        candidate_id: id.into(),
        rule_id: format!("fixture.{slug}"),
        identity_anchor: format!("{slug}-root-control"),
        identity_instance: None,
        title: format!("Seeded {cwe} vulnerability {id}"),
        summary: "An attacker-controlled fixture value reaches a security-sensitive operation."
            .into(),
        category: "adversarial-fixture".into(),
        cwe: vec![cwe.into()],
        severity,
        confidence: SecurityConfidence::High,
        source: SecurityLocation {
            path: path.into(),
            line: Some(1),
            description: "Attacker-controlled fixture input".into(),
        },
        control: SecurityLocation {
            path: path.into(),
            line: Some(1),
            description: "The expected security control is absent or fails open".into(),
        },
        sink: SecurityLocation {
            path: path.into(),
            line: Some(1),
            description: "Security-sensitive fixture operation".into(),
        },
        impact: format!("{cwe}: seeded adversarial fixture impact"),
        remediation: "Apply the control demonstrated by the corresponding safe fixture.".into(),
        validation: SecurityValidation {
            disposition: SecurityDisposition::Reportable,
            evidence: "The oracle-backed source-to-sink trace is reachable in this fixture".into(),
            counterevidence: vec!["The repository is inert test data and cannot be deployed".into()],
        },
        poc: SecurityPocEvidence {
            goal: format!("Reproduce {cwe} and prove the seeded safe control does not trigger it"),
            outcome: SecurityPocOutcome::Reproduced,
            positive_receipt_id: Some(positive),
            negative_receipt_id: Some(negative),
            limitations: vec!["Contract fixture uses host-issued simulated receipts".into()],
        },
        attack_path: Some(SecurityAttackPath {
            attacker: "Fixture tenant or unauthenticated caller".into(),
            entrypoint: path.into(),
            preconditions: vec!["The fictional service is running".into()],
            path: vec![
                "attacker input".into(),
                "missing or broken control".into(),
                "security-sensitive sink".into(),
            ],
            likelihood: "high inside the deliberately vulnerable fixture".into(),
        }),
    }
}

fn suppressed_control(path: &str) -> SecurityCandidate {
    let candidate_id = format!("safe-control:{}", path.replace(['/', '.'], "-"));
    let (positive, negative) = poc_receipt_ids(&candidate_id);
    let identity_instance = path.replace(['/', '.'], "-");
    SecurityCandidate {
        candidate_id,
        rule_id: "fixture.safe-control".into(),
        identity_anchor: "protected-operation-control".into(),
        identity_instance: Some(identity_instance),
        title: "Seeded safe control blocks the candidate path".into(),
        summary: "The fixture contains an explicit control that prevents exploitation.".into(),
        category: "adversarial-fixture".into(),
        cwe: Vec::new(),
        severity: SecuritySeverity::Low,
        confidence: SecurityConfidence::High,
        source: SecurityLocation {
            path: path.into(),
            line: Some(1),
            description: "Potentially attacker-controlled fixture input".into(),
        },
        control: SecurityLocation {
            path: path.into(),
            line: Some(1),
            description: "Explicit allowlist, parameter binding, or canonical-path check".into(),
        },
        sink: SecurityLocation {
            path: path.into(),
            line: Some(1),
            description: "Guarded security-sensitive operation".into(),
        },
        impact: "No demonstrated impact because the control blocks the path".into(),
        remediation: "Retain the existing explicit security control.".into(),
        validation: SecurityValidation {
            disposition: SecurityDisposition::Suppressed,
            evidence: "A concrete control prevents attacker input from reaching the sink".into(),
            counterevidence: vec!["No bypass is present in the fixture".into()],
        },
        poc: SecurityPocEvidence {
            goal: "Challenge the safe control with malicious and allowed inputs".into(),
            outcome: SecurityPocOutcome::NotReproduced,
            positive_receipt_id: Some(positive),
            negative_receipt_id: Some(negative),
            limitations: vec!["Contract fixture uses host-issued simulated receipts".into()],
        },
        attack_path: None,
    }
}

fn standard_bundle(inventory: &SecurityInventory, oracle: &Oracle) -> SecurityScanBundle {
    let excluded = oracle.excluded_paths.iter().collect::<BTreeSet<_>>();
    let mut candidates = oracle
        .expected_findings
        .iter()
        .map(|finding| candidate(&finding.id, &finding.path, &finding.cwe, finding.severity))
        .collect::<Vec<_>>();
    candidates.extend(
        oracle
            .safe_control_paths
            .iter()
            .map(|path| suppressed_control(path)),
    );
    SecurityScanBundle {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scan_id: "adversarial-standard".into(),
        mode: SecurityScanMode::Standard,
        model: SECURITY_MODEL.into(),
        scope: inventory.scope.clone(),
        inventory_id: inventory.inventory_id.clone(),
        phase: SecurityScanPhase::Reporting,
        threat_model: SecurityThreatModel {
            assets: vec![
                "Tenant records".into(),
                "Session and service credentials".into(),
                "Host filesystem and internal network".into(),
            ],
            trust_boundaries: vec![
                "Internet request to fictional API".into(),
                "Tenant input to privileged process and storage".into(),
            ],
            attacker_inputs: vec!["Route, query, body, cookie, archive, and webhook fields".into()],
            invariants: vec![
                "A tenant cannot cross identity or filesystem boundaries".into(),
                "Untrusted data cannot become code, SQL, markup, or a network destination".into(),
            ],
        },
        coverage: inventory
            .paths
            .iter()
            .map(|path| {
                let is_excluded = excluded.contains(path);
                SecurityCoverage {
                    path: path.clone(),
                    status: if is_excluded {
                        SecurityCoverageStatus::Excluded
                    } else {
                        SecurityCoverageStatus::Reviewed
                    },
                    reason: is_excluded
                        .then(|| "Excluded by fixture SECURITY.md as generated vendor code".into()),
                }
            })
            .collect(),
        supporting_coverage: Vec::new(),
        diff_target: None,
        deep_run_id: None,
        candidates,
    }
}

#[tokio::test]
async fn adversarial_standard_scan_seals_all_seeded_findings_and_suppresses_controls() {
    let root = fixture_root();
    let oracle = oracle();
    let inventory = collect_security_inventory(&LocalExecutor, &root, &root)
        .await
        .expect("inventory the intentionally vulnerable repository");
    let bundle = standard_bundle(&inventory, &oracle);
    let receipts = poc_ledger(&bundle);
    let seal = finalize_security_scan(&bundle, &inventory, &receipts)
        .expect("seal complete adversarial scan");

    assert_eq!(seal.findings.len(), oracle.expected_findings.len());
    assert_eq!(
        seal.candidate_count,
        oracle.expected_findings.len() + oracle.safe_control_paths.len()
    );
    assert_eq!(seal.excluded_files, oracle.excluded_paths.len());
    assert!(seal
        .findings
        .iter()
        .all(|finding| !oracle.safe_control_paths.contains(&finding.source_path)));
}

#[tokio::test]
async fn adversarial_negative_controls_fail_closed() {
    let root = fixture_root();
    let oracle = oracle();
    let inventory = collect_security_inventory(&LocalExecutor, &root, &root)
        .await
        .expect("inventory fixture");
    let complete = standard_bundle(&inventory, &oracle);
    let receipts = poc_ledger(&complete);

    let mut missing_file = complete.clone();
    missing_file.coverage.pop();
    assert!(finalize_security_scan(&missing_file, &inventory, &receipts)
        .unwrap_err()
        .contains("coverage does not match target inventory"));

    let mut stale = complete.clone();
    stale.inventory_id.push_str("-stale");
    assert!(finalize_security_scan(&stale, &inventory, &receipts)
        .unwrap_err()
        .contains("inventoryId is stale"));

    let mut no_attack_path = complete.clone();
    no_attack_path.candidates[0].attack_path = None;
    assert!(
        finalize_security_scan(&no_attack_path, &inventory, &receipts)
            .unwrap_err()
            .contains("has no attackPath")
    );

    let mut invented_path = complete;
    invented_path.candidates[0].source.path = "../outside-repository.ts".into();
    assert!(
        finalize_security_scan(&invented_path, &inventory, &receipts)
            .unwrap_err()
            .contains("is not in the target evidence set")
    );
}

#[tokio::test]
async fn adversarial_working_tree_diff_binds_rename_delete_modify_and_untracked_files() {
    let temp = tempfile::tempdir().expect("temporary Git repository");
    copy_tree(&fixture_root(), temp.path());
    git(temp.path(), &["init", "-q"]);
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-q", "-m", "safe baseline"]);

    std::fs::rename(
        temp.path().join("src/safe/queries.ts"),
        temp.path().join("src/safe/query-renamed.ts"),
    )
    .expect("rename safe query");
    std::fs::remove_file(temp.path().join("src/web/redirect.ts")).expect("delete redirect");
    std::fs::write(
        temp.path().join("src/network/fetch.ts"),
        "export async function fetchPreview(destination: string) { return fetch(destination); }\n",
    )
    .expect("modify SSRF path");
    std::fs::write(
        temp.path().join("src/api/new-upload.ts"),
        "export const uploadPath = (name: string) => `/srv/uploads/${name}`;\n",
    )
    .expect("create untracked upload path");

    let inventory = collect_security_inventory(&LocalExecutor, temp.path(), temp.path())
        .await
        .expect("inventory changed repository");
    let diff = collect_security_diff_inventory(
        &LocalExecutor,
        temp.path(),
        temp.path(),
        SecurityDiffKind::WorkingTree,
        "HEAD",
        None,
    )
    .await
    .expect("collect exact working tree");
    let statuses = diff
        .changed_files
        .iter()
        .map(|file| (file.path.as_str(), file.status.as_str()))
        .collect::<BTreeSet<_>>();
    assert!(statuses.contains(&("src/network/fetch.ts", "modified")));
    assert!(statuses.contains(&("src/web/redirect.ts", "deleted")));
    assert!(statuses
        .iter()
        .any(|(path, status)| path.ends_with("query-renamed.ts") && *status == "renamed"));
    assert!(statuses.contains(&("src/api/new-upload.ts", "added")));

    let bundle = diff_bundle(&inventory, &diff);
    let receipts = poc_ledger(&bundle);
    let seal = finalize_security_diff(&bundle, &inventory, &diff, &receipts)
        .expect("seal exact diff scan");
    assert_eq!(seal.reviewed_files, diff.changed_files.len());
    assert_eq!(seal.findings.len(), 1);

    std::fs::write(
        temp.path().join("src/network/fetch.ts"),
        "export const changedAgain = true;\n",
    )
    .expect("mutate after target receipt");
    let changed = collect_security_diff_inventory(
        &LocalExecutor,
        temp.path(),
        temp.path(),
        SecurityDiffKind::WorkingTree,
        "HEAD",
        None,
    )
    .await
    .expect("refresh exact working tree");
    assert!(
        finalize_security_diff(&bundle, &inventory, &changed, &receipts)
            .unwrap_err()
            .contains("diffTarget is stale")
    );
}

fn diff_bundle(inventory: &SecurityInventory, diff: &SecurityDiffInventory) -> SecurityScanBundle {
    SecurityScanBundle {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scan_id: "adversarial-diff".into(),
        mode: SecurityScanMode::Diff,
        model: SECURITY_MODEL.into(),
        scope: inventory.scope.clone(),
        inventory_id: inventory.inventory_id.clone(),
        phase: SecurityScanPhase::Reporting,
        threat_model: SecurityThreatModel {
            assets: vec!["Internal network and tenant records".into()],
            trust_boundaries: vec!["Changed request handler to privileged service".into()],
            attacker_inputs: vec!["Changed destination and upload path".into()],
            invariants: vec!["A patch cannot widen attacker control of privileged sinks".into()],
        },
        coverage: diff
            .changed_files
            .iter()
            .map(|file| SecurityCoverage {
                path: file.path.clone(),
                status: SecurityCoverageStatus::Reviewed,
                reason: None,
            })
            .collect(),
        supporting_coverage: vec![SecurityCoverage {
            path: "SECURITY.md".into(),
            status: SecurityCoverageStatus::Reviewed,
            reason: None,
        }],
        diff_target: Some(diff.target.clone()),
        deep_run_id: None,
        candidates: vec![candidate(
            "DIFF-SSRF",
            "src/network/fetch.ts",
            "CWE-918",
            SecuritySeverity::High,
        )],
    }
}

fn poc_receipt_ids(candidate_id: &str) -> (String, String) {
    let stem = candidate_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .take(96)
        .collect::<String>();
    (format!("{stem}-positive"), format!("{stem}-negative"))
}

fn poc_ledger(bundle: &SecurityScanBundle) -> SecurityPocLedger {
    let mut ledger = SecurityPocLedger::default();
    for candidate in &bundle.candidates {
        let (positive, negative) = poc_receipt_ids(&candidate.candidate_id);
        for (receipt_id, control, script) in [
            (
                positive,
                SecurityPocControl::Positive,
                "fixture-positive-script",
            ),
            (
                negative,
                SecurityPocControl::Negative,
                "fixture-negative-script",
            ),
        ] {
            ledger
                .record(SecurityPocReceipt {
                    contract_version: SECURITY_SCAN_CONTRACT_VERSION,
                    receipt_id: receipt_id.clone(),
                    scan_id: bundle.scan_id.clone(),
                    candidate_id: candidate.candidate_id.clone(),
                    inventory_id: bundle.inventory_id.clone(),
                    control,
                    language: "javascript".into(),
                    script_sha256: format!("{script}-{}", candidate.candidate_id),
                    expected_observation_sha256: format!(
                        "{script}-observation-{}",
                        candidate.candidate_id
                    ),
                    workspace_sha256: format!("workspace-{}", bundle.inventory_id),
                    stdout_sha256: format!("{script}-stdout"),
                    stderr_sha256: format!("{script}-stderr"),
                    expected_exit_code: 0,
                    exit_code: Some(0),
                    passed: true,
                    containment: "managed_disposable".into(),
                    artifact_path: format!(
                        ".clark/security-scans/{}/poc/{receipt_id}/receipt.json",
                        bundle.scan_id
                    ),
                    execution: None,
                })
                .unwrap();
        }
    }
    ledger
}

fn copy_tree(source: &Path, destination: &Path) {
    for entry in std::fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            std::fs::create_dir_all(&target).expect("create fixture directory");
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Clark Security Simulation")
        .env("GIT_AUTHOR_EMAIL", "security@example.invalid")
        .env("GIT_COMMITTER_NAME", "Clark Security Simulation")
        .env("GIT_COMMITTER_EMAIL", "security@example.invalid")
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
