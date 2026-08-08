use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseBatchBundle, EnterpriseEntityKind, EnterpriseEvent,
    EnterpriseFact, EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseTrustChain, EnterpriseTrustManifest, GraphEntityObservation,
};
use scout_coordinator::{
    CoordinatorStore, HostedIngestConfig, HostedIngestServer, TenantAuthenticator, TenantBearerAuth,
};
use scout_ingest_protocol::{CoordinatorSigningKey, IngestReceipt, IngestRequest, ScoutTenantId};

const TOKEN: &str = "ck_live_hosted_ingest_test_only";

struct Fixture {
    tenant_id: ScoutTenantId,
    enterprise_id: EnterpriseId,
    chain: EnterpriseTrustChain,
    grant: EnterpriseSignerGrant,
    collector: EnterpriseSigningKey,
}

impl Fixture {
    fn new(enterprise: &str, tenant: &str) -> Self {
        let enterprise_id = EnterpriseId::new(enterprise).unwrap();
        let administrator = EnterpriseSigningKey::from_seed([7; 32]);
        let collector = EnterpriseSigningKey::from_seed([8; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise_id.clone(),
            format!("trust:{}", "a".repeat(64)),
            100,
            100_000,
            &administrator,
        )
        .unwrap();
        let grant = EnterpriseSignerGrant::issue(
            &manifest,
            collector.signer_id(),
            collector.public_key_hex(),
            BTreeSet::from([EnterpriseSignerRole::Collector]),
            EnterpriseGrantScope {
                machine_id: "machine-a".into(),
                run_id: "run-a".into(),
                adapter_instance_id: "aws-prod".into(),
                auth_context_id: "auth-read-only".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: 1,
                last_source_sequence: 10_000,
            },
            100,
            90_000,
            &[&administrator],
        )
        .unwrap();
        Self {
            tenant_id: ScoutTenantId::new(tenant).unwrap(),
            enterprise_id,
            chain: EnterpriseTrustChain {
                anchor_manifest_id: manifest.manifest_id.clone(),
                manifests: vec![manifest],
            },
            grant,
            collector,
        }
    }

    fn request(&self, sequence: u64) -> IngestRequest {
        let observation = GraphEntityObservation::new(
            &self.enterprise_id,
            EnterpriseEntityKind::Service,
            AuthorityRef::new(
                "aws",
                "account:prod",
                format!("service:checkout-{sequence}"),
            )
            .unwrap(),
            BTreeSet::from([format!("checkout-{sequence}")]),
            BTreeSet::from([format!("{sequence:064x}")]),
        )
        .unwrap();
        let event = EnterpriseEvent::new(
            self.enterprise_id.clone(),
            EnterpriseProvenance {
                machine_id: "machine-a".into(),
                run_id: "run-a".into(),
                adapter_instance_id: "aws-prod".into(),
                auth_context_id: "auth-read-only".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                source_sequence: sequence,
                observed_at_ms: 1_000 + sequence,
                source_fingerprint: format!("{:064x}", sequence + 10_000),
            },
            EnterpriseFact::EntityObserved(observation),
        )
        .unwrap();
        let batch = EnterpriseBatch::new(self.enterprise_id.clone(), [event]).unwrap();
        let signed_batch = EnterpriseSignedBatch::sign(
            batch,
            &self.chain.manifests[0],
            self.grant.clone(),
            1_000 + sequence,
            &self.collector,
        )
        .unwrap();
        IngestRequest::new(
            self.tenant_id.clone(),
            format!("outbox-attempt:{:064x}", sequence + 20_000),
            EnterpriseBatchBundle {
                trust_chain: self.chain.clone(),
                signed_batch,
            },
        )
        .unwrap()
    }
}

struct ServerHarness {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    root: tempfile::TempDir,
    store: CoordinatorStore,
}

impl ServerHarness {
    fn start(fixture: &Fixture, config: HostedIngestConfig) -> Self {
        let auth = TenantBearerAuth::new([(fixture.tenant_id.clone(), TOKEN.to_owned())]).unwrap();
        Self::start_with_auth(fixture, config, auth)
    }

    fn start_with_auth<A>(fixture: &Fixture, config: HostedIngestConfig, auth: A) -> Self
    where
        A: TenantAuthenticator + 'static,
    {
        let root = tempfile::tempdir().unwrap();
        let store = CoordinatorStore::open(root.path(), CoordinatorSigningKey::from_seed([42; 32]))
            .unwrap();
        store
            .pin_enterprise(
                &fixture.tenant_id,
                &fixture.enterprise_id,
                &fixture.chain.anchor_manifest_id,
                &fixture.chain,
            )
            .unwrap();
        let server = HostedIngestServer::bind("127.0.0.1:0", store.clone(), auth, config).unwrap();
        let address = server.local_addr().unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let child_shutdown = shutdown.clone();
        let thread = thread::spawn(move || server.serve_until(&child_shutdown).unwrap());
        Self {
            address,
            shutdown,
            thread: Some(thread),
            root,
            store,
        }
    }
}

#[derive(Clone)]
struct ClarkOrganizationAuth {
    tenant_id: ScoutTenantId,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl TenantAuthenticator for ClarkOrganizationAuth {
    fn authenticate_bearer(&self, token: &str) -> Option<ScoutTenantId> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        (token == TOKEN).then(|| self.tenant_id.clone())
    }
}

impl Drop for ServerHarness {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

struct Response {
    status: u16,
    headers: String,
    body: Vec<u8>,
}

fn exchange(address: SocketAddr, request: &[u8]) -> Response {
    decode_response(exchange_raw(address, request))
}

fn exchange_raw(address: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(request).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = Vec::new();
    if let Err(error) = stream.read_to_end(&mut response) {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    response
}

fn decode_response(response: Vec<u8>) -> Response {
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let headers = String::from_utf8(response[..split].to_vec()).unwrap();
    let status = headers
        .lines()
        .next()
        .unwrap()
        .split_ascii_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    Response {
        status,
        headers,
        body: response[split + 4..].to_vec(),
    }
}

fn post_request(request: &IngestRequest, token: &str, idempotency_key: &str) -> Vec<u8> {
    let body = serde_json::to_vec(request).unwrap();
    format!(
        "POST /v1/scout/enterprise-batches HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nIdempotency-Key: {idempotency_key}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body)
    .collect()
}

fn get_request(path: &str, token: &str) -> Vec<u8> {
    format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n"
    )
    .into_bytes()
}

#[test]
fn real_http_boundary_ingests_replays_and_reads_tenant_state() {
    let fixture = Fixture::new("enterprise-acme", "organization:acme");
    let server = ServerHarness::start(&fixture, HostedIngestConfig::default());
    let request = fixture.request(1);
    let batch_id = request.bundle.signed_batch.batch.batch_id.to_string();
    let raw = post_request(&request, TOKEN, &batch_id);

    let accepted = exchange(server.address, &raw);
    assert_eq!(accepted.status, 200);
    assert!(accepted.headers.contains("Cache-Control: no-store"));
    assert!(!String::from_utf8_lossy(&accepted.body).contains(TOKEN));
    let receipt: IngestReceipt = serde_json::from_slice(&accepted.body).unwrap();
    receipt
        .verify(&server.store.coordinator_public_key())
        .unwrap();
    assert_eq!(receipt.batch_id.as_str(), batch_id);

    let replayed = exchange(server.address, &raw);
    assert_eq!(replayed.status, 200);
    assert_eq!(replayed.body, accepted.body);

    let receipt_path = format!(
        "/v1/scout/enterprises/{}/batches/{batch_id}/receipt",
        fixture.enterprise_id
    );
    let fetched = exchange(server.address, &get_request(&receipt_path, TOKEN));
    assert_eq!(fetched.status, 200);
    assert_eq!(fetched.body, accepted.body);

    let status_path = format!("/v1/scout/enterprises/{}/status", fixture.enterprise_id);
    let status = exchange(server.address, &get_request(&status_path, TOKEN));
    assert_eq!(status.status, 200);
    let value: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(value["tenant_id"], fixture.tenant_id.as_str());
    assert_eq!(value["accepted_batches"], 1);
    assert_eq!(value["next_sequence"], 2);
    assert_eq!(
        server
            .store
            .status(&fixture.tenant_id, &fixture.enterprise_id)
            .unwrap()
            .unwrap()
            .accepted_batches,
        1
    );
    assert!(server
        .root
        .path()
        .join("scout-coordinator.sqlite3")
        .is_file());
}

#[test]
fn authoritative_organization_auth_is_injectable_and_plaintext_is_loopback_only() {
    let fixture = Fixture::new("enterprise-acme", "organization:acme");
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let auth = ClarkOrganizationAuth {
        tenant_id: fixture.tenant_id.clone(),
        calls: calls.clone(),
    };
    let server =
        ServerHarness::start_with_auth(&fixture, HostedIngestConfig::default(), auth.clone());
    let request = fixture.request(1);
    let batch_id = request.bundle.signed_batch.batch.batch_id.to_string();

    assert_eq!(
        exchange(server.address, &post_request(&request, TOKEN, &batch_id)).status,
        200
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);

    let error = match HostedIngestServer::bind(
        "0.0.0.0:0",
        server.store.clone(),
        auth,
        HostedIngestConfig::default(),
    ) {
        Ok(_) => panic!("plaintext non-loopback listener unexpectedly accepted"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn authentication_tenant_and_idempotency_are_enforced_without_echo() {
    let fixture = Fixture::new("enterprise-acme", "organization:acme");
    let server = ServerHarness::start(&fixture, HostedIngestConfig::default());
    let request = fixture.request(1);
    let batch_id = request.bundle.signed_batch.batch.batch_id.to_string();

    let wrong_token = "secret-token-that-must-never-be-returned";
    let unauthorized = exchange(
        server.address,
        &post_request(&request, wrong_token, &batch_id),
    );
    assert_eq!(unauthorized.status, 401);
    assert!(unauthorized.headers.contains("WWW-Authenticate: Bearer"));
    assert!(!String::from_utf8_lossy(&unauthorized.body).contains(wrong_token));

    let other = Fixture::new("enterprise-other", "organization:other");
    let other_request = other.request(2);
    let forbidden = exchange(
        server.address,
        &post_request(
            &other_request,
            TOKEN,
            other_request.bundle.signed_batch.batch.batch_id.as_str(),
        ),
    );
    assert_eq!(forbidden.status, 403);

    let mismatch = exchange(
        server.address,
        &post_request(&request, TOKEN, &format!("batch:{}", "f".repeat(64))),
    );
    assert_eq!(mismatch.status, 409);
    assert_eq!(
        server
            .store
            .status(&fixture.tenant_id, &fixture.enterprise_id)
            .unwrap()
            .unwrap()
            .accepted_batches,
        0
    );
}

#[test]
fn malformed_oversized_and_slow_requests_fail_at_the_socket_boundary() {
    let fixture = Fixture::new("enterprise-acme", "organization:acme");
    let config = HostedIngestConfig::default()
        .with_request_timeout(Duration::from_millis(75))
        .unwrap();
    let server = ServerHarness::start(&fixture, config);

    let canary = "bearer-canary-must-not-echo";
    let malformed_body = format!("{{\"secret\":\"{canary}\"}}");
    let malformed = format!(
        "POST /v1/scout/enterprise-batches HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nIdempotency-Key: batch:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{malformed_body}",
        "a".repeat(64),
        malformed_body.len()
    );
    let invalid_json = exchange(server.address, malformed.as_bytes());
    assert_eq!(invalid_json.status, 400);
    assert!(!String::from_utf8_lossy(&invalid_json.body).contains(canary));

    let oversized = format!(
        "POST /v1/scout/enterprise-batches HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nIdempotency-Key: batch:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        "a".repeat(64),
        16 * 1024 * 1024 + 1
    );
    assert_eq!(exchange(server.address, oversized.as_bytes()).status, 413);

    let mut stream = TcpStream::connect(server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "POST /v1/scout/enterprise-batches HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nIdempotency-Key: batch:{}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{}}",
        "a".repeat(64)
    )
    .unwrap();
    let mut timeout_response = Vec::new();
    stream.read_to_end(&mut timeout_response).unwrap();
    assert_eq!(decode_response(timeout_response).status, 408);
}

#[test]
fn header_and_connection_limits_are_enforced_before_work_is_admitted() {
    let fixture = Fixture::new("enterprise-acme", "organization:acme");
    let config = HostedIngestConfig::default()
        .with_request_timeout(Duration::from_secs(1))
        .unwrap()
        .with_max_concurrent_connections(1)
        .unwrap();
    let server = ServerHarness::start(&fixture, config);

    let missing_host = format!(
        "GET /v1/scout/enterprises/{}/status HTTP/1.1\r\nAuthorization: Bearer {TOKEN}\r\n\r\n",
        fixture.enterprise_id
    );
    assert_eq!(
        exchange(server.address, missing_host.as_bytes()).status,
        400
    );
    let ambiguous_header = format!(
        "GET /v1/scout/enterprises/{}/status HTTP/1.1\r\nHost: localhost\r\nX_Forwarded_Host: attacker.test\r\nAuthorization: Bearer {TOKEN}\r\n\r\n",
        fixture.enterprise_id
    );
    assert_eq!(
        exchange(server.address, ambiguous_header.as_bytes()).status,
        400
    );

    let oversized_header = format!(
        "GET /v1/scout/enterprises/{}/status HTTP/1.1\r\nHost: localhost\r\nX-Padding: {}\r\n\r\n",
        fixture.enterprise_id,
        "x".repeat(33 * 1024)
    );
    let limited = exchange_raw(server.address, oversized_header.as_bytes());
    if limited.windows(4).any(|window| window == b"\r\n\r\n") {
        assert_eq!(decode_response(limited).status, 431);
    } else if !limited.is_empty() {
        assert!(b"HTTP/1.1 431".starts_with(&limited) || limited.starts_with(b"HTTP/1.1 431"));
    }
    thread::sleep(Duration::from_millis(100));

    let mut blocked = TcpStream::connect(server.address).unwrap();
    write!(
        blocked,
        "POST /v1/scout/enterprise-batches HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nIdempotency-Key: batch:{}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{}}",
        "a".repeat(64)
    )
    .unwrap();
    thread::sleep(Duration::from_millis(30));

    let status_path = format!("/v1/scout/enterprises/{}/status", fixture.enterprise_id);
    let mut observed_capacity_rejection = false;
    for _ in 0..20 {
        let saturated = exchange(server.address, &get_request(&status_path, TOKEN));
        if saturated.status == 503 {
            observed_capacity_rejection = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(observed_capacity_rejection);
    drop(blocked);
}

#[test]
fn auth_registry_rejects_unsafe_or_ambiguous_configuration() {
    let tenant = ScoutTenantId::new("organization:acme").unwrap();
    assert!(TenantBearerAuth::new(Vec::<(ScoutTenantId, String)>::new()).is_err());
    assert!(TenantBearerAuth::new([(tenant.clone(), "short")]).is_err());
    assert!(TenantBearerAuth::new([
        (tenant.clone(), TOKEN),
        (ScoutTenantId::new("organization:other").unwrap(), TOKEN),
    ])
    .is_err());
    assert!(HostedIngestConfig::default()
        .with_request_timeout(Duration::from_nanos(1))
        .is_err());
}
