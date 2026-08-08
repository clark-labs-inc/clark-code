#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use reqwest::Url;

use crate::service::ScoutAdapterService;
use crate::types::{
    AuthCandidateHandle, AuthCandidateSource, CensusRequest, CensusResponse, FetchPageResponse,
    SafeFailureCode, VerifyAuthRequest, VerifyAuthResponse, RUNTIME_PROTOCOL_VERSION,
};
use crate::vault::StoredAuthRef;

use super::fixtures::{config, environment, fake_cli, now_ms, request};

#[tokio::test]
async fn aws_cursor_survives_reload_without_serializing_raw_material() {
    let directory = tempfile::tempdir().unwrap();
    let executable = fake_cli(directory.path());
    let environment = environment([]);
    let api = Url::parse("http://127.0.0.1:9/").unwrap();
    let service = ScoutAdapterService::open(config(
        &directory.path().join("vault"),
        environment.clone(),
        None,
        Some(executable.clone()),
        None,
        api.clone(),
    ))
    .unwrap();
    let (target, candidate) = aws_profile_candidate(&service, "primary").await;
    let auth = verify_aws(&service, &target, candidate).await;
    let first = request(
        &target,
        &auth,
        "list_organization_accounts",
        "aws.organizations.account",
        "account",
        "global",
        &["id", "name", "state"],
    );
    let first_receipt = match service.fetch_page(first).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected first response: {other:?}"),
    };
    assert!(first_receipt.records.is_empty());
    assert!(first_receipt.next_cursor_handle.is_some());
    assert!(matches!(
        first_receipt.outcome,
        scout_adapter_protocol::AdapterPageOutcome::Succeeded { final_page: false }
    ));
    let public = serde_json::to_string(&first_receipt).unwrap();
    assert!(!public.contains("provider-cursor-canary-raw"));

    let vault_bytes = std::fs::read(service.vault().state_path_for_test()).unwrap();
    let vault_text = String::from_utf8(vault_bytes).unwrap();
    assert!(!vault_text.contains("provider-cursor-canary-raw"));
    assert!(!vault_text.contains("AWS_SECRET_ACCESS_KEY"));
    drop(service);

    let reloaded = ScoutAdapterService::open(config(
        &directory.path().join("vault"),
        environment,
        None,
        Some(executable.clone()),
        None,
        api,
    ))
    .unwrap();
    let mut second = first_receipt.request.clone();
    second.request_id = scout_adapter_protocol::RequestId::random();
    second.page_ordinal = 1;
    second.cursor_handle = first_receipt.next_cursor_handle.clone();
    second.requested_at_ms = now_ms();
    let second_receipt = match reloaded.fetch_page(second).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected second response: {other:?}"),
    };
    assert_eq!(second_receipt.records.len(), 1);
    assert!(second_receipt.next_cursor_handle.is_none());
    assert_eq!(second_receipt.records[0].identity_authority_scope, "global");

    let resources = request(
        &target,
        &auth,
        "list_resource_explorer_resources",
        "aws.resource_explorer.resource",
        "cloud_resource",
        "us-east-1",
        &["arn", "owning_account_id", "region", "resource_type"],
    );
    let resources = match reloaded.fetch_page(resources).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected resource response: {other:?}"),
    };
    assert_eq!(resources.records.len(), 1);
    assert_eq!(
        resources.records[0].identity_authority_scope,
        "123456789012"
    );
    let owner = resources.records[0].links.iter().next().unwrap();
    assert_eq!(owner.relationship_type, "owned_by");
    assert_eq!(owner.target_provider_type, "aws.organizations.account");
    assert_eq!(owner.target_authority_scope, "global");
    assert_eq!(owner.target_native_id, "aws-account:123456789012");

    let log = std::fs::read_to_string(format!("{}.log", executable.display())).unwrap();
    for mutation in [" create", " delete", " update", " put", " post", " mutate"] {
        assert!(
            !log.to_ascii_lowercase().contains(mutation),
            "mutation verb in {log}"
        );
    }
    assert!(log.contains("sts get-caller-identity"));
    assert!(log.contains("organizations list-accounts"));
    assert!(log.contains("resource-explorer-2 search"));

    let root_mode = std::fs::metadata(directory.path().join("vault"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let state_mode = std::fs::metadata(reloaded.vault().state_path_for_test())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(root_mode, 0o700);
    assert_eq!(state_mode, 0o600);
}

#[tokio::test]
async fn mislabeled_coverage_fails_before_provider_dispatch() {
    let directory = tempfile::tempdir().unwrap();
    let executable = fake_cli(directory.path());
    let service = ScoutAdapterService::open(config(
        &directory.path().join("vault"),
        environment([]),
        None,
        Some(executable.clone()),
        None,
        Url::parse("http://127.0.0.1:9/").unwrap(),
    ))
    .unwrap();
    let (target, candidate) = aws_profile_candidate(&service, "primary").await;
    let auth = verify_aws(&service, &target, candidate).await;
    let mut mislabeled = request(
        &target,
        &auth,
        "list_organization_accounts",
        "aws.organizations.account",
        "account",
        "global",
        &["id", "name", "state"],
    );
    mislabeled.coverage.resource_kind = "repository".to_owned();

    assert!(matches!(
        service.fetch_page(mislabeled).await,
        FetchPageResponse::Failed {
            failure: crate::SafeFailure {
                code: SafeFailureCode::InvalidRequest,
                ..
            }
        }
    ));
    let log = std::fs::read_to_string(format!("{}.log", executable.display())).unwrap();
    assert!(log.contains("sts get-caller-identity"));
    assert!(
        !log.contains("organizations list-accounts"),
        "mislabeled request reached provider dispatch: {log}"
    );
}

#[tokio::test]
async fn target_binding_and_safe_denied_stale_failures_are_enforced() {
    let directory = tempfile::tempdir().unwrap();
    let executable = fake_cli(directory.path());
    let service = ScoutAdapterService::open(config(
        &directory.path().join("vault"),
        environment([]),
        None,
        Some(executable),
        None,
        Url::parse("http://127.0.0.1:9/").unwrap(),
    ))
    .unwrap();
    let census = service.census(CensusRequest::default()).await;
    let (target, candidates) = match census {
        CensusResponse::Succeeded {
            target, candidates, ..
        } => (target, candidates),
        other => panic!("unexpected census: {other:?}"),
    };
    let denied = AuthCandidateHandle::for_target_ref(
        &target.target_id,
        &StoredAuthRef::AwsProfile {
            profile: "denied".to_owned(),
        }
        .stable_key(),
    );
    let stale = AuthCandidateHandle::for_target_ref(
        &target.target_id,
        &StoredAuthRef::AwsProfile {
            profile: "stale".to_owned(),
        }
        .stable_key(),
    );
    assert!(candidates
        .iter()
        .any(|candidate| candidate.handle == denied));
    assert!(candidates.iter().any(|candidate| candidate.handle == stale));

    let mut wrong_target = verify_request(&target, denied.clone());
    wrong_target.target_identity_sha256 = "a".repeat(64);
    assert!(matches!(
        service.verify_auth(wrong_target).await,
        VerifyAuthResponse::Failed {
            failure: crate::SafeFailure {
                code: SafeFailureCode::TargetMismatch,
                ..
            }
        }
    ));
    assert!(matches!(
        service.verify_auth(verify_request(&target, denied)).await,
        VerifyAuthResponse::Failed {
            failure: crate::SafeFailure {
                code: SafeFailureCode::AccessDenied,
                ..
            }
        }
    ));
    assert!(matches!(
        service.verify_auth(verify_request(&target, stale)).await,
        VerifyAuthResponse::Failed {
            failure: crate::SafeFailure {
                code: SafeFailureCode::AuthorizationStale,
                ..
            }
        }
    ));
}

async fn aws_profile_candidate(
    service: &ScoutAdapterService,
    profile: &str,
) -> (scout_adapter_protocol::TargetIdentity, AuthCandidateHandle) {
    let (target, candidates) = match service.census(CensusRequest::default()).await {
        CensusResponse::Succeeded {
            target, candidates, ..
        } => (*target, candidates),
        other => panic!("unexpected census: {other:?}"),
    };
    let expected = AuthCandidateHandle::for_target_ref(
        &target.target_id,
        &StoredAuthRef::AwsProfile {
            profile: profile.to_owned(),
        }
        .stable_key(),
    );
    let candidate = candidates
        .into_iter()
        .find(|candidate| {
            candidate.handle == expected && candidate.source == AuthCandidateSource::TargetProfile
        })
        .unwrap();
    (target, candidate.handle)
}

async fn verify_aws(
    service: &ScoutAdapterService,
    target: &scout_adapter_protocol::TargetIdentity,
    candidate: AuthCandidateHandle,
) -> scout_adapter_protocol::AuthContextDescriptor {
    match service
        .verify_auth(VerifyAuthRequest {
            runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
            target_id: target.target_id.clone(),
            target_identity_sha256: target.fingerprint_sha256().unwrap(),
            candidate_handle: candidate,
            adapter_id: crate::aws::adapter_id(),
            requested_authority_scope: Some("123456789012".to_owned()),
        })
        .await
    {
        VerifyAuthResponse::Succeeded { auth_context, .. } => *auth_context,
        other => panic!("unexpected verify response: {other:?}"),
    }
}

fn verify_request(
    target: &scout_adapter_protocol::TargetIdentity,
    candidate_handle: AuthCandidateHandle,
) -> VerifyAuthRequest {
    VerifyAuthRequest {
        runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
        target_id: target.target_id.clone(),
        target_identity_sha256: target.fingerprint_sha256().unwrap(),
        candidate_handle,
        adapter_id: crate::aws::adapter_id(),
        requested_authority_scope: Some("123456789012".to_owned()),
    }
}
