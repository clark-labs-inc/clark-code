#![cfg(unix)]

use reqwest::Url;
use scout_adapter_protocol::AdapterPageOutcome;

use crate::service::ScoutAdapterService;
use crate::types::{
    CensusRequest, CensusResponse, FetchPageResponse, SafeFailureCode, ToolKind, VerifyAuthRequest,
    VerifyAuthResponse, RUNTIME_PROTOCOL_VERSION,
};

use super::fixtures::{config, environment, fake_gcloud, now_ms, request};

#[tokio::test]
async fn gcloud_identity_org_project_and_asset_slice_is_bounded_and_read_only() {
    let directory = tempfile::tempdir().unwrap();
    let executable = fake_gcloud(directory.path());
    let service = ScoutAdapterService::open(config(
        &directory.path().join("vault"),
        environment([]),
        None,
        None,
        Some(executable.clone()),
        Url::parse("http://127.0.0.1:9/").unwrap(),
    ))
    .unwrap();
    let (target, candidate) = match service.census(CensusRequest::default()).await {
        CensusResponse::Succeeded {
            target,
            candidates,
            tools,
            ..
        } => {
            assert!(tools.iter().any(|tool| {
                tool.tool == ToolKind::GcloudCli && tool.available && tool.census_failure.is_none()
            }));
            let candidate = candidates
                .into_iter()
                .find(|candidate| candidate.provider == "gcp")
                .unwrap();
            (*target, candidate)
        }
        other => panic!("unexpected census: {other:?}"),
    };
    let global_auth = verify(&service, &target, candidate.handle.clone(), "global").await;
    let mut organizations = request(
        &target,
        &global_auth,
        "list_organizations",
        "gcp.organization",
        "organization",
        "global",
        &["name", "display_name", "state"],
    );
    organizations.query.page_size = 1;
    let first = receipt(service.fetch_page(organizations).await);
    assert_eq!(first.records.len(), 1);
    assert!(first.next_cursor_handle.is_some());
    let mut second_request = first.request.clone();
    second_request.request_id = scout_adapter_protocol::RequestId::random();
    second_request.page_ordinal = 1;
    second_request.cursor_handle = first.next_cursor_handle.clone();
    second_request.requested_at_ms = now_ms();
    let second = receipt(service.fetch_page(second_request).await);
    assert_eq!(second.records.len(), 1);
    assert!(second.next_cursor_handle.is_none());

    let vault = std::fs::read_to_string(service.vault().state_path_for_test()).unwrap();
    assert!(!vault.contains("organizations/1"));
    assert!(!vault.contains("provider-secret-canary"));

    let org_auth = verify(
        &service,
        &target,
        candidate.handle.clone(),
        "organizations/123",
    )
    .await;
    let folders = request(
        &target,
        &org_auth,
        "list_folders",
        "gcp.folder",
        "folder",
        "global",
        &["name", "display_name", "state", "parent"],
    );
    let folders = receipt(service.fetch_page(folders).await);
    assert_eq!(folders.records.len(), 1);
    assert_eq!(folders.records[0].native_id, "folders/7");
    let folder_parent = folders.records[0].links.iter().next().unwrap();
    assert_eq!(folder_parent.relationship_type, "member_of");
    assert_eq!(folder_parent.target_provider_type, "gcp.organization");
    assert_eq!(folder_parent.target_native_id, "organizations/123");

    let projects = request(
        &target,
        &org_auth,
        "list_projects",
        "gcp.project",
        "project",
        "global",
        &["project_id", "project_number", "lifecycle_state"],
    );
    let projects = receipt(service.fetch_page(projects).await);
    assert_eq!(projects.records.len(), 1);
    assert_eq!(projects.records[0].native_id, "projects/42");
    assert_eq!(projects.records[0].identity_authority_scope, "global");
    let project_parent = projects.records[0].links.iter().next().unwrap();
    assert_eq!(project_parent.relationship_type, "member_of");
    assert_eq!(project_parent.target_provider_type, "gcp.organization");
    assert_eq!(project_parent.target_native_id, "organizations/123");
    let folder_auth = verify(&service, &target, candidate.handle.clone(), "folders/7").await;
    let folder_projects = request(
        &target,
        &folder_auth,
        "list_projects",
        "gcp.project",
        "project",
        "global",
        &["project_id", "project_number", "parent_type", "parent_id"],
    );
    let folder_projects = receipt(service.fetch_page(folder_projects).await);
    assert_eq!(folder_projects.records[0].native_id, "projects/43");
    let folder_project_parent = folder_projects.records[0].links.iter().next().unwrap();
    assert_eq!(folder_project_parent.target_provider_type, "gcp.folder");
    assert_eq!(folder_project_parent.target_native_id, "folders/7");
    let assets = request(
        &target,
        &org_auth,
        "search_all_resources",
        "gcp.cloud_asset.resource",
        "cloud_resource",
        "global",
        &["name", "asset_type", "project", "location", "state"],
    );
    let assets = receipt(service.fetch_page(assets).await);
    assert_eq!(assets.records.len(), 1);
    assert_eq!(assets.records[0].identity_authority_scope, "global");
    let asset_owner = assets.records[0].links.iter().next().unwrap();
    assert_eq!(asset_owner.relationship_type, "owned_by");
    assert_eq!(asset_owner.target_provider_type, "gcp.project");
    assert_eq!(asset_owner.target_native_id, "projects/42");

    let denied = service
        .verify_auth(verify_request(
            &target,
            candidate.handle,
            "organizations/999",
        ))
        .await;
    assert!(matches!(
        denied,
        VerifyAuthResponse::Failed {
            failure: crate::SafeFailure {
                code: SafeFailureCode::AccessDenied,
                ..
            }
        }
    ));
    assert!(!serde_json::to_string(&denied)
        .unwrap()
        .contains("provider-secret-canary"));

    let log = std::fs::read_to_string(format!("{}.log", executable.display())).unwrap();
    for mutation in [
        " login", " create", " delete", " update", " set-", " add-", " remove",
    ] {
        assert!(!log.to_ascii_lowercase().contains(mutation), "{log}");
    }
    assert!(log.contains("auth list"));
    assert!(log.contains("organizations list"));
    assert!(log.contains("resource-manager folders list"));
    assert!(log.contains("projects list"));
    assert!(log.contains("asset search-all-resources"));
}

async fn verify(
    service: &ScoutAdapterService,
    target: &scout_adapter_protocol::TargetIdentity,
    candidate: crate::AuthCandidateHandle,
    scope: &str,
) -> scout_adapter_protocol::AuthContextDescriptor {
    match service
        .verify_auth(verify_request(target, candidate, scope))
        .await
    {
        VerifyAuthResponse::Succeeded { auth_context, .. } => *auth_context,
        other => panic!("unexpected verify: {other:?}"),
    }
}

fn verify_request(
    target: &scout_adapter_protocol::TargetIdentity,
    candidate_handle: crate::AuthCandidateHandle,
    scope: &str,
) -> VerifyAuthRequest {
    VerifyAuthRequest {
        runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
        target_id: target.target_id.clone(),
        target_identity_sha256: target.fingerprint_sha256().unwrap(),
        candidate_handle,
        adapter_id: crate::gcp::adapter_id(),
        requested_authority_scope: Some(scope.to_owned()),
    }
}

fn receipt(response: FetchPageResponse) -> Box<scout_adapter_protocol::AdapterPageReceipt> {
    match response {
        FetchPageResponse::Succeeded { receipt } => {
            assert!(matches!(
                receipt.outcome,
                AdapterPageOutcome::Succeeded { .. }
            ));
            receipt
        }
        other => panic!("unexpected fetch: {other:?}"),
    }
}
