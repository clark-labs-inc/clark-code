use std::sync::Arc;

use reqwest::Url;
use scout_adapter_protocol::{AdapterPageOutcome, FailureReason, NormalizedRecord};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::service::ScoutAdapterService;
use crate::types::{
    AuthCandidate, CensusRequest, CensusResponse, FetchPageResponse, SafeFailureCode, ToolKind,
    VerifyAuthRequest, VerifyAuthResponse, RUNTIME_PROTOCOL_VERSION,
};

use super::fixtures::{config, environment, now_ms, request};

#[tokio::test]
async fn gitlab_group_projects_paginate_deterministically_with_opaque_target_handles() {
    let (base, captured, server) = gitlab_server(GitlabFixture::Success, 8).await;
    let first = collect_success_fixture(base.clone()).await;
    let second = collect_success_fixture(base).await;
    server.await.unwrap();
    assert_eq!(first, second);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].provider_namespace, "gitlab");
    assert_eq!(first[0].provider_type, "gitlab.project");
    assert!(first[0]
        .identity_authority_scope
        .starts_with("http://127.0.0.1:"));
    assert_eq!(first[0].native_id, "gitlab-project:101");
    let owner = first[0].links.iter().next().unwrap();
    assert_eq!(owner.relationship_type, "owned_by");
    assert_eq!(owner.target_provider_type, "gitlab.group");
    assert_eq!(owner.target_native_id, "gitlab-group:77");
    assert_eq!(
        owner.target_authority_scope,
        first[0].identity_authority_scope
    );

    let requests = captured.lock().await;
    assert_eq!(requests.len(), 8);
    assert!(requests
        .iter()
        .all(|request| request.contains("private-token: target-gitlab-token-canary")));
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("GET /api/v4/user "))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("/api/v4/groups/acme%2Fplatform "))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| {
                request.contains("/api/v4/groups/acme%2Fplatform/projects?")
                    && request.contains("include_subgroups=true")
                    && request.contains("with_shared=false")
                    && request.contains("order_by=id")
                    && request.contains("sort=asc")
                    && request.contains("&page=1 ")
            })
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("&page=7 "))
            .count(),
        2
    );
}

#[tokio::test]
async fn gitlab_denial_becomes_a_terminal_typed_coverage_gap_without_secret_echo() {
    let (base, captured, server) = gitlab_server(GitlabFixture::Denied, 3).await;
    let directory = tempfile::tempdir().unwrap();
    let service = ScoutAdapterService::open(
        config(
            directory.path(),
            environment([("GITLAB_TOKEN", "target-gitlab-token-canary")]),
            None,
            None,
            None,
            Url::parse("http://127.0.0.1:9/").unwrap(),
        )
        .with_gitlab_api_base(base),
    )
    .unwrap();
    let (target, _candidate, auth) = verify_group(&service).await;
    let page = request(
        &target,
        &auth,
        "list_group_projects",
        "gitlab.project",
        "repository",
        "global",
        &["name", "path_with_namespace"],
    );
    let receipt = match service.fetch_page(page).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("expected an evidence-bearing denial receipt: {other:?}"),
    };
    assert_eq!(
        receipt.outcome,
        AdapterPageOutcome::Denied {
            reason: FailureReason::AccessDenied
        }
    );
    assert!(receipt.records.is_empty());
    assert!(receipt.next_cursor_handle.is_none());
    let public = serde_json::to_string(&receipt).unwrap();
    assert!(!public.contains("target-gitlab-token-canary"));
    let vault = std::fs::read_to_string(service.vault().state_path_for_test()).unwrap();
    assert!(!vault.contains("target-gitlab-token-canary"));
    server.await.unwrap();
    assert_eq!(captured.lock().await.len(), 3);
}

#[tokio::test]
async fn gitlab_candidates_cannot_cross_execution_targets() {
    let first_directory = tempfile::tempdir().unwrap();
    let second_directory = tempfile::tempdir().unwrap();
    let first = ScoutAdapterService::open(config(
        first_directory.path(),
        environment([("GITLAB_TOKEN", "first-target-token")]),
        None,
        None,
        None,
        Url::parse("http://127.0.0.1:9/").unwrap(),
    ))
    .unwrap();
    let second = ScoutAdapterService::open(config(
        second_directory.path(),
        environment([("GITLAB_TOKEN", "second-target-token")]),
        None,
        None,
        None,
        Url::parse("http://127.0.0.1:9/").unwrap(),
    ))
    .unwrap();
    let (first_target, first_candidate) = gitlab_candidate(&first).await;
    let second_target = match second.census(CensusRequest::default()).await {
        CensusResponse::Succeeded { target, .. } => *target,
        other => panic!("unexpected census: {other:?}"),
    };
    assert_ne!(first_target.target_id, second_target.target_id);
    let first_target_sha256 = first_target.fingerprint_sha256().unwrap();
    let response = second
        .verify_auth(VerifyAuthRequest {
            runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
            target_id: first_target.target_id,
            target_identity_sha256: first_target_sha256,
            candidate_handle: first_candidate.handle,
            adapter_id: crate::gitlab::adapter_id(),
            requested_authority_scope: Some("acme/platform".to_owned()),
        })
        .await;
    assert!(matches!(
        response,
        VerifyAuthResponse::Failed {
            failure: crate::SafeFailure {
                code: SafeFailureCode::TargetMismatch,
                retryable: false
            }
        }
    ));
}

async fn collect_success_fixture(base: Url) -> Vec<NormalizedRecord> {
    let directory = tempfile::tempdir().unwrap();
    let service = ScoutAdapterService::open(
        config(
            directory.path(),
            environment([("GITLAB_TOKEN", "target-gitlab-token-canary")]),
            None,
            None,
            None,
            Url::parse("http://127.0.0.1:9/").unwrap(),
        )
        .with_gitlab_api_base(base),
    )
    .unwrap();
    let (target, _, auth) = verify_group(&service).await;
    let first_request = request(
        &target,
        &auth,
        "list_group_projects",
        "gitlab.project",
        "repository",
        "global",
        &[
            "name",
            "path",
            "path_with_namespace",
            "visibility",
            "archived",
            "default_branch",
            "web_url",
            "namespace_full_path",
            "topics",
        ],
    );
    let first = match service.fetch_page(first_request).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected first page: {other:?}"),
    };
    assert!(first.next_cursor_handle.is_some());
    assert!(!serde_json::to_string(&first)
        .unwrap()
        .contains("target-gitlab-token-canary"));
    let mut second_request = first.request.clone();
    second_request.request_id = scout_adapter_protocol::RequestId::random();
    second_request.page_ordinal = 1;
    second_request.cursor_handle = first.next_cursor_handle.clone();
    second_request.requested_at_ms = now_ms();
    let second = match service.fetch_page(second_request).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected second page: {other:?}"),
    };
    assert!(second.next_cursor_handle.is_none());
    let mut records = first.records.clone();
    records.extend(second.records.clone());
    records
}

async fn verify_group(
    service: &ScoutAdapterService,
) -> (
    scout_adapter_protocol::TargetIdentity,
    AuthCandidate,
    scout_adapter_protocol::AuthContextDescriptor,
) {
    let (target, candidate) = gitlab_candidate(service).await;
    let response = service
        .verify_auth(VerifyAuthRequest {
            runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
            target_id: target.target_id.clone(),
            target_identity_sha256: target.fingerprint_sha256().unwrap(),
            candidate_handle: candidate.handle.clone(),
            adapter_id: crate::gitlab::adapter_id(),
            requested_authority_scope: Some("acme/platform".to_owned()),
        })
        .await;
    let auth = match response {
        VerifyAuthResponse::Succeeded { auth_context, .. } => *auth_context,
        other => panic!("unexpected GitLab authorization: {other:?}"),
    };
    (target, candidate, auth)
}

async fn gitlab_candidate(
    service: &ScoutAdapterService,
) -> (scout_adapter_protocol::TargetIdentity, AuthCandidate) {
    match service.census(CensusRequest::default()).await {
        CensusResponse::Succeeded {
            target,
            candidates,
            tools,
            coverage_manifest,
            ..
        } => {
            assert!(tools.iter().any(|tool| {
                tool.tool == ToolKind::NativeGitlabHttps
                    && tool.available
                    && tool.census_failure.is_none()
            }));
            assert!(coverage_manifest
                .routes
                .iter()
                .any(|route| route.adapter_id == "clark/gitlab-group@1"));
            let candidate = candidates
                .into_iter()
                .find(|candidate| candidate.provider == "gitlab")
                .unwrap();
            (*target, candidate)
        }
        other => panic!("unexpected census: {other:?}"),
    }
}

#[derive(Clone, Copy)]
enum GitlabFixture {
    Success,
    Denied,
}

async fn gitlab_server(
    fixture: GitlabFixture,
    requests: usize,
) -> (Url, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let server_captured = Arc::clone(&captured);
    let server = tokio::spawn(async move {
        for _ in 0..requests {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0_u8; 2_048];
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            let first_line = request.lines().next().unwrap().to_owned();
            server_captured.lock().await.push(request);
            let (status, body, next_page) = if first_line.contains("/user ") {
                ("200 OK", r#"{"id":42,"username":"scout"}"#, None)
            } else if !first_line.contains("/projects?") {
                ("200 OK", r#"{"id":70,"full_path":"acme/platform"}"#, None)
            } else if matches!(fixture, GitlabFixture::Denied) {
                ("403 Forbidden", r#"{"message":"forbidden-canary"}"#, None)
            } else if first_line.contains("&page=1 ") {
                (
                    "200 OK",
                    r#"[{"id":101,"name":"Clark","path":"clark","path_with_namespace":"acme/platform/clark","visibility":"private","archived":false,"default_branch":"main","web_url":"https://gitlab.example/acme/platform/clark","namespace":{"id":77,"full_path":"acme/platform"},"topics":["agents","rust"]}]"#,
                    Some("7"),
                )
            } else {
                (
                    "200 OK",
                    r#"[{"id":202,"name":"Control","path":"control","path_with_namespace":"acme/platform/control","visibility":"internal","archived":true,"default_branch":null,"web_url":"https://gitlab.example/acme/platform/control","namespace":{"id":77,"full_path":"acme/platform"},"topics":[]}]"#,
                    None,
                )
            };
            let next_header = next_page
                .map(|page| format!("X-Next-Page: {page}\r\n"))
                .unwrap_or_default();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{next_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (
        Url::parse(&format!("http://{address}/api/v4/")).unwrap(),
        captured,
        server,
    )
}
