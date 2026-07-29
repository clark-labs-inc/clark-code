use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

#[derive(Clone)]
struct ResponseSpec {
    status: &'static str,
    body: String,
}

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: String,
    body: Vec<u8>,
}

async fn server(
    build: impl FnOnce(&str) -> Vec<ResponseSpec>,
) -> (
    String,
    Arc<Mutex<Vec<CapturedRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let responses = build(&base);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let handle = tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            captured.lock().unwrap().push(request);
            let wire = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status,
                response.body.len(),
                response.body
            );
            stream.write_all(wire.as_bytes()).await.unwrap();
        }
    });
    (base, requests, handle)
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> CapturedRequest {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4_096];
    let (header_end, content_length) = loop {
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = find_header_end(&bytes) {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            break (header_end, content_length);
        }
    };
    while bytes.len() < header_end + 4 + content_length {
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&buffer[..read]);
    }
    let header_text = String::from_utf8_lossy(&bytes[..header_end]).to_string();
    let request_line = header_text.lines().next().unwrap();
    let mut request_line = request_line.split_whitespace();
    CapturedRequest {
        method: request_line.next().unwrap().into(),
        path: request_line.next().unwrap().into(),
        headers: header_text,
        body: bytes[header_end + 4..header_end + 4 + content_length].to_vec(),
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test]
async fn artifact_upload_keeps_clark_bearer_off_the_presigned_host() {
    let organization_id = uuid::Uuid::new_v4().to_string();
    let repository_id = uuid::Uuid::new_v4().to_string();
    let scan_id = uuid::Uuid::new_v4().to_string();
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let artifact = ArtifactSpec {
        role: "manifest",
        storage_tier: "evidence",
        classification: "confidential",
        content_type: "application/json",
        bytes: br#"{"documentType":"clark-security.scan-manifest"}"#.to_vec(),
    };
    let sha256 = format!("sha256:{}", sha256_hex(&artifact.bytes));
    let identity = sha256_hex(format!("clark-security-artifact/v1\0manifest\0{sha256}").as_bytes());
    let client_artifact_id = format!("artifact:{identity}");
    let expected_record = json!({
        "id": artifact_id,
        "scanId": scan_id,
        "clientArtifactId": client_artifact_id,
        "role": "manifest",
        "storageTier": "evidence",
        "classification": "confidential",
        "objectVersionId": "version-1",
        "sizeBytes": artifact.bytes.len(),
        "sha256": sha256,
    });
    let grant_artifact_id = artifact_id.clone();
    let grant_organization_id = organization_id.clone();
    let grant_repository_id = repository_id.clone();
    let grant_scan_id = scan_id.clone();
    let grant_client_id = client_artifact_id.clone();
    let grant_sha = sha256.clone();
    let grant_size = artifact.bytes.len();
    let (base, captured, handle) = server(|base| {
        vec![
            ResponseSpec {
                status: "201 Created",
                body: json!({
                    "authorization": {
                        "id": grant_artifact_id,
                        "organizationId": grant_organization_id,
                        "repositoryId": grant_repository_id,
                        "scanId": grant_scan_id,
                        "clientArtifactId": grant_client_id,
                        "role": "manifest",
                        "storageTier": "evidence",
                        "classification": "confidential",
                        "contentType": "application/json",
                        "sizeBytes": grant_size,
                        "sha256": grant_sha,
                        "status": "pending",
                        "objectVersionId": null
                    },
                    "uploadUrl": format!("{base}/vault-upload"),
                    "uploadHeaders": [{"name": "x-clark-checksum", "value": "safe"}]
                })
                .to_string(),
            },
            ResponseSpec {
                status: "200 OK",
                body: "{}".into(),
            },
            ResponseSpec {
                status: "200 OK",
                body: expected_record.to_string(),
            },
        ]
    })
    .await;
    let client = ClarkSecurityPlatformClient::new(base, "ck_live_test".into(), http()).unwrap();
    let record = client
        .upload_artifact(&organization_id, &repository_id, &scan_id, &artifact)
        .await
        .unwrap();
    assert_eq!(record.id, artifact_id);
    handle.await.unwrap();

    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 3);
    assert_eq!(captured[0].method, "POST");
    assert!(captured[0]
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer ck_live_test"));
    assert_eq!(captured[1].method, "PUT");
    assert_eq!(captured[1].path, "/vault-upload");
    assert!(!captured[1]
        .headers
        .to_ascii_lowercase()
        .contains("authorization:"));
    assert_eq!(captured[1].body, artifact.bytes);
    assert_eq!(captured[2].method, "POST");
}

#[tokio::test]
async fn verified_artifact_retry_never_uploads_again() {
    let organization_id = uuid::Uuid::new_v4().to_string();
    let repository_id = uuid::Uuid::new_v4().to_string();
    let scan_id = uuid::Uuid::new_v4().to_string();
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let artifact = ArtifactSpec {
        role: "coverage",
        storage_tier: "evidence",
        classification: "confidential",
        content_type: "application/json",
        bytes: b"{}".to_vec(),
    };
    let sha256 = format!("sha256:{}", sha256_hex(&artifact.bytes));
    let identity = sha256_hex(format!("clark-security-artifact/v1\0coverage\0{sha256}").as_bytes());
    let client_artifact_id = format!("artifact:{identity}");
    let grant_organization_id = organization_id.clone();
    let grant_repository_id = repository_id.clone();
    let grant_scan_id = scan_id.clone();
    let grant_sha = sha256.clone();
    let grant_id = artifact_id.clone();
    let grant_client = client_artifact_id.clone();
    let (base, captured, handle) = server(move |_| {
        vec![ResponseSpec {
            status: "201 Created",
            body: json!({
                "authorization": {
                    "id": grant_id,
                    "organizationId": grant_organization_id,
                    "repositoryId": grant_repository_id,
                    "scanId": grant_scan_id,
                    "clientArtifactId": grant_client,
                    "role": "coverage",
                    "storageTier": "evidence",
                    "classification": "confidential",
                    "contentType": "application/json",
                    "sizeBytes": 2,
                    "sha256": grant_sha,
                    "status": "verified",
                    "objectVersionId": "immutable-version"
                },
                "uploadUrl": null,
                "uploadHeaders": []
            })
            .to_string(),
        }]
    })
    .await;
    let client = ClarkSecurityPlatformClient::new(base, "ck_live_test".into(), http()).unwrap();
    let record = client
        .upload_artifact(&organization_id, &repository_id, &scan_id, &artifact)
        .await
        .unwrap();
    assert_eq!(record.object_version_id, "immutable-version");
    handle.await.unwrap();
    assert_eq!(captured.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn exact_scan_id_is_sent_on_every_task_claim() {
    let organization_id = uuid::Uuid::new_v4().to_string();
    let repository_id = uuid::Uuid::new_v4().to_string();
    let scan_id = uuid::Uuid::new_v4().to_string();
    let (base, captured, handle) = server(|_| {
        vec![ResponseSpec {
            status: "204 No Content",
            body: String::new(),
        }]
    })
    .await;
    let client = ClarkSecurityPlatformClient::new(base, "ck_live_test".into(), http()).unwrap();
    assert!(client
        .claim_task(
            &organization_id,
            &repository_id,
            &scan_id,
            &uuid::Uuid::new_v4().to_string(),
            "inventory",
        )
        .await
        .unwrap()
        .is_none());
    handle.await.unwrap();
    let captured = captured.lock().unwrap();
    let body: serde_json::Value = serde_json::from_slice(&captured[0].body).unwrap();
    assert_eq!(body["scanId"], scan_id);
}
