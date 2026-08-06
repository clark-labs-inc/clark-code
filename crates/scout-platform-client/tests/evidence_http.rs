use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use scout_ingest_protocol::cartography::{
    CartographyChange, CartographyChangePage, CartographyChangeQuery, CollectorSigningKey,
    EvidenceStatus, EvidenceUploadAuthorization, EvidenceUploadGrant, EvidenceUploadRequest,
    GraphDeltaPage, GraphDeltaQuery, GraphObjectKind, GraphSnapshotPage, GraphSnapshotQuery,
    GraphSnapshotRef, SimulationOverlayPage, SimulationOverlayQuery, SimulationOverlayRecord,
    SimulationOverlayStatus, TaskClaimRequest, TaskClaimResponse, UploadHeader,
};
use scout_platform_client::{
    enroll_machine, ClarkCartographyClient, ClarkCartographyClientConfig,
    ClarkCartographyEnrollmentConfig, CollectorMachineIdentity, MachineEnrollment,
    MachineEnrollmentRequest, ScoutCartographySession, ScoutCartographySessionConfig,
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

#[derive(Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[tokio::test]
async fn enrolled_session_signs_claims_with_a_host_private_bound_identity() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let identity_root = tempfile::tempdir().unwrap();
    let binding = format!("{base_url}|{organization_id}|{workspace_id}");
    let identity =
        CollectorMachineIdentity::load_or_create(identity_root.path(), &binding).unwrap();
    let enrollment = MachineEnrollment {
        id: Uuid::new_v4(),
        organization_id,
        workspace_id,
        signer_id: identity.signer_id(),
        public_key: identity.public_key_hex(),
        platform: "linux".into(),
        architecture: "x86_64".into(),
        coordinator_public_key: "ab".repeat(32),
    };
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let server_recorded = recorded.clone();
    let enrollment_json = serde_json::to_vec(&enrollment).unwrap();
    let server = thread::spawn(move || {
        let (mut enroll_stream, _) = listener.accept().unwrap();
        server_recorded
            .lock()
            .unwrap()
            .push(read_request(&mut enroll_stream));
        write_response(
            &mut enroll_stream,
            "201 Created",
            "application/json",
            &enrollment_json,
        );
        let (mut claim_stream, _) = listener.accept().unwrap();
        let claim_request = read_request(&mut claim_stream);
        let claim: TaskClaimRequest = serde_json::from_slice(&claim_request.body).unwrap();
        let response = TaskClaimResponse {
            request_id: claim.request_id.clone(),
            task: None,
        };
        server_recorded.lock().unwrap().push(claim_request);
        write_response(
            &mut claim_stream,
            "200 OK",
            "application/json",
            &serde_json::to_vec(&response).unwrap(),
        );
    });

    let session = ScoutCartographySession::enroll(
        ScoutCartographySessionConfig::new(
            &base_url,
            "platform-test-key",
            identity_root.path(),
            organization_id,
            workspace_id,
            "linux",
            "x86_64",
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(session.enrollment(), &enrollment);
    let claim = session.claim_next_task(run_id, 30).await.unwrap();
    assert!(claim.task.is_none());
    server.join().unwrap();

    let requests = recorded.lock().unwrap();
    let posted_enrollment: MachineEnrollmentRequest =
        serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(posted_enrollment.public_key, identity.public_key_hex());
    let posted_claim: TaskClaimRequest = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(posted_claim.organization_id, organization_id);
    assert_eq!(posted_claim.workspace_id, workspace_id);
    assert_eq!(posted_claim.run_id, run_id);
    assert_eq!(posted_claim.machine_id, enrollment.id);
    assert_eq!(posted_claim.signer_id, identity.signer_id());
    assert_eq!(posted_claim.request_id, claim.request_id);
}

#[tokio::test]
async fn enrollment_pins_the_exact_backend_and_machine_binding() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let signer = CollectorSigningKey::from_seed([29; 32]);
    let request = MachineEnrollmentRequest {
        organization_id,
        workspace_id,
        public_key: signer.public_key_hex(),
        platform: "linux".into(),
        architecture: "x86_64".into(),
    };
    let enrollment = MachineEnrollment {
        id: Uuid::new_v4(),
        organization_id,
        workspace_id,
        signer_id: signer.signer_id(),
        public_key: signer.public_key_hex(),
        platform: "linux".into(),
        architecture: "x86_64".into(),
        coordinator_public_key: "ab".repeat(32),
    };
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let server_recorded = recorded.clone();
    let response_json = serde_json::to_vec(&enrollment).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        server_recorded
            .lock()
            .unwrap()
            .push(read_request(&mut stream));
        write_response(
            &mut stream,
            "201 Created",
            "application/json",
            &response_json,
        );
    });

    let enrolled = enroll_machine(
        ClarkCartographyEnrollmentConfig::new(&base_url, "platform-test-key").unwrap(),
        &request,
    )
    .await
    .unwrap();
    assert_eq!(enrolled.enrollment, enrollment);
    server.join().unwrap();

    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/system-cartography/machines/enroll");
    assert!(header(&requests[0], "authorization")
        .is_some_and(|value| value == "Bearer platform-test-key"));
    let posted: MachineEnrollmentRequest = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(posted, request);
}

#[tokio::test]
async fn snapshot_query_posts_the_exact_tenant_and_temporal_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let query = GraphSnapshotQuery {
        organization_id,
        workspace_id,
        effective_at_ms: Some(1_700_000_000_000),
        known_at_ms: Some(1_700_000_000_100),
        object_kinds: BTreeSet::from([GraphObjectKind::Entity, GraphObjectKind::Edge]),
        limit: 25,
        cursor: None,
    };
    let page = GraphSnapshotPage {
        organization_id,
        workspace_id,
        effective_at_ms: query.effective_at_ms.unwrap(),
        known_at_ms: query.known_at_ms.unwrap(),
        entries: Vec::new(),
        next_cursor: None,
    };
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let server_recorded = recorded.clone();
    let page_json = serde_json::to_vec(&page).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        server_recorded
            .lock()
            .unwrap()
            .push(read_request(&mut stream));
        write_response(&mut stream, "200 OK", "application/json", &page_json);
    });

    let client = ClarkCartographyClient::new(
        ClarkCartographyClientConfig::new(&base_url, "platform-test-key", "ab".repeat(32)).unwrap(),
    )
    .unwrap();
    assert_eq!(client.query_snapshot(&query).await.unwrap(), page);
    server.join().unwrap();

    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/system-cartography/snapshots/query");
    assert!(header(&requests[0], "authorization")
        .is_some_and(|value| value == "Bearer platform-test-key"));
    let posted: GraphSnapshotQuery = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(posted, query);
}

#[tokio::test]
async fn delta_query_posts_the_exact_pinned_temporal_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let query = GraphDeltaQuery {
        organization_id,
        workspace_id,
        from_effective_at_ms: 1_700_000_000_000,
        from_known_at_ms: Some(1_700_000_000_500),
        to_effective_at_ms: Some(1_700_000_001_000),
        to_known_at_ms: Some(1_700_000_002_000),
        object_kinds: BTreeSet::from([GraphObjectKind::Entity, GraphObjectKind::Edge]),
        include_unchanged: false,
        limit: 25,
        cursor: None,
    };
    let snapshot_ref = |effective_at_ms, known_at_ms| GraphSnapshotRef {
        organization_id,
        workspace_id,
        effective_at_ms,
        known_at_ms,
        filter_sha256: "ab".repeat(32),
    };
    let page = GraphDeltaPage {
        organization_id,
        workspace_id,
        from_snapshot: snapshot_ref(query.from_effective_at_ms, query.from_known_at_ms.unwrap()),
        to_snapshot: snapshot_ref(
            query.to_effective_at_ms.unwrap(),
            query.to_known_at_ms.unwrap(),
        ),
        entries: Vec::new(),
        next_cursor: None,
    };
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let server_recorded = recorded.clone();
    let page_json = serde_json::to_vec(&page).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        server_recorded
            .lock()
            .unwrap()
            .push(read_request(&mut stream));
        write_response(&mut stream, "200 OK", "application/json", &page_json);
    });

    let client = ClarkCartographyClient::new(
        ClarkCartographyClientConfig::new(&base_url, "platform-test-key", "ab".repeat(32)).unwrap(),
    )
    .unwrap();
    assert_eq!(client.query_delta(&query).await.unwrap(), page);
    server.join().unwrap();

    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/system-cartography/deltas/query");
    let posted: GraphDeltaQuery = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(posted, query);
}

#[tokio::test]
async fn simulation_query_pins_the_overlay_identity_and_version() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let query = SimulationOverlayQuery {
        organization_id,
        workspace_id,
        stable_key: "checkout.failure-model".into(),
        version: Some(3),
        limit: 100,
        cursor: None,
    };
    let page = SimulationOverlayPage {
        overlay: SimulationOverlayRecord {
            id: Uuid::new_v4(),
            organization_id,
            workspace_id,
            stable_key: query.stable_key.clone(),
            version: query.version.unwrap(),
            name: "Checkout failure model".into(),
            status: SimulationOverlayStatus::Complete,
            snapshot: GraphSnapshotRef {
                organization_id,
                workspace_id,
                effective_at_ms: 1_700_000_000_000,
                known_at_ms: 1_700_000_001_000,
                filter_sha256: "ab".repeat(32),
            },
            content_sha256: "cd".repeat(32),
            summary: serde_json::json!({"covered": 42}),
            created_at_ms: 1_700_000_002_000,
        },
        memberships: Vec::new(),
        next_cursor: None,
    };
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let server_recorded = recorded.clone();
    let page_json = serde_json::to_vec(&page).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        server_recorded
            .lock()
            .unwrap()
            .push(read_request(&mut stream));
        write_response(&mut stream, "200 OK", "application/json", &page_json);
    });

    let client = ClarkCartographyClient::new(
        ClarkCartographyClientConfig::new(&base_url, "platform-test-key", "ab".repeat(32)).unwrap(),
    )
    .unwrap();
    assert_eq!(client.query_simulation_overlay(&query).await.unwrap(), page);
    server.join().unwrap();

    let requests = recorded.lock().unwrap();
    assert_eq!(
        requests[0].path,
        "/v1/system-cartography/simulation-overlays/query"
    );
    let posted: SimulationOverlayQuery = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(posted, query);
}

#[tokio::test]
async fn change_query_preserves_monotonic_workspace_sequence() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let query = CartographyChangeQuery {
        organization_id,
        workspace_id,
        after_sequence: 41,
        limit: 100,
    };
    let page = CartographyChangePage {
        organization_id,
        workspace_id,
        changes: vec![CartographyChange {
            organization_id,
            workspace_id,
            sequence: 42,
            event_type: "batch_accepted".into(),
            occurred_at_ms: 1_700_000_000_000,
            payload: serde_json::json!({"batch_id": format!("batch:{}", "a".repeat(64))}),
        }],
        next_after_sequence: 42,
    };
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let server_recorded = recorded.clone();
    let page_json = serde_json::to_vec(&page).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        server_recorded
            .lock()
            .unwrap()
            .push(read_request(&mut stream));
        write_response(&mut stream, "200 OK", "application/json", &page_json);
    });

    let client = ClarkCartographyClient::new(
        ClarkCartographyClientConfig::new(&base_url, "platform-test-key", "ab".repeat(32)).unwrap(),
    )
    .unwrap();
    assert_eq!(client.query_changes(&query).await.unwrap(), page);
    server.join().unwrap();

    let requests = recorded.lock().unwrap();
    assert_eq!(requests[0].path, "/v1/system-cartography/changes/query");
    let posted: CartographyChangeQuery = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(posted, query);
}

#[tokio::test]
async fn authorizes_then_uploads_exact_bytes_without_platform_credentials_on_s3_put() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let bytes = b"portable-evidence".to_vec();
    let sha256 = hex_lower(&Sha256::digest(&bytes));
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let source_id = Uuid::new_v4();
    let machine_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let signer = CollectorSigningKey::from_seed([19; 32]);
    let request = EvidenceUploadRequest::sign(
        organization_id,
        workspace_id,
        run_id,
        source_id,
        machine_id,
        task_id,
        3,
        "portable-upload-nonce".into(),
        now_ms(),
        "application/octet-stream".into(),
        bytes.len() as u64,
        sha256.clone(),
        &signer,
    )
    .unwrap();
    let grant = EvidenceUploadGrant {
        authorization: EvidenceUploadAuthorization {
            evidence_id: request.evidence_id.clone(),
            organization_id,
            workspace_id,
            run_id,
            source_id,
            machine_id,
            task_id,
            fence: 3,
            bucket: "clark-artifacts-test".into(),
            key: "system-cartography/v1/test/evidence.bin".into(),
            content_type: "application/octet-stream".into(),
            size_bytes: bytes.len() as u64,
            sha256,
            version_id: None,
            expires_at_ms: now_ms() + 60_000,
            status: EvidenceStatus::Pending,
        },
        upload_url: Some(format!("{base_url}/authorized-upload")),
        upload_headers: vec![UploadHeader {
            name: "x-amz-meta-sha256".into(),
            value: request.sha256.clone(),
        }],
    };
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let server_recorded = recorded.clone();
    let grant_json = serde_json::to_vec(&grant).unwrap();
    let server = thread::spawn(move || {
        let (mut authorize_stream, _) = listener.accept().unwrap();
        server_recorded
            .lock()
            .unwrap()
            .push(read_request(&mut authorize_stream));
        write_response(
            &mut authorize_stream,
            "201 Created",
            "application/json",
            &grant_json,
        );
        let (mut upload_stream, _) = listener.accept().unwrap();
        server_recorded
            .lock()
            .unwrap()
            .push(read_request(&mut upload_stream));
        write_response(
            &mut upload_stream,
            "200 OK",
            "application/octet-stream",
            b"",
        );
    });

    let client = ClarkCartographyClient::new(
        ClarkCartographyClientConfig::new(&base_url, "platform-test-key", "ab".repeat(32)).unwrap(),
    )
    .unwrap();
    let returned = client.authorize_evidence(&request).await.unwrap();
    assert_eq!(returned, grant);
    client.upload_evidence(&returned, &bytes).await.unwrap();
    server.join().unwrap();

    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/v1/system-cartography/evidence/uploads");
    assert!(header(&requests[0], "authorization")
        .is_some_and(|value| value == "Bearer platform-test-key"));
    let posted: EvidenceUploadRequest = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(posted, request);

    assert_eq!(requests[1].method, "PUT");
    assert_eq!(requests[1].path, "/authorized-upload");
    assert_eq!(requests[1].body, bytes);
    assert!(header(&requests[1], "authorization").is_none());
    assert_eq!(
        header(&requests[1], "x-amz-meta-sha256"),
        Some(request.sha256.as_str())
    );
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    let (header_end, content_length) = loop {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = head
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find_map(|(name, value)| {
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
    let mut lines = head.lines();
    let request_line = lines.next().unwrap();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap().to_owned();
    let path = parts.next().unwrap().to_owned();
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

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
}

fn header<'a>(request: &'a RecordedRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
