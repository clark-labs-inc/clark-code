use super::*;

fn base_request(root_name: &str) -> SecurityPocRunRequest {
    SecurityPocRunRequest {
        scan_id: "scan-1".into(),
        candidate_id: "cand-1".into(),
        inventory_id: "inv-1".into(),
        control: PocControl::Positive,
        language: PocLanguage::Shell,
        expected_observation: "marker printed".into(),
        script: "printf poc-ok".into(),
        expected_exit_code: 0,
        timeout_seconds: 10,
        run_root: format!(".agent/security-scans/scan-1/poc/runs/cand-1-positive-{root_name}"),
        inventory: vec![PocInventoryFile {
            path: "src/app.txt".into(),
            bytes: b"hello poc\n".to_vec(),
        }],
    }
}

#[tokio::test]
#[ignore = "requires native OS containment; exercised by the Sandbox Conformance lane"]
async fn run_seals_a_managed_disposable_receipt_with_matching_digests() {
    let dir = tempfile::tempdir().unwrap();
    let outcome = run(dir.path(), &base_request("a1")).await.unwrap();
    let receipt = &outcome.receipt;

    assert_eq!(receipt.containment, "managed_disposable");
    assert_eq!(receipt.contract_version, 2);
    assert!(receipt.receipt_id.starts_with("poc-"));
    assert!(receipt.passed);
    assert_eq!(receipt.exit_code, Some(0));
    assert_eq!(receipt.stdout_sha256, sha256_hex(&outcome.stdout));
    assert_eq!(receipt.stderr_sha256, sha256_hex(&outcome.stderr));
    assert_eq!(receipt.script_sha256, sha256_hex(b"printf poc-ok"));
    // The staged inventory digest is recomputed by the runner, not trusted.
    assert!(!receipt.workspace_sha256.is_empty());
    assert_eq!(receipt.scan_id, "scan-1");
    assert_eq!(receipt.candidate_id, "cand-1");
    assert_eq!(receipt.inventory_id, "inv-1");
    // Receipt is persisted on the target under the run root.
    let artifact = dir.path().join(&receipt.artifact_path);
    assert!(
        artifact.is_file(),
        "receipt.json should exist at {artifact:?}"
    );
    // The disposable workspace staged the inventory file.
    let staged = dir
        .path()
        .join(&receipt.artifact_path)
        .parent()
        .unwrap()
        .join("workspace/src/app.txt");
    assert_eq!(std::fs::read(&staged).unwrap(), b"hello poc\n");
}

#[tokio::test]
async fn run_fails_closed_on_unsafe_inventory_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut request = base_request("b1");
    request.inventory.push(PocInventoryFile {
        path: "../escape.txt".into(),
        bytes: b"x".to_vec(),
    });
    let error = run(dir.path(), &request).await.unwrap_err();
    assert!(error.contains("unsafe path"), "unexpected error: {error}");
}

#[tokio::test]
async fn run_rejects_an_escaping_run_root() {
    let dir = tempfile::tempdir().unwrap();
    let mut request = base_request("c1");
    request.run_root = "../outside".into();
    let error = run(dir.path(), &request).await.unwrap_err();
    assert!(
        error.contains("not a safe relative path"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn run_rejects_bad_ids_and_oversized_script() {
    let dir = tempfile::tempdir().unwrap();
    let mut request = base_request("d1");
    request.scan_id = "bad id!".into();
    assert!(run(dir.path(), &request)
        .await
        .unwrap_err()
        .contains("scan_id"));

    let mut request = base_request("d2");
    request.script = "x".repeat(256 * 1024 + 1);
    assert!(run(dir.path(), &request)
        .await
        .unwrap_err()
        .contains("script"));
}

#[tokio::test]
#[ignore = "requires native OS containment; exercised by the Sandbox Conformance lane"]
async fn dispatch_round_trips_through_the_service_name() {
    let dir = tempfile::tempdir().unwrap();
    let request = serde_json::to_vec(&base_request("e1")).unwrap();
    let response = dispatch(SERVICE_NAME, dir.path(), &request).await.unwrap();
    let response: SecurityPocRunResponse = serde_json::from_slice(&response).unwrap();
    assert_eq!(response.receipt.containment, "managed_disposable");
    assert!(response.receipt.passed);

    let error = dispatch("not-the-service", dir.path(), &request)
        .await
        .unwrap_err();
    assert!(error.contains("unsupported target service"));
}
