use std::path::Path;

use super::*;

#[test]
fn secret_bearing_paths_are_rejected_on_posix_and_windows_shapes() {
    assert!(sensitive_path(Path::new("service/.env.production")));
    assert!(sensitive_path(Path::new("keys/private.pem")));
    assert!(sensitive_path(Path::new(".aws/credentials")));
    assert!(sensitive_path(Path::new(".aws/config")));
    assert!(sensitive_path(Path::new(".config/gh/hosts.yml")));
    assert!(sensitive_path(Path::new(".docker/config.json")));
    assert!(sensitive_path(Path::new(".kube/config")));
    assert!(sensitive_path(Path::new(
        r"C:\Users\Clark Code\.aws\credentials"
    )));
    assert!(!sensitive_path(Path::new("src/config.rs")));
}

#[test]
fn probe_paths_are_project_relative_on_posix_and_windows() {
    assert!(project_relative_path("src/config.rs"));
    assert!(!project_relative_path("/etc/passwd"));
    assert!(!project_relative_path(r"C:\Users\Clark Code\secret.txt"));
    assert!(!project_relative_path(r"\\server\share\secret.txt"));
}

#[test]
fn source_receipts_redact_secret_looking_assignments() {
    assert_eq!(
        redact_source_line("API_KEY=do-not-leak"),
        "[REDACTED: possible secret-bearing line]"
    );
    assert_eq!(
        redact_source_line("Authorization: Bearer do-not-leak"),
        "[REDACTED: possible secret-bearing line]"
    );
    assert_eq!(redact_source_line("let retries = 3;"), "let retries = 3;");
}

#[test]
fn probe_recipes_cannot_escalate_source_receipts_to_live_or_poc_evidence() {
    assert!(probe_can_verify_kind(
        ScoutEvidenceKind::SourceTrace,
        ProbeOperation::SourceSlice
    ));
    assert!(probe_can_verify_kind(
        ScoutEvidenceKind::Census,
        ProbeOperation::JsonArrayCount
    ));
    assert!(!probe_can_verify_kind(
        ScoutEvidenceKind::LiveState,
        ProbeOperation::SourceSlice
    ));
    assert!(!probe_can_verify_kind(
        ScoutEvidenceKind::OfflinePoc,
        ProbeOperation::SourceSlice
    ));
}

#[test]
fn source_ranges_are_bounded() {
    let args = ProbeArgs {
        action: ProbeAction::Record,
        run_id: "run".into(),
        evidence_id: "evidence".into(),
        target_evidence_id: None,
        operation: Some(ProbeOperation::SourceSlice),
        path: Some("src/lib.rs".into()),
        scope: Some("repo".into()),
        line_start: Some(1),
        line_end: Some(MAX_SOURCE_LINES + 1),
        needle: None,
        json_pointer: None,
    };
    assert!(recipe_from_args(&args).unwrap_err().contains("at most"));
}
