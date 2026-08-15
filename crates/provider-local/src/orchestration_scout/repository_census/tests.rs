use super::*;
use crate::exec::LocalExecutor;

#[tokio::test]
async fn census_reconciles_transport_equivalent_remotes_without_leaking_paths() {
    let directory = tempfile::tempdir().unwrap();
    let status = tokio::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(directory.path())
        .status()
        .await
        .unwrap();
    assert!(status.success());
    std::fs::write(
        directory.path().join("package.json"),
        r#"{
            "name": "enterprise-api",
            "description": "Customer identity API",
            "scripts": {"test": "secret command body is not returned"},
            "dependencies": {"postgres": "1.0.0"}
        }"#,
    )
    .unwrap();
    let status = tokio::process::Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "git@github.com:Example/Enterprise.git",
        ])
        .current_dir(directory.path())
        .status()
        .await
        .unwrap();
    assert!(status.success());

    let (census, bindings) = census_roots(&LocalExecutor, &[directory.path().to_path_buf()])
        .await
        .unwrap();
    assert_eq!(census.checkout_count, 1);
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        census.repositories[0].canonical_remote.as_deref(),
        Some("github.com/example/enterprise")
    );
    let encoded = serde_json::to_string(&census).unwrap();
    assert!(!encoded.contains(directory.path().to_string_lossy().as_ref()));
    assert_eq!(census.gaps, ["unapproved_filesystem_locations_not_scanned"]);

    let checkout_id = census.repositories[0].checkout_id.clone();
    let outcome = census_outcome(census);
    assert!(outcome.content.contains(&checkout_id));
    assert!(!outcome
        .content
        .contains(directory.path().to_string_lossy().as_ref()));
    let inspection = inspect_checkout(
        &LocalExecutor,
        &checkout_id,
        bindings.get(&checkout_id).unwrap(),
    )
    .await
    .unwrap();
    assert!(inspection.package_names.contains("enterprise-api"));
    assert!(inspection.descriptions.contains("Customer identity API"));
    assert!(inspection.dependency_names.contains("postgres"));
    assert!(inspection.command_names.contains("test"));
    let encoded = serde_json::to_string(&inspection).unwrap();
    assert!(!encoded.contains("secret command body"));
    assert!(!encoded.contains(directory.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn backend_task_collects_local_checkout_as_a_bound_adapter_receipt() {
    let directory = tempfile::tempdir().unwrap();
    assert!(tokio::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(directory.path())
        .status()
        .await
        .unwrap()
        .success());
    assert!(tokio::process::Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/Example/Enterprise.git",
        ])
        .current_dir(directory.path())
        .status()
        .await
        .unwrap()
        .success());
    std::fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname = \"enterprise-api\"\ndescription = \"Customer API\"\n",
    )
    .unwrap();
    let (census, bindings) = census_roots(&LocalExecutor, &[directory.path().to_path_buf()])
        .await
        .unwrap();
    let scope: AdapterPageTaskScope = serde_json::from_value(json!({
        "schema_version": 1,
        "first_source_sequence": 1,
        "adapter_id": LOCAL_REPOSITORY_ADAPTER_ID,
        "enterprise_id": "organization:test",
        "charter_id": "charter:test",
        "discovery_epoch": 1,
        "coverage_sequence": 1,
        "region_or_project": "host_approved_read_roots",
        "resource_kind": "repository_checkout",
        "query": {
            "operation": "list_host_approved_checkouts",
            "authority_scope": "host_approved_read_roots",
            "provider_resource_type": "local.repository_checkout",
            "filters": {},
            "projection": [
                "display_name",
                "repository_fingerprint",
                "canonical_remote",
                "package_names",
                "descriptions"
            ],
            "page_size": 1000
        },
        "page_ordinal": 0,
        "cursor_handle": null,
        "limits": {
            "max_records": 1000,
            "max_response_bytes": 8_000_000,
            "max_duration_ms": 30_000
        }
    }))
    .unwrap();
    let digest = |byte: u8| format!("{byte:02x}").repeat(32);
    let target = scout_adapter_protocol::TargetIdentity::new(
        digest(1),
        digest(2),
        digest(3),
        digest(4),
        "linux".into(),
        "x86_64".into(),
    )
    .unwrap();

    let receipt =
        local_repository_receipt(&LocalExecutor, &scope, target, &census, &bindings, 1_000)
            .await
            .unwrap();

    assert_eq!(receipt.records.len(), 1);
    assert_eq!(
        receipt.records[0].fields["canonical_remote"],
        SafeFieldValue::Text("github.com/example/enterprise".into())
    );
    assert!(receipt.records[0]
        .links
        .iter()
        .any(|link| link.target_native_id == "github.com/example/enterprise"));
    let encoded = serde_json::to_string(&receipt).unwrap();
    assert!(!encoded.contains(directory.path().to_string_lossy().as_ref()));
    assert!(encoded.contains("enterprise-api"));
}
