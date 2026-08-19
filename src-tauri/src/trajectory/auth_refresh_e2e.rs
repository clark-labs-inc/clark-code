use super::{AppendRequest, CloudTrajectoryClient, CloudTrajectoryConfig, TrajectoryCloudBoundary};
use crate::commands::ProductCloudOutcome;
use crate::runtime_registry::{AccountKey, CloudAccountState};
use crate::state::AppState;
use agent_core::{AgentEvent, RunId, Snapshot};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

const OWNER_SCOPE: &str = "account-hyper-realistic";
const CONVERSATION_ID: &str = "conversation/auth refresh";
const EXPIRED_TOKEN: &str = "fixture-expired-jwt";
const REFRESHED_TOKEN: &str = "fixture-refreshed-jwt";

#[derive(Clone, Debug, Eq, PartialEq)]
enum RendererEvent {
    AuthExpired,
    SyncWarning(String),
    ConversationDeleted(String),
}

struct HttpProductBoundary {
    http: reqwest::Client,
    state: AppState,
    renderer_events: Arc<StdMutex<Vec<RendererEvent>>>,
    auth_expired: mpsc::UnboundedSender<()>,
}

#[async_trait::async_trait]
impl TrajectoryCloudBoundary for HttpProductBoundary {
    async fn append(
        &self,
        conversation_id: &str,
        request: &AppendRequest,
    ) -> Result<ProductCloudOutcome, String> {
        // This is the downstream product's real trajectory HTTP shape: the
        // bearer stays native, the conversation id is path-encoded, and only
        // the durable AppendRequest crosses the wire.
        let authority = self
            .state
            .runtime_registry
            .cloud_account()
            .await
            .ok_or("mock product has no native cloud account")?;
        let response = self
            .http
            .post(format!(
                "{}/api/desktop/conversations/{}/trajectory",
                authority.rest_base,
                urlencoding::encode(conversation_id),
            ))
            .bearer_auth(authority.token.as_str())
            .json(request)
            .send()
            .await
            .map_err(|error| format!("mock Clark cloud transport failed: {error}"))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let error = format!("mock Clark cloud request returned {status}: {body}");
        Ok(match status {
            status if status.is_success() => ProductCloudOutcome::Ok(Value::Null),
            reqwest::StatusCode::UNAUTHORIZED => ProductCloudOutcome::Unauthorized(error),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE => {
                ProductCloudOutcome::NotFound(error)
            }
            reqwest::StatusCode::CONFLICT => ProductCloudOutcome::Conflict(error),
            reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::TOO_MANY_REQUESTS => {
                ProductCloudOutcome::Unavailable(error)
            }
            status if status.is_server_error() => ProductCloudOutcome::Unavailable(error),
            _ => ProductCloudOutcome::Rejected(error),
        })
    }

    fn emit_auth_expired(&self) {
        self.record(RendererEvent::AuthExpired);
        let _ = self.auth_expired.send(());
    }

    fn emit_sync_warning(&self, message: &str) {
        self.record(RendererEvent::SyncWarning(message.to_string()));
    }

    fn emit_conversation_deleted(&self, conversation_id: &str) {
        self.record(RendererEvent::ConversationDeleted(
            conversation_id.to_string(),
        ));
    }
}

impl HttpProductBoundary {
    fn record(&self, event: RendererEvent) {
        self.renderer_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
    }
}

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    raw_body: Vec<u8>,
    body: Value,
}

#[tokio::test]
async fn expired_token_refreshes_and_replays_the_durable_outbox_end_to_end() {
    let (gateway_base, gateway) = spawn_scripted_gateway().await;
    let state = AppState::new();
    state
        .runtime_registry
        .set_cloud_account(Some(cloud_account(&gateway_base, EXPIRED_TOKEN)))
        .await;

    let renderer_events = Arc::new(StdMutex::new(Vec::new()));
    let (auth_expired_tx, mut auth_expired_rx) = mpsc::unbounded_channel();
    let cloud = Arc::new(HttpProductBoundary {
        http: reqwest::Client::new(),
        state: state.clone(),
        renderer_events: renderer_events.clone(),
        auth_expired: auth_expired_tx,
    });
    let directory = tempfile::tempdir().unwrap();
    let database_path = directory.path().join("cloud-history-outbox.sqlite3");
    let client = CloudTrajectoryClient::with_cloud_boundary(
        CONVERSATION_ID.into(),
        config(),
        OWNER_SCOPE.into(),
        state.clone(),
        cloud,
        database_path.clone(),
    );

    // This task is the WebView refresh actor: it reacts only to the real
    // auth-expired event, takes long enough to prove the uploader waits, then
    // publishes a same-account token generation back to the native registry.
    let refresh_state = state.clone();
    let refresh_base = gateway_base.clone();
    let renderer_refresh = tokio::spawn(async move {
        auth_expired_rx
            .recv()
            .await
            .expect("native host should request one renderer refresh");
        tokio::time::sleep(Duration::from_millis(75)).await;
        refresh_state
            .runtime_registry
            .set_cloud_account(Some(cloud_account(&refresh_base, REFRESHED_TOKEN)))
            .await;
    });

    client.initialize(&Snapshot::new(), 0).await.unwrap();
    let local_sequence = client
        .append(&[AgentEvent::RunStarted {
            run: RunId::new("run-auth-refresh"),
        }])
        .await
        .unwrap();
    assert_eq!(local_sequence, 1, "the run must be durably queued first");

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if client.outbox.pending().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("refreshed credentials should drain the durable outbox");
    renderer_refresh.await.unwrap();
    let requests = tokio::time::timeout(Duration::from_secs(1), gateway)
        .await
        .expect("mock gateway should receive the authenticated retry")
        .unwrap();

    assert_eq!(requests.len(), 2);
    assert_request_contract(&requests[0], EXPIRED_TOKEN);
    assert_request_contract(&requests[1], REFRESHED_TOKEN);
    assert_eq!(
        requests[0].raw_body, requests[1].raw_body,
        "retry must replay the byte-equivalent batch with stable event ids"
    );
    assert_eq!(
        requests[0].body["events"][0]["payload"]["event"],
        json!({"event": "run_started", "run": "run-auth-refresh"})
    );
    assert_eq!(
        requests[0].body["events"][0]["payload"]["metadata"]["testScenario"],
        "expired-token-refresh"
    );

    let recorded_events = renderer_events
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(
        recorded_events,
        vec![RendererEvent::AuthExpired],
        "a successful refresh must not masquerade as cloud unreachability"
    );

    let connection = rusqlite::Connection::open(&database_path).unwrap();
    let (acknowledged, stored): (i64, Vec<u8>) = connection
        .query_row(
            "SELECT acknowledged, request_json FROM trajectory_outbox WHERE local_seq = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(acknowledged, 1, "the 200 response must acknowledge SQLite");
    assert_eq!(
        stored, requests[0].raw_body,
        "the network retry must be the exact batch committed before transport"
    );
}

fn config() -> CloudTrajectoryConfig {
    CloudTrajectoryConfig {
        title: "Refresh recovery fixture".into(),
        provider: "clark_max".into(),
        project: Some("/fixture/project".into()),
        repository_fingerprint: Some("fixture-repository-fingerprint".into()),
        remote_host: None,
        mode: Some("local".into()),
        metadata: json!({
            "testScenario": "expired-token-refresh",
            "specialistContext": {
                "kind": "code",
                "workflow": "code:default"
            }
        }),
    }
}

fn cloud_account(base: &str, token: &str) -> CloudAccountState {
    CloudAccountState {
        rest_base: base.to_string(),
        account: AccountKey::new(OWNER_SCOPE).unwrap(),
        token: Zeroizing::new(token.to_string()),
    }
}

fn assert_request_contract(request: &CapturedRequest, token: &str) {
    let expected_authorization = format!("Bearer {token}");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.path,
        "/api/desktop/conversations/conversation%2Fauth%20refresh/trajectory"
    );
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some(expected_authorization.as_str())
    );
    assert!(request
        .headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("application/json")));
    assert_eq!(
        request.body["conversation"]["title"],
        "Refresh recovery fixture"
    );
    assert_eq!(request.body["events"].as_array().map(Vec::len), Some(1));
}

async fn spawn_scripted_gateway() -> (String, tokio::task::JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let gateway = tokio::spawn(async move {
        let mut captured = Vec::new();
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            captured.push(read_request(&mut stream).await);
            if attempt == 0 {
                write_response(
                    &mut stream,
                    "401 Unauthorized",
                    json!({"message": "fixture access token expired"}),
                )
                .await;
            } else {
                write_response(
                    &mut stream,
                    "200 OK",
                    json!({"accepted": 1, "headRevision": 1}),
                )
                .await;
            }
        }
        captured
    });
    (format!("http://{address}"), gateway)
}

async fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0_u8; 4096];
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0, "mock gateway connection closed before headers");
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers_text = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = headers_text.split("\r\n");
    let mut request_line = lines.next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_string();
    let path = request_line.next().unwrap().to_string();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect::<HashMap<_, _>>();
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();
    while bytes.len() < header_end + content_length {
        let mut buffer = [0_u8; 4096];
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(
            read > 0,
            "mock gateway connection closed before the JSON body"
        );
        bytes.extend_from_slice(&buffer[..read]);
    }
    let raw_body = bytes[header_end..header_end + content_length].to_vec();
    let body = serde_json::from_slice(&raw_body).unwrap();
    CapturedRequest {
        method,
        path,
        headers,
        raw_body,
        body,
    }
}

async fn write_response(stream: &mut TcpStream, status: &str, body: Value) {
    let body = serde_json::to_vec(&body).unwrap();
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await.unwrap();
    stream.write_all(&body).await.unwrap();
    stream.shutdown().await.unwrap();
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
