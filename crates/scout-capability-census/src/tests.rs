use super::*;

fn snapshot(executables: &[&str]) -> SystemSnapshot {
    SystemSnapshot {
        platform: "test".into(),
        architecture: "test-arch".into(),
        executable_names: executables.iter().map(|value| (*value).into()).collect(),
        environment_names: vec![
            "AWS_PROFILE".into(),
            "GITHUB_TOKEN".into(),
            "UNRELATED_SECRET_CANARY".into(),
        ],
        credential_surfaces: vec!["aws_config".into(), "github_cli_hosts".into()],
        executables_truncated: false,
        environment_names_truncated: false,
    }
}

fn config(root: &std::path::Path) -> CensusConfig {
    CensusConfig {
        scan_roots: vec![root.to_path_buf()],
        limits: CensusLimits::default(),
    }
}

#[test]
fn secret_canaries_never_reach_receipt() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "PUBLIC_NAME=visible-canary\nSECRET_TOKEN=secret-canary\nAWS_PROFILE=production-canary\n",
    )
    .unwrap();
    let receipt = run_with_snapshot(config(temp.path()), snapshot(&["git", "aws"])).unwrap();
    let encoded = serde_json::to_string(&receipt).unwrap();
    assert!(encoded.contains("PUBLIC_NAME"));
    assert!(encoded.contains("SECRET_TOKEN"));
    assert!(encoded.contains("AWS_PROFILE"));
    assert!(!encoded.contains("visible-canary"));
    assert!(!encoded.contains("secret-canary"));
    assert!(!encoded.contains("production-canary"));
    assert!(!encoded.contains("UNRELATED_SECRET_CANARY"));
    assert!(!receipt.redaction.values_emitted);
    assert!(!receipt.redaction.discovered_executables_executed);
}

#[test]
fn missing_cloud_clis_are_explicit_rust_fallback_gaps() {
    let temp = tempfile::tempdir().unwrap();
    let receipt = run_with_snapshot(config(temp.path()), snapshot(&["git", "jq"])).unwrap();
    let aws = receipt
        .rust_fallback_gaps
        .iter()
        .find(|gap| gap.missing_tool == "aws")
        .unwrap();
    let gcp = receipt
        .rust_fallback_gaps
        .iter()
        .find(|gap| gap.missing_tool == "gcloud")
        .unwrap();
    assert_eq!(aws.state, "missing");
    assert_eq!(gcp.state, "missing");
    assert!(aws
        .constraints
        .iter()
        .any(|constraint| constraint.contains("pure-Rust SDK fallback")));
    assert!(receipt.coverage.rust_fallback_missing >= 2);
}

#[test]
fn semantic_digest_ignores_absolute_root_and_host_labels() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for root in [first.path(), second.path()] {
        std::fs::create_dir(root.join("service")).unwrap();
        std::fs::write(root.join("service/.env.example"), "API_NAME=x\n").unwrap();
    }
    let first_receipt = run_with_snapshot(config(first.path()), snapshot(&["git", "jq"])).unwrap();
    let mut second_snapshot = snapshot(&["git", "jq"]);
    second_snapshot.platform = "another-os".into();
    second_snapshot.architecture = "another-arch".into();
    let second_receipt = run_with_snapshot(config(second.path()), second_snapshot).unwrap();
    assert_ne!(
        first_receipt.roots[0].resolved_path,
        second_receipt.roots[0].resolved_path
    );
    assert_eq!(
        first_receipt.semantic_digest_sha256,
        second_receipt.semantic_digest_sha256
    );
}

#[test]
fn semantic_digest_does_not_encode_dotenv_value_or_value_length() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join(".env"), "SECRET_TOKEN=x\n").unwrap();
    std::fs::write(
        second.path().join(".env"),
        "SECRET_TOKEN=a-much-longer-secret-value\n",
    )
    .unwrap();
    let first_receipt = run_with_snapshot(config(first.path()), snapshot(&["git"])).unwrap();
    let second_receipt = run_with_snapshot(config(second.path()), snapshot(&["git"])).unwrap();
    assert_ne!(
        first_receipt.dotenv_files[0].bytes_read,
        second_receipt.dotenv_files[0].bytes_read
    );
    assert_eq!(
        first_receipt.semantic_digest_sha256,
        second_receipt.semantic_digest_sha256
    );
}

#[test]
fn file_and_byte_bounds_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".env"), "FIRST=one\nSECOND=two\n").unwrap();
    std::fs::write(temp.path().join(".env.local"), "THIRD=three\n").unwrap();
    let mut request = config(temp.path());
    request.limits.max_dotenv_files = 1;
    request.limits.max_total_bytes = 32;
    request.limits.max_file_bytes = 32;
    request.limits.max_keys_per_file = 1;
    let receipt = run_with_snapshot(request, snapshot(&[])).unwrap();
    assert_eq!(receipt.dotenv_files.len(), 1);
    assert!(receipt.truncation.dotenv_files);
    assert!(receipt.dotenv_files[0].key_names_truncated);
}

#[cfg(unix)]
#[test]
fn scanner_does_not_follow_directory_or_dotenv_symlinks() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join(".env"), "OUTSIDE_SECRET=never\n").unwrap();
    symlink(outside.path(), root.path().join("linked-dir")).unwrap();
    symlink(outside.path().join(".env"), root.path().join(".env.linked")).unwrap();
    let receipt = run_with_snapshot(config(root.path()), snapshot(&[])).unwrap();
    assert!(receipt.dotenv_files.is_empty());
    assert_eq!(receipt.coverage.skipped_symlinks, 2);
}

#[cfg(unix)]
#[test]
fn scanner_rejects_a_symlink_root() {
    use std::os::unix::fs::symlink;

    let real = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let linked = parent.path().join("linked");
    symlink(real.path(), &linked).unwrap();
    let error = run_with_snapshot(config(&linked), snapshot(&[])).unwrap_err();
    assert!(matches!(error, CensusError::SymlinkRoot(_)));
}
