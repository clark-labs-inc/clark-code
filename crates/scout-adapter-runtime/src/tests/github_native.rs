use std::sync::Arc;

use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::service::ScoutAdapterService;
use crate::types::{
    AuthCandidateSource, CensusRequest, CensusResponse, FetchPageResponse, VerifyAuthRequest,
    VerifyAuthResponse, RUNTIME_PROTOCOL_VERSION,
};

use super::fixtures::{config, environment, now_ms, request};

#[tokio::test]
async fn native_github_uses_only_target_token_and_paginates() {
    let (base, captured, server) = github_server(4).await;
    let directory = tempfile::tempdir().unwrap();
    let service = ScoutAdapterService::open(config(
        directory.path(),
        environment([("GH_TOKEN", "target-token-canary")]),
        None,
        None,
        None,
        base,
    ))
    .unwrap();
    let (target, candidate) = match service.census(CensusRequest::default()).await {
        CensusResponse::Succeeded {
            target, candidates, ..
        } => {
            let candidate = candidates
                .into_iter()
                .find(|candidate| candidate.source == AuthCandidateSource::TargetEnvironment)
                .unwrap();
            (*target, candidate)
        }
        other => panic!("unexpected census: {other:?}"),
    };
    let verify = service
        .verify_auth(VerifyAuthRequest {
            runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
            target_id: target.target_id.clone(),
            target_identity_sha256: target.fingerprint_sha256().unwrap(),
            candidate_handle: candidate.handle,
            adapter_id: crate::github::adapter_id(),
            requested_authority_scope: Some("acme".to_owned()),
        })
        .await;
    let auth = match verify {
        VerifyAuthResponse::Succeeded { auth_context, .. } => *auth_context,
        other => {
            let requests = captured.lock().await;
            panic!("unexpected verify: {other:?}; requests={requests:?}")
        }
    };
    let first = request(
        &target,
        &auth,
        "list_repositories",
        "github.repository",
        "repository",
        "global",
        &["name", "private"],
    );
    let first = match service.fetch_page(first).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected first page: {other:?}"),
    };
    assert_eq!(first.records.len(), 1);
    assert!(first.next_cursor_handle.is_some());
    let mut second_request = first.request.clone();
    second_request.request_id = scout_adapter_protocol::RequestId::random();
    second_request.page_ordinal = 1;
    second_request.cursor_handle = first.next_cursor_handle.clone();
    second_request.requested_at_ms = now_ms();
    let second = match service.fetch_page(second_request).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected second page: {other:?}"),
    };
    assert!(second.records.is_empty());
    assert!(second.next_cursor_handle.is_none());
    server.await.unwrap();

    let requests = captured.lock().await;
    assert_eq!(requests.len(), 4);
    for request in requests.iter() {
        assert!(request.contains("authorization: Bearer target-token-canary"));
        assert!(!request.contains("desktop-decoy-token"));
    }
    assert!(requests
        .iter()
        .any(|request| request.contains("GET /user ")));
    assert!(requests
        .iter()
        .any(|request| request.contains("GET /orgs/acme ")));
    assert!(requests
        .iter()
        .any(|request| request.contains("/orgs/acme/repos?per_page=100&page=1")));
    assert!(requests
        .iter()
        .any(|request| request.contains("/orgs/acme/repos?per_page=100&page=2")));
}

#[tokio::test]
async fn native_github_enumerates_authenticated_organizations_before_repositories() {
    let (base, captured, server) = github_server(2).await;
    let directory = tempfile::tempdir().unwrap();
    let service = ScoutAdapterService::open(config(
        directory.path(),
        environment([("GH_TOKEN", "target-token-canary")]),
        None,
        None,
        None,
        base,
    ))
    .unwrap();
    let (target, candidate) = match service.census(CensusRequest::default()).await {
        CensusResponse::Succeeded {
            target, candidates, ..
        } => {
            let candidate = candidates
                .into_iter()
                .find(|candidate| candidate.source == AuthCandidateSource::TargetEnvironment)
                .unwrap();
            (*target, candidate)
        }
        other => panic!("unexpected census: {other:?}"),
    };
    let auth = match service
        .verify_auth(VerifyAuthRequest {
            runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
            target_id: target.target_id.clone(),
            target_identity_sha256: target.fingerprint_sha256().unwrap(),
            candidate_handle: candidate.handle,
            adapter_id: crate::github::adapter_id(),
            requested_authority_scope: Some("global".into()),
        })
        .await
    {
        VerifyAuthResponse::Succeeded { auth_context, .. } => *auth_context,
        other => panic!("unexpected verify: {other:?}"),
    };
    let page = request(
        &target,
        &auth,
        "list_organizations",
        "github.organization",
        "organization",
        "global",
        &["login"],
    );
    let receipt = match service.fetch_page(page).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected page: {other:?}"),
    };
    assert_eq!(receipt.records.len(), 1);
    assert_eq!(receipt.records[0].native_id, "github-organization:7");
    assert_eq!(
        receipt.records[0].fields.get("login"),
        Some(&scout_adapter_protocol::SafeFieldValue::Text("acme".into()))
    );
    server.await.unwrap();
    let requests = captured.lock().await;
    assert!(requests
        .iter()
        .any(|request| request.contains("GET /user ")));
    assert!(requests
        .iter()
        .any(|request| request.contains("GET /user/orgs?per_page=100&page=1")));
    assert!(!requests
        .iter()
        .any(|request| request.contains("GET /orgs/acme ")));
}

#[tokio::test]
async fn native_github_enumerates_every_repository_visible_to_the_authenticated_user() {
    let (base, captured, server) = github_server(2).await;
    let directory = tempfile::tempdir().unwrap();
    let service = ScoutAdapterService::open(config(
        directory.path(),
        environment([("GH_TOKEN", "target-token-canary")]),
        None,
        None,
        None,
        base,
    ))
    .unwrap();
    let (target, candidate) = match service.census(CensusRequest::default()).await {
        CensusResponse::Succeeded {
            target, candidates, ..
        } => {
            let candidate = candidates
                .into_iter()
                .find(|candidate| candidate.source == AuthCandidateSource::TargetEnvironment)
                .unwrap();
            (*target, candidate)
        }
        other => panic!("unexpected census: {other:?}"),
    };
    let auth = match service
        .verify_auth(VerifyAuthRequest {
            runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
            target_id: target.target_id.clone(),
            target_identity_sha256: target.fingerprint_sha256().unwrap(),
            candidate_handle: candidate.handle,
            adapter_id: crate::github::adapter_id(),
            requested_authority_scope: Some("global".into()),
        })
        .await
    {
        VerifyAuthResponse::Succeeded { auth_context, .. } => *auth_context,
        other => panic!("unexpected verify: {other:?}"),
    };
    let page = request(
        &target,
        &auth,
        "list_accessible_repositories",
        "github.repository",
        "repository",
        "global",
        &["full_name", "owner_login"],
    );
    let receipt = match service.fetch_page(page).await {
        FetchPageResponse::Succeeded { receipt } => receipt,
        other => panic!("unexpected page: {other:?}"),
    };
    assert_eq!(receipt.records.len(), 1);
    assert_eq!(receipt.records[0].native_id, "github-repository:9");
    assert!(receipt.records[0].links.iter().any(|link| {
        link.relationship_type == "canonical_remote"
            && link.target_provider_type == "git.repository"
            && link.target_native_id == "github.com/acme/clark"
    }));
    server.await.unwrap();
    let requests = captured.lock().await;
    assert!(requests.iter().any(|request| {
        request.contains("GET /user/repos?")
            && request.contains("affiliation=owner%2Ccollaborator%2Corganization_member")
            && !request.contains("visibility=")
    }));
}

#[test]
fn verify_request_rejects_a_model_supplied_token_field() {
    let value = serde_json::json!({
        "runtime_protocol_version": 1,
        "target_id": format!("target:{}", "1".repeat(64)),
        "target_identity_sha256": "2".repeat(64),
        "candidate_handle": format!("candidate:{}", "3".repeat(64)),
        "adapter_id": "clark/github-organization@1",
        "requested_authority_scope": "acme",
        "token": "desktop-decoy-token"
    });
    assert!(serde_json::from_value::<VerifyAuthRequest>(value).is_err());
}

async fn github_server(
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
            let (body, link) = if first_line.contains("GET /user ") {
                (r#"{"id":42,"login":"scout"}"#, None)
            } else if first_line.contains("GET /orgs/acme ") {
                (r#"{"id":7,"login":"acme"}"#, None)
            } else if first_line.contains("GET /user/orgs?") {
                (r#"[{"id":7,"login":"acme"}]"#, None)
            } else if first_line.contains("&page=1") {
                (
                    r#"[{"id":9,"name":"clark","full_name":"acme/clark","private":true,"archived":false,"disabled":false,"fork":false,"default_branch":"main","visibility":"private","html_url":"https://github.com/acme/clark","owner":{"login":"acme"}}]"#,
                    Some(
                        "Link: <http://example.invalid/page=2>; rel=\"next\"; type=\"application/json\"\r\n",
                    ),
                )
            } else {
                ("[]", None)
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                link.unwrap_or_default(),
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (
        Url::parse(&format!("http://{address}/")).unwrap(),
        captured,
        server,
    )
}
