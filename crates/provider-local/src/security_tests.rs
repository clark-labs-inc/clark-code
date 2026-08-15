use super::test_support::{poc_ledger as receipt_ledger, reproduced_poc};
use super::*;

pub(super) fn inventory() -> SecurityInventory {
    let paths = vec!["src/auth.rs".into(), "src/routes.rs".into()];
    SecurityInventory {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scope: ".".into(),
        inventory_id: inventory_digest(".", &paths),
        paths,
    }
}

pub(super) fn poc_ledger() -> SecurityPocLedger {
    receipt_ledger(&inventory().inventory_id)
}

fn reportable_candidate() -> SecurityCandidate {
    SecurityCandidate {
        candidate_id: "candidate-1".into(),
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
            path: "src/routes.rs".into(),
            line: Some(12),
            description: "Untrusted redirect URL from the request".into(),
        },
        control: SecurityLocation {
            path: "src/auth.rs".into(),
            line: Some(30),
            description: "Authentication runs but destination policy is absent".into(),
        },
        sink: SecurityLocation {
            path: "src/routes.rs".into(),
            line: Some(19),
            description: "HTTP client follows the supplied destination".into(),
        },
        impact: "Authenticated attacker reaches internal metadata services".into(),
        remediation: "Allowlist outbound destinations after canonical URL parsing.".into(),
        validation: SecurityValidation {
            disposition: SecurityDisposition::Reportable,
            evidence: "Static trace confirms the request value reaches the HTTP client".into(),
            counterevidence: vec!["Endpoint requires a normal user session".into()],
        },
        poc: reproduced_poc(),
        attack_path: Some(SecurityAttackPath {
            attacker: "Authenticated tenant user".into(),
            entrypoint: "POST /fetch".into(),
            preconditions: vec!["Ordinary tenant account".into()],
            path: vec![
                "JSON destination is parsed".into(),
                "Destination reaches the shared HTTP client".into(),
            ],
            likelihood: "medium".into(),
        }),
    }
}

pub(super) fn bundle() -> SecurityScanBundle {
    let inventory = inventory();
    SecurityScanBundle {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scan_id: "scan-1".into(),
        mode: SecurityScanMode::Standard,
        model: "security-test-model".into(),
        scope: inventory.scope.clone(),
        inventory_id: inventory.inventory_id,
        phase: SecurityScanPhase::Reporting,
        threat_model: SecurityThreatModel {
            assets: vec!["Tenant secrets".into()],
            trust_boundaries: vec!["Public API to application service".into()],
            attacker_inputs: vec!["Request JSON".into()],
            invariants: vec!["Tenant users cannot select internal destinations".into()],
        },
        coverage: vec![
            SecurityCoverage {
                path: "src/auth.rs".into(),
                status: SecurityCoverageStatus::Reviewed,
                reason: None,
            },
            SecurityCoverage {
                path: "src/routes.rs".into(),
                status: SecurityCoverageStatus::Reviewed,
                reason: None,
            },
        ],
        supporting_coverage: Vec::new(),
        diff_target: None,
        deep_run_id: None,
        candidates: vec![reportable_candidate()],
    }
}

fn diff_inventory() -> SecurityDiffInventory {
    SecurityDiffInventory {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scope: ".".into(),
        target: SecurityDiffTarget {
            kind: SecurityDiffKind::WorkingTree,
            base: "HEAD".into(),
            head: None,
            target_id: "diff-target-1".into(),
        },
        resolved_base: "base-commit".into(),
        resolved_head: "working-tree-object".into(),
        changed_files: vec![SecurityDiffFile {
            path: "src/routes.rs".into(),
            previous_path: None,
            status: "modified".into(),
        }],
    }
}

fn diff_bundle() -> SecurityScanBundle {
    let mut bundle = bundle();
    let diff = diff_inventory();
    bundle.mode = SecurityScanMode::Diff;
    bundle.coverage = vec![SecurityCoverage {
        path: "src/routes.rs".into(),
        status: SecurityCoverageStatus::Reviewed,
        reason: None,
    }];
    bundle.supporting_coverage = vec![SecurityCoverage {
        path: "src/auth.rs".into(),
        status: SecurityCoverageStatus::Reviewed,
        reason: None,
    }];
    bundle.diff_target = Some(diff.target);
    bundle
}

#[test]
fn complete_bundle_seals_with_stable_semantic_identity() {
    let inventory = inventory();
    let ledger = poc_ledger();
    let first = finalize_security_scan(&bundle(), &inventory, &ledger).unwrap();
    let mut moved = bundle();
    moved.candidates[0].source.line = Some(900);
    moved.candidates[0].control.line = Some(901);
    moved.candidates[0].sink.line = Some(902);
    let second = finalize_security_scan(&moved, &inventory, &ledger).unwrap();
    assert_eq!(first.findings.len(), 1);
    assert_eq!(first.findings[0].finding_id, second.findings[0].finding_id);
    assert_ne!(first.bundle_digest, second.bundle_digest);
}

#[test]
fn missing_coverage_cannot_be_reported_as_a_clean_scan() {
    let inventory = inventory();
    let mut incomplete = bundle();
    incomplete.coverage.pop();
    let error = finalize_security_scan(&incomplete, &inventory, &poc_ledger()).unwrap_err();
    assert!(error.contains("coverage does not match target inventory"));
    assert!(error.contains("src/routes.rs"));
}

#[test]
fn stale_inventory_is_rejected() {
    let inventory = inventory();
    let mut stale = bundle();
    stale.inventory_id = "old-snapshot".into();
    assert!(finalize_security_scan(&stale, &inventory, &poc_ledger())
        .unwrap_err()
        .contains("inventoryId is stale"));
}

#[test]
fn reportable_candidate_requires_attack_path_evidence() {
    let inventory = inventory();
    let mut incomplete = bundle();
    incomplete.candidates[0].attack_path = None;
    assert!(
        finalize_security_scan(&incomplete, &inventory, &poc_ledger())
            .unwrap_err()
            .contains("has no attackPath")
    );
}

#[test]
fn candidates_require_stable_agent_identity_and_concrete_locations() {
    let inventory = inventory();
    let mut invalid = bundle();
    invalid.candidates[0].identity_anchor = "src/routes.rs:19".into();
    assert!(finalize_security_scan(&invalid, &inventory, &poc_ledger())
        .unwrap_err()
        .contains("identityAnchor must be a lowercase stable slug"));

    let mut missing_line = bundle();
    missing_line.candidates[0].control.line = None;
    assert!(
        finalize_security_scan(&missing_line, &inventory, &poc_ledger())
            .unwrap_err()
            .contains("requires a concrete one-based line")
    );

    let mut duplicate = bundle();
    let mut second = duplicate.candidates[0].clone();
    second.candidate_id = "candidate-2".into();
    duplicate.candidates.push(second);
    assert!(
        finalize_security_scan(&duplicate, &inventory, &poc_ledger())
            .unwrap_err()
            .contains("same Security scanner identity")
    );
}

#[test]
fn suppressed_candidate_does_not_become_a_finding() {
    let inventory = inventory();
    let mut safe = bundle();
    safe.candidates[0].validation.disposition = SecurityDisposition::Suppressed;
    safe.candidates[0].attack_path = None;
    safe.candidates[0].poc.outcome = SecurityPocOutcome::NotReproduced;
    let seal = finalize_security_scan(&safe, &inventory, &poc_ledger()).unwrap();
    assert!(seal.findings.is_empty());
    assert_eq!(seal.candidate_count, 1);
}

#[test]
fn standard_finalizer_rejects_other_modes() {
    let inventory = inventory();
    for mode in [SecurityScanMode::Diff, SecurityScanMode::Deep] {
        let mut unsupported = bundle();
        unsupported.mode = mode;
        assert!(
            finalize_security_scan(&unsupported, &inventory, &poc_ledger())
                .unwrap_err()
                .contains("expected `Standard`")
        );
    }
}

#[test]
fn complete_diff_bundle_seals_changed_and_supporting_coverage() {
    let seal = finalize_security_diff(
        &diff_bundle(),
        &inventory(),
        &diff_inventory(),
        &poc_ledger(),
    )
    .unwrap();
    assert_eq!(seal.diff_target_id.as_deref(), Some("diff-target-1"));
    assert_eq!(seal.reviewed_files, 1);
    assert_eq!(seal.supporting_files, 1);
    assert_eq!(seal.findings.len(), 1);
}

#[test]
fn diff_finalizer_rejects_stale_target_and_missing_changed_file() {
    let mut stale = diff_bundle();
    stale
        .diff_target
        .as_mut()
        .unwrap()
        .target_id
        .push_str("-old");
    assert!(
        finalize_security_diff(&stale, &inventory(), &diff_inventory(), &poc_ledger())
            .unwrap_err()
            .contains("diffTarget is stale")
    );

    let mut incomplete = diff_bundle();
    incomplete.coverage.clear();
    let error = finalize_security_diff(&incomplete, &inventory(), &diff_inventory(), &poc_ledger())
        .unwrap_err();
    assert!(error.contains("changed-file inventory"));
    assert!(error.contains("src/routes.rs"));
}

#[test]
fn diff_candidate_must_touch_a_changed_path() {
    let mut unrelated = diff_bundle();
    unrelated.candidates[0].source.path = "src/auth.rs".into();
    unrelated.candidates[0].sink.path = "src/auth.rs".into();
    assert!(
        finalize_security_diff(&unrelated, &inventory(), &diff_inventory(), &poc_ledger(),)
            .unwrap_err()
            .contains("does not touch a changed path")
    );
}

#[tokio::test]
async fn inventory_is_sorted_and_excludes_scan_outputs() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::create_dir_all(temp.path().join(".agent/security-scans/scan-1")).unwrap();
    std::fs::write(temp.path().join("src/z.rs"), "z").unwrap();
    std::fs::write(temp.path().join("src/a.rs"), "a").unwrap();
    std::fs::write(
        temp.path().join(".agent/security-scans/scan-1/scan.json"),
        "{}",
    )
    .unwrap();
    let inventory =
        collect_security_inventory(&crate::exec::LocalExecutor, temp.path(), temp.path())
            .await
            .unwrap();
    assert_eq!(inventory.paths, vec!["src/a.rs", "src/z.rs"]);
}

#[tokio::test]
async fn inventory_accepts_an_explicit_single_file_scope() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("focused.rs");
    std::fs::write(&target, "fn focused() {}").unwrap();

    let inventory = collect_security_inventory(&crate::exec::LocalExecutor, temp.path(), &target)
        .await
        .unwrap();

    assert_eq!(inventory.scope, "focused.rs");
    assert_eq!(inventory.paths, vec!["focused.rs"]);
}

#[tokio::test]
async fn inventory_id_is_stable_when_a_file_is_rewritten_with_identical_bytes() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("same.rs"), "identical bytes").unwrap();
    let first = collect_security_inventory(&crate::exec::LocalExecutor, temp.path(), temp.path())
        .await
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    // Rewriting identical bytes updates mtime but must not rotate the id.
    std::fs::write(temp.path().join("same.rs"), "identical bytes").unwrap();
    let second = collect_security_inventory(&crate::exec::LocalExecutor, temp.path(), temp.path())
        .await
        .unwrap();
    assert_eq!(first.inventory_id, second.inventory_id);
}

#[tokio::test]
async fn inventory_id_changes_when_file_contents_change_without_renaming() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("same.rs"), "before").unwrap();
    let first = collect_security_inventory(&crate::exec::LocalExecutor, temp.path(), temp.path())
        .await
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    std::fs::write(temp.path().join("same.rs"), "after with a new length").unwrap();
    let second = collect_security_inventory(&crate::exec::LocalExecutor, temp.path(), temp.path())
        .await
        .unwrap();
    assert_ne!(first.inventory_id, second.inventory_id);
    assert_eq!(first.paths, second.paths);
}

fn git(root: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Security scanner Test")
        .env("GIT_AUTHOR_EMAIL", "security@example.com")
        .env("GIT_COMMITTER_NAME", "Security scanner Test")
        .env("GIT_COMMITTER_EMAIL", "security@example.com")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[tokio::test]
async fn working_tree_diff_inventory_covers_all_git_change_shapes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    std::fs::write(root.join("modified.rs"), "before\n").unwrap();
    std::fs::write(root.join("old.rs"), "rename me\n").unwrap();
    std::fs::write(root.join("deleted.rs"), "delete me\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "initial"]);

    std::fs::write(root.join("modified.rs"), "after\n").unwrap();
    std::fs::rename(root.join("old.rs"), root.join("renamed.rs")).unwrap();
    std::fs::remove_file(root.join("deleted.rs")).unwrap();
    std::fs::write(root.join("added.rs"), "new\n").unwrap();

    let first = collect_security_diff_inventory(
        &crate::exec::LocalExecutor,
        root,
        root,
        SecurityDiffKind::WorkingTree,
        "HEAD",
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        first
            .changed_files
            .iter()
            .map(|file| (file.path.as_str(), file.status.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("added.rs", "added"),
            ("deleted.rs", "deleted"),
            ("modified.rs", "modified"),
            ("renamed.rs", "renamed"),
        ]
    );
    assert_eq!(
        first.changed_files[3].previous_path.as_deref(),
        Some("old.rs")
    );

    std::fs::create_dir_all(root.join(".agent/security-scans/scan-1")).unwrap();
    std::fs::write(
        root.join(".agent/security-scans/scan-1/scan.json"),
        "{\"phase\":\"reporting\"}\n",
    )
    .unwrap();
    let with_scan_output = collect_security_diff_inventory(
        &crate::exec::LocalExecutor,
        root,
        root,
        SecurityDiffKind::WorkingTree,
        "HEAD",
        None,
    )
    .await
    .unwrap();
    assert_eq!(first.target.target_id, with_scan_output.target.target_id);
    assert_eq!(first.changed_files, with_scan_output.changed_files);

    std::fs::write(root.join("added.rs"), "newer\n").unwrap();
    let second = collect_security_diff_inventory(
        &crate::exec::LocalExecutor,
        root,
        root,
        SecurityDiffKind::WorkingTree,
        "HEAD",
        None,
    )
    .await
    .unwrap();
    assert_ne!(first.target.target_id, second.target.target_id);
}

#[tokio::test]
async fn range_diff_resolves_symbolic_revisions_and_honors_scope() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::create_dir(root.join("docs")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "one\n").unwrap();
    std::fs::write(root.join("docs/readme.md"), "one\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "initial"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("src/lib.rs"), "two\n").unwrap();
    std::fs::write(root.join("docs/readme.md"), "two\n").unwrap();
    git(root, &["commit", "-qam", "second"]);

    let diff = collect_security_diff_inventory(
        &crate::exec::LocalExecutor,
        root,
        &root.join("src"),
        SecurityDiffKind::Range,
        &base,
        Some("HEAD"),
    )
    .await
    .unwrap();
    assert_eq!(diff.scope, "src");
    assert_eq!(diff.target.head.as_deref(), Some("HEAD"));
    assert_eq!(diff.changed_files.len(), 1);
    assert_eq!(diff.changed_files[0].path, "src/lib.rs");
    assert_eq!(diff.resolved_head, git(root, &["rev-parse", "HEAD"]));
}
