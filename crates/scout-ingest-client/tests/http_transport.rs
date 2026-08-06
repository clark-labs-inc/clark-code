use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseBatchBundle, EnterpriseEntityKind, EnterpriseEvent,
    EnterpriseFact, EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseTrustChain, EnterpriseTrustManifest, GraphEntityObservation,
};
use scout_ingest_client::{
    CentralIngestTransport, ReqwestCentralIngestTransport, ReqwestTransportConfig,
};
use scout_ingest_protocol::{CoordinatorSigningKey, IngestReceipt, IngestRequest, ScoutTenantId};

const TEST_API_KEY: &str = "ck_live_test_only";

#[derive(Clone)]
struct ResponseSpec {
    status: &'static str,
    content_type: Option<&'static str>,
    body: Vec<u8>,
    delay: Duration,
}

impl ResponseSpec {
    fn json(status: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: Some("application/json"),
            body,
            delay: Duration::ZERO,
        }
    }
}

#[derive(Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct FakeServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: thread::JoinHandle<()>,
}

impl FakeServer {
    fn start(responses: Vec<ResponseSpec>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                captured.lock().unwrap().push(read_request(&mut stream));
                if !response.delay.is_zero() {
                    thread::sleep(response.delay);
                }
                write_response(&mut stream, &response);
            }
        });
        Self {
            base_url: format!("http://{address}/ignored/base"),
            requests,
            handle,
        }
    }

    fn transport(&self) -> ReqwestCentralIngestTransport {
        ReqwestCentralIngestTransport::new(
            ReqwestTransportConfig::new(&self.base_url, TEST_API_KEY).unwrap(),
        )
        .unwrap()
    }

    fn finish(self) -> Vec<RecordedRequest> {
        self.handle.join().unwrap();
        let requests = Arc::try_unwrap(self.requests)
            .unwrap_or_else(|_| panic!("fake server retained a request capture reference"));
        requests
            .into_inner()
            .unwrap_or_else(|_| panic!("fake server request capture mutex was poisoned"))
    }
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    let (header_end, content_length) = loop {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0, "client closed before sending a complete request");
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
                .unwrap_or_default();
            if bytes.len() >= header_end + 4 + content_length {
                break (header_end, content_length);
            }
        }
    };
    let head = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap().to_owned();
    let path = request_parts.next().unwrap().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    RecordedRequest {
        method,
        path,
        headers,
        body: bytes[header_end + 4..header_end + 4 + content_length].to_vec(),
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_response(stream: &mut TcpStream, response: &ResponseSpec) {
    let content_type = response
        .content_type
        .map(|value| format!("Content-Type: {value}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        content_type,
        response.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&response.body);
}

fn ingest_request() -> IngestRequest {
    let tenant_id = ScoutTenantId::new("organization:http-transport").unwrap();
    let enterprise_id = EnterpriseId::new("http-transport-enterprise").unwrap();
    let signer = EnterpriseSigningKey::from_seed([51; 32]);
    let manifest = EnterpriseTrustManifest::initial(
        enterprise_id.clone(),
        format!("trust:{}", "a".repeat(64)),
        100,
        100_000,
        &signer,
    )
    .unwrap();
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: manifest.manifest_id.clone(),
        manifests: vec![manifest.clone()],
    };
    let observation = GraphEntityObservation::new(
        &enterprise_id,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:test", "service:http").unwrap(),
        BTreeSet::from(["http".into()]),
        BTreeSet::from(["b".repeat(64)]),
    )
    .unwrap();
    let event = EnterpriseEvent::new(
        enterprise_id.clone(),
        EnterpriseProvenance {
            machine_id: "machine-http".into(),
            run_id: "run-http".into(),
            adapter_instance_id: "aws-test".into(),
            auth_context_id: "auth-read-only".into(),
            discovery_epoch: "epoch-http".into(),
            discovery_epoch_sequence: 1,
            source_sequence: 1,
            observed_at_ms: 1_000,
            source_fingerprint: "c".repeat(64),
        },
        EnterpriseFact::EntityObserved(observation),
    )
    .unwrap();
    let batch = EnterpriseBatch::new(enterprise_id, [event]).unwrap();
    let grant = EnterpriseSignerGrant::issue(
        &manifest,
        signer.signer_id(),
        signer.public_key_hex(),
        BTreeSet::from([
            EnterpriseSignerRole::Collector,
            EnterpriseSignerRole::Coordinator,
        ]),
        EnterpriseGrantScope {
            machine_id: "machine-http".into(),
            run_id: "run-http".into(),
            adapter_instance_id: "aws-test".into(),
            auth_context_id: "auth-read-only".into(),
            discovery_epoch: "epoch-http".into(),
            discovery_epoch_sequence: 1,
            first_source_sequence: 1,
            last_source_sequence: 1,
        },
        100,
        90_000,
        &[&signer],
    )
    .unwrap();
    let signed_batch =
        EnterpriseSignedBatch::sign(batch, &manifest, grant, 1_000, &signer).unwrap();
    IngestRequest::new(
        tenant_id,
        format!("outbox-attempt:{}", "d".repeat(64)),
        EnterpriseBatchBundle {
            trust_chain: chain,
            signed_batch,
        },
    )
    .unwrap()
}

fn receipt(request: &IngestRequest) -> IngestReceipt {
    IngestReceipt::issue(
        request.tenant_id.clone(),
        request.bundle.signed_batch.batch.enterprise_id.clone(),
        request.bundle.trust_chain.anchor_manifest_id.clone(),
        request.bundle.signed_batch.batch.batch_id.clone(),
        request.envelope_sha256().unwrap(),
        "e".repeat(64),
        1,
        1,
        20_000,
        None,
        &CoordinatorSigningKey::from_seed([61; 32]),
    )
    .unwrap()
}

#[tokio::test]
async fn posts_bearer_json_with_signed_batch_idempotency_key() {
    let request = ingest_request();
    let expected_receipt = receipt(&request);
    let server = FakeServer::start(vec![ResponseSpec::json(
        "201 Created",
        serde_json::to_vec(&expected_receipt).unwrap(),
    )]);

    let actual = server.transport().submit(&request).await.unwrap();
    assert_eq!(actual, expected_receipt);
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    let captured = &requests[0];
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/v1/scout/enterprise-batches");
    assert_eq!(
        captured.headers.get("authorization").map(String::as_str),
        Some("Bearer ck_live_test_only")
    );
    assert_eq!(
        captured.headers.get("idempotency-key").map(String::as_str),
        Some(request.bundle.signed_batch.batch.batch_id.as_str())
    );
    assert_ne!(
        captured.headers.get("idempotency-key").map(String::as_str),
        Some(request.attempt_id.as_str())
    );
    assert_eq!(
        captured.headers.get("accept").map(String::as_str),
        Some("application/json")
    );
    assert!(captured
        .headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("application/json")));
    let posted: IngestRequest = serde_json::from_slice(&captured.body).unwrap();
    assert_eq!(posted, request);
}

#[tokio::test]
async fn replay_reuses_batch_idempotency_key_across_attempts() {
    let request = ingest_request();
    let expected_receipt = receipt(&request);
    let response = ResponseSpec::json("200 OK", serde_json::to_vec(&expected_receipt).unwrap());
    let server = FakeServer::start(vec![response.clone(), response]);
    let transport = server.transport();

    assert_eq!(transport.submit(&request).await.unwrap(), expected_receipt);
    let mut replay = request.clone();
    replay.attempt_id = format!("outbox-attempt:{}", "e".repeat(64));
    assert_eq!(transport.submit(&replay).await.unwrap(), expected_receipt);

    let requests = server.finish();
    let first_key = requests[0].headers.get("idempotency-key").unwrap();
    let second_key = requests[1].headers.get("idempotency-key").unwrap();
    assert_eq!(first_key, second_key);
    assert_eq!(
        first_key,
        request.bundle.signed_batch.batch.batch_id.as_str()
    );
    assert_ne!(requests[0].body, requests[1].body);
}

#[tokio::test]
async fn reports_401_and_409_without_treating_them_as_receipts() {
    let request = ingest_request();
    let server = FakeServer::start(vec![
        ResponseSpec::json("401 Unauthorized", br#"{"error":"unauthorized"}"#.to_vec()),
        ResponseSpec::json("409 Conflict", br#"{"error":"batch conflict"}"#.to_vec()),
    ]);
    let transport = server.transport();

    let unauthorized = transport.submit(&request).await.unwrap_err();
    assert!(unauthorized.contains("401 Unauthorized"));
    assert!(unauthorized.contains("unauthorized"));
    let conflict = transport.submit(&request).await.unwrap_err();
    assert!(conflict.contains("409 Conflict"));
    assert!(conflict.contains("batch conflict"));
    assert_eq!(server.finish().len(), 2);
}

#[tokio::test]
async fn rejects_oversized_receipt_body() {
    let request = ingest_request();
    let server = FakeServer::start(vec![ResponseSpec::json(
        "200 OK",
        vec![b' '; 256 * 1024 + 1],
    )]);

    let error = server.transport().submit(&request).await.unwrap_err();
    assert!(error.contains("receipt body exceeds 262144 bytes"));
    assert_eq!(server.finish().len(), 1);
}

#[tokio::test]
async fn bounds_oversized_error_body() {
    let request = ingest_request();
    let server = FakeServer::start(vec![ResponseSpec::json(
        "409 Conflict",
        vec![b'x'; 16 * 1024 + 1],
    )]);

    let error = server.transport().submit(&request).await.unwrap_err();
    assert!(error.contains("409 Conflict"));
    assert!(error.contains("error body exceeds 16384 bytes"));
    assert_eq!(server.finish().len(), 1);
}

#[tokio::test]
async fn rejects_malformed_and_non_strict_receipt_json() {
    let request = ingest_request();
    let valid_receipt = receipt(&request);
    let mut receipt_with_unknown_field = serde_json::to_value(valid_receipt).unwrap();
    receipt_with_unknown_field
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), serde_json::json!(true));
    let server = FakeServer::start(vec![
        ResponseSpec::json("200 OK", br#"{"schema_version":"#.to_vec()),
        ResponseSpec::json(
            "200 OK",
            serde_json::to_vec(&receipt_with_unknown_field).unwrap(),
        ),
    ]);
    let transport = server.transport();

    let malformed = transport.submit(&request).await.unwrap_err();
    assert!(malformed.contains("invalid Scout central-ingestion receipt JSON"));
    let unknown = transport.submit(&request).await.unwrap_err();
    assert!(unknown.contains("unknown field `unexpected`"));
    assert_eq!(server.finish().len(), 2);
}

#[tokio::test]
async fn enforces_configured_request_timeout() {
    let request = ingest_request();
    let response = ResponseSpec {
        delay: Duration::from_millis(250),
        ..ResponseSpec::json("200 OK", serde_json::to_vec(&receipt(&request)).unwrap())
    };
    let server = FakeServer::start(vec![response]);
    let transport = ReqwestCentralIngestTransport::new(
        ReqwestTransportConfig::new(&server.base_url, TEST_API_KEY)
            .unwrap()
            .with_request_timeout(Duration::from_millis(50))
            .unwrap(),
    )
    .unwrap();

    let error = transport.submit(&request).await.unwrap_err();
    assert_eq!(error, "Scout central-ingestion request timed out");
    assert_eq!(server.finish().len(), 1);
}
