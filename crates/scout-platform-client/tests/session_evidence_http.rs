use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use scout_ingest_protocol::cartography::{
    EvidenceCommitOutcome, EvidenceCommitRequest, EvidenceStatus, EvidenceUploadGrant,
    EvidenceUploadRequest,
};
use scout_platform_client::{
    CollectorMachineIdentity, MachineEnrollment, ScoutCartographySession,
    ScoutCartographySessionConfig,
};
use uuid::Uuid;

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

#[tokio::test]
async fn session_uploads_commits_and_returns_immutable_evidence_reference() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let source_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
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
    let enrollment_json = serde_json::to_vec(&enrollment).unwrap();
    let server_base_url = base_url.clone();
    let server = thread::spawn(move || {
        let (mut enroll_stream, _) = listener.accept().unwrap();
        let enroll = read_request(&mut enroll_stream);
        assert_eq!(enroll.path, "/v1/cartography/machines/enroll");
        write_response(
            &mut enroll_stream,
            "201 Created",
            "application/json",
            &enrollment_json,
        );

        let (mut authorize_stream, _) = listener.accept().unwrap();
        let authorize = read_request(&mut authorize_stream);
        assert_eq!(authorize.path, "/v1/cartography/evidence/uploads");
        let upload: EvidenceUploadRequest = serde_json::from_slice(&authorize.body).unwrap();
        let authorization = scout_ingest_protocol::cartography::EvidenceUploadAuthorization {
            evidence_id: upload.evidence_id.clone(),
            organization_id: upload.organization_id,
            workspace_id: upload.workspace_id,
            run_id: upload.run_id,
            source_id: upload.source_id,
            machine_id: upload.machine_id,
            task_id: upload.task_id,
            fence: upload.fence,
            bucket: "cartography-evidence".into(),
            key: format!("system-cartography/v1/{}.json", upload.evidence_id),
            content_type: upload.content_type.clone(),
            size_bytes: upload.size_bytes,
            sha256: upload.sha256.clone(),
            version_id: None,
            expires_at_ms: now_ms() + 60_000,
            status: EvidenceStatus::Pending,
        };
        let grant = EvidenceUploadGrant {
            authorization: authorization.clone(),
            upload_url: Some(format!("{server_base_url}/evidence-put")),
            upload_headers: Vec::new(),
        };
        write_response(
            &mut authorize_stream,
            "201 Created",
            "application/json",
            &serde_json::to_vec(&grant).unwrap(),
        );

        let (mut upload_stream, _) = listener.accept().unwrap();
        let uploaded = read_request(&mut upload_stream);
        assert_eq!(uploaded.method, "PUT");
        assert_eq!(uploaded.path, "/evidence-put");
        assert_eq!(uploaded.body, b"{\"safe\":\"metadata\"}");
        write_response(
            &mut upload_stream,
            "200 OK",
            "application/octet-stream",
            b"",
        );

        let (mut commit_stream, _) = listener.accept().unwrap();
        let commit_request = read_request(&mut commit_stream);
        assert_eq!(commit_request.path, "/v1/cartography/evidence/commits");
        let commit: EvidenceCommitRequest = serde_json::from_slice(&commit_request.body).unwrap();
        assert_eq!(commit.evidence_id, authorization.evidence_id);
        let mut verified = authorization;
        verified.status = EvidenceStatus::Verified;
        verified.version_id = Some("s3-version-1".into());
        let outcome = EvidenceCommitOutcome {
            evidence: verified,
            rejection_reason: None,
        };
        write_response(
            &mut commit_stream,
            "200 OK",
            "application/json",
            &serde_json::to_vec(&outcome).unwrap(),
        );
    });

    let session = ScoutCartographySession::enroll(
        ScoutCartographySessionConfig::new(
            &base_url,
            "platform-test-key",
            "/v1/cartography",
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
    let evidence = session
        .upload_evidence(
            run_id,
            source_id,
            task_id,
            7,
            "application/json",
            b"{\"safe\":\"metadata\"}",
        )
        .await
        .unwrap();
    server.join().unwrap();

    assert_eq!(evidence.bucket, "cartography-evidence");
    assert_eq!(evidence.version_id.as_deref(), Some("s3-version-1"));
    assert_eq!(evidence.size_bytes, 19);
}

#[tokio::test]
async fn idempotent_retry_reuses_verified_evidence_without_another_put() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let organization_id = Uuid::new_v4();
    let workspace_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let source_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let requested_at_ms = now_ms();
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
    let enrollment_json = serde_json::to_vec(&enrollment).unwrap();
    let server = thread::spawn(move || {
        let (mut enroll_stream, _) = listener.accept().unwrap();
        let enroll = read_request(&mut enroll_stream);
        assert_eq!(enroll.path, "/v1/cartography/machines/enroll");
        write_response(
            &mut enroll_stream,
            "201 Created",
            "application/json",
            &enrollment_json,
        );

        let mut first_upload = None;
        for _ in 0..2 {
            let (mut authorize_stream, _) = listener.accept().unwrap();
            let authorize = read_request(&mut authorize_stream);
            assert_eq!(authorize.path, "/v1/cartography/evidence/uploads");
            let upload: EvidenceUploadRequest = serde_json::from_slice(&authorize.body).unwrap();
            if let Some(first) = &first_upload {
                assert_eq!(&upload, first);
            } else {
                first_upload = Some(upload.clone());
            }
            let authorization = scout_ingest_protocol::cartography::EvidenceUploadAuthorization {
                evidence_id: upload.evidence_id,
                organization_id: upload.organization_id,
                workspace_id: upload.workspace_id,
                run_id: upload.run_id,
                source_id: upload.source_id,
                machine_id: upload.machine_id,
                task_id: upload.task_id,
                fence: upload.fence,
                bucket: "cartography-evidence".into(),
                key: "system-cartography/v1/stable-receipt.json".into(),
                content_type: upload.content_type,
                size_bytes: upload.size_bytes,
                sha256: upload.sha256,
                version_id: Some("stable-s3-version".into()),
                expires_at_ms: requested_at_ms + 60_000,
                status: EvidenceStatus::Verified,
            };
            let grant = EvidenceUploadGrant {
                authorization,
                upload_url: None,
                upload_headers: Vec::new(),
            };
            write_response(
                &mut authorize_stream,
                "200 OK",
                "application/json",
                &serde_json::to_vec(&grant).unwrap(),
            );
        }
    });

    let session = ScoutCartographySession::enroll(
        ScoutCartographySessionConfig::new(
            &base_url,
            "platform-test-key",
            "/v1/cartography",
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
    let mut references = Vec::new();
    for _ in 0..2 {
        references.push(
            session
                .upload_evidence_idempotent(
                    run_id,
                    source_id,
                    task_id,
                    7,
                    "receipt:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    requested_at_ms,
                    "application/json",
                    b"{\"safe\":\"metadata\"}",
                )
                .await
                .unwrap(),
        );
    }
    server.join().unwrap();

    assert_eq!(references[0], references[1]);
    assert_eq!(
        references[0].version_id.as_deref(),
        Some("stable-s3-version")
    );
}

fn read_request(stream: &mut TcpStream) -> Request {
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
    let mut request_line = head.lines().next().unwrap().split_whitespace();
    Request {
        method: request_line.next().unwrap().into(),
        path: request_line.next().unwrap().into(),
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
    stream.flush().unwrap();
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
