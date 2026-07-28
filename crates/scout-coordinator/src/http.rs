mod wire;

use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agent_orchestration::{EnterpriseBatchId, EnterpriseId};
use scout_ingest_protocol::{IngestRequest, ScoutTenantId};
use sha2::{Digest, Sha256};

use crate::CoordinatorStore;
use wire::{read_body, read_request_head, write_response, HttpResponse, RequestHead};

pub const DEFAULT_MAX_INGEST_BODY_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_HEADER_BYTES: usize = 32 * 1024;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_CONCURRENT_CONNECTIONS: usize = 64;
const INGEST_PATH: &str = "/v1/scout/enterprise-batches";

#[derive(Clone)]
pub struct HostedIngestConfig {
    max_body_bytes: usize,
    max_header_bytes: usize,
    request_timeout: Duration,
    max_concurrent_connections: usize,
}

impl Default for HostedIngestConfig {
    fn default() -> Self {
        Self {
            max_body_bytes: DEFAULT_MAX_INGEST_BODY_BYTES,
            max_header_bytes: DEFAULT_MAX_HEADER_BYTES,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_concurrent_connections: DEFAULT_MAX_CONCURRENT_CONNECTIONS,
        }
    }
}

impl HostedIngestConfig {
    pub fn with_max_body_bytes(mut self, value: usize) -> Result<Self, String> {
        if value == 0 || value > 64 * 1024 * 1024 {
            return Err("Scout hosted-ingest body limit must be 1..=67108864 bytes".into());
        }
        self.max_body_bytes = value;
        Ok(self)
    }

    pub fn with_request_timeout(mut self, value: Duration) -> Result<Self, String> {
        if value < Duration::from_millis(1) || value > Duration::from_secs(120) {
            return Err("Scout hosted-ingest timeout must be 1ms..=120s".into());
        }
        self.request_timeout = value;
        Ok(self)
    }

    pub fn with_max_concurrent_connections(mut self, value: usize) -> Result<Self, String> {
        if value == 0 || value > 4096 {
            return Err("Scout hosted-ingest concurrency must be 1..=4096".into());
        }
        self.max_concurrent_connections = value;
        Ok(self)
    }
}

struct TokenEntry {
    digest: [u8; 32],
    tenant_id: ScoutTenantId,
}

/// Tenant authentication material for the hosted ingestion boundary.
///
/// Plaintext bearer tokens are hashed during construction and are never kept,
/// serialized, formatted, or returned in an HTTP response.
#[derive(Clone)]
pub struct TenantBearerAuth {
    entries: Arc<Vec<TokenEntry>>,
}

impl TenantBearerAuth {
    pub fn new<I, S>(tokens: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = (ScoutTenantId, S)>,
        S: AsRef<str>,
    {
        let mut entries = Vec::<TokenEntry>::new();
        for (tenant_id, token) in tokens {
            let token = token.as_ref();
            if token.len() < 16
                || token.len() > 512
                || token.trim() != token
                || token.bytes().any(|byte| byte.is_ascii_control())
            {
                return Err(
                    "Scout hosted-ingest bearer token must be 16..=512 visible bytes".into(),
                );
            }
            let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
            if entries
                .iter()
                .any(|entry| constant_time_eq(&entry.digest, &digest))
            {
                return Err("Scout hosted-ingest bearer tokens must be unique".into());
            }
            entries.push(TokenEntry { digest, tenant_id });
        }
        if entries.is_empty() {
            return Err("Scout hosted ingestion requires at least one tenant token".into());
        }
        Ok(Self {
            entries: Arc::new(entries),
        })
    }
}

/// Maps one already-parsed bearer token to Clark's authoritative tenant.
///
/// Implementations sit inside the trusted host boundary. They must neither
/// persist nor log the token and must fail closed when the token is expired,
/// revoked, or lacks the exact Scout organization scope.
pub trait TenantAuthenticator: Send + Sync {
    fn authenticate_bearer(&self, token: &str) -> Option<ScoutTenantId>;
}

impl TenantAuthenticator for TenantBearerAuth {
    fn authenticate_bearer(&self, token: &str) -> Option<ScoutTenantId> {
        if token.is_empty() {
            return None;
        }
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut matched = None;
        for entry in self.entries.iter() {
            if constant_time_eq(&entry.digest, &digest) {
                matched = Some(entry.tenant_id.clone());
            }
        }
        matched
    }
}

#[derive(Clone)]
struct Shared {
    store: CoordinatorStore,
    auth: Arc<dyn TenantAuthenticator>,
    config: HostedIngestConfig,
    active: Arc<AtomicUsize>,
}

/// Minimal loopback HTTP/1.1 boundary intended to sit behind Clark's TLS ingress.
///
/// It deliberately has no TLS or deployment configuration of its own. The
/// boundary owns authentication, request limits, tenant isolation, and exact
/// idempotency before calling [`CoordinatorStore`]. Binding a plaintext
/// non-loopback listener is rejected so callers cannot accidentally bypass
/// the TLS ingress.
pub struct HostedIngestServer {
    listener: TcpListener,
    shared: Shared,
}

impl HostedIngestServer {
    pub fn bind<A>(
        address: impl ToSocketAddrs,
        store: CoordinatorStore,
        auth: A,
        config: HostedIngestConfig,
    ) -> io::Result<Self>
    where
        A: TenantAuthenticator + 'static,
    {
        let listener = TcpListener::bind(address)?;
        if !listener.local_addr()?.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Scout plaintext ingestion may bind only to loopback behind TLS ingress",
            ));
        }
        Ok(Self {
            listener,
            shared: Shared {
                store,
                auth: Arc::new(auth),
                config,
                active: Arc::new(AtomicUsize::new(0)),
            },
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn serve(self) -> io::Result<()> {
        self.serve_until(&AtomicBool::new(false))
    }

    /// Serve until `shutdown` becomes true. This is primarily useful for a
    /// managed process supervisor and real-socket boundary tests.
    pub fn serve_until(self, shutdown: &AtomicBool) -> io::Result<()> {
        self.listener.set_nonblocking(true)?;
        while !shutdown.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok((stream, _)) => self.dispatch(stream),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn dispatch(&self, mut stream: TcpStream) {
        if stream.set_nonblocking(false).is_err() {
            return;
        }
        let active = self.shared.active.fetch_add(1, Ordering::AcqRel);
        if active >= self.shared.config.max_concurrent_connections {
            self.shared.active.fetch_sub(1, Ordering::AcqRel);
            let _ = write_response(&mut stream, HttpResponse::service_unavailable());
            return;
        }
        let shared = self.shared.clone();
        thread::spawn(move || {
            let _ = handle_connection(&mut stream, &shared);
            shared.active.fetch_sub(1, Ordering::AcqRel);
        });
    }
}

#[derive(Clone, Copy)]
enum Route {
    Ingest,
    Receipt,
    Status,
}

fn handle_connection(stream: &mut TcpStream, shared: &Shared) -> io::Result<()> {
    let deadline = Instant::now() + shared.config.request_timeout;
    let response = match read_request_head(stream, &shared.config, deadline) {
        Ok(head) => dispatch_request(stream, shared, deadline, head),
        Err(problem) => HttpResponse::problem(problem),
    };
    write_response(stream, response)
}

fn dispatch_request(
    stream: &mut TcpStream,
    shared: &Shared,
    deadline: Instant,
    head: RequestHead,
) -> HttpResponse {
    let route = match (head.method.as_str(), head.path.as_str()) {
        ("POST", INGEST_PATH) => Route::Ingest,
        ("GET", path) if path.ends_with("/receipt") => Route::Receipt,
        ("GET", path) if path.ends_with("/status") => Route::Status,
        ("POST" | "GET", _) => return HttpResponse::not_found(),
        _ => return HttpResponse::method_not_allowed(),
    };
    let Some(tenant_id) = head.headers.get("authorization").and_then(|value| {
        value
            .strip_prefix("Bearer ")
            .and_then(|token| shared.auth.authenticate_bearer(token))
    }) else {
        return HttpResponse::unauthorized();
    };

    match route {
        Route::Ingest => ingest(stream, shared, deadline, head, tenant_id),
        Route::Receipt => {
            if request_has_body(&head) {
                return HttpResponse::bad_request("request_body_not_allowed");
            }
            get_receipt(shared, &tenant_id, &head.path)
        }
        Route::Status => {
            if request_has_body(&head) {
                return HttpResponse::bad_request("request_body_not_allowed");
            }
            get_status(shared, &tenant_id, &head.path)
        }
    }
}

fn ingest(
    stream: &mut TcpStream,
    shared: &Shared,
    deadline: Instant,
    head: RequestHead,
    tenant_id: ScoutTenantId,
) -> HttpResponse {
    if head.headers.contains_key("transfer-encoding") {
        return HttpResponse::bad_request("transfer_encoding_not_supported");
    }
    let Some(content_length) = head
        .headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return HttpResponse::length_required();
    };
    if content_length > shared.config.max_body_bytes {
        return HttpResponse::payload_too_large();
    }
    if !head
        .headers
        .get("content-type")
        .is_some_and(|value| is_json_content_type(value))
    {
        return HttpResponse::unsupported_media_type();
    }
    let Some(idempotency_key) = head.headers.get("idempotency-key") else {
        return HttpResponse::bad_request("idempotency_key_required");
    };
    let body = match read_body(
        stream,
        head.initial_body,
        content_length,
        deadline,
        shared.config.max_body_bytes,
    ) {
        Ok(body) => body,
        Err(problem) => return HttpResponse::problem(problem),
    };
    let request: IngestRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return HttpResponse::bad_request("invalid_ingest_json"),
    };
    if request.tenant_id != tenant_id {
        return HttpResponse::forbidden();
    }
    if idempotency_key != request.bundle.signed_batch.batch.batch_id.as_str() {
        return HttpResponse::conflict("idempotency_key_mismatch");
    }
    let observed_at_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(_) => return HttpResponse::internal_error(),
    };
    match shared.store.ingest(&tenant_id, &request, observed_at_ms) {
        Ok(receipt) => HttpResponse::json(200, "OK", &receipt),
        Err(_) => HttpResponse::conflict("ingest_rejected"),
    }
}

fn get_receipt(shared: &Shared, tenant_id: &ScoutTenantId, path: &str) -> HttpResponse {
    let Some((enterprise_id, batch_id)) = parse_receipt_path(path) else {
        return HttpResponse::not_found();
    };
    match shared
        .store
        .receipt(tenant_id, &enterprise_id, batch_id.as_str())
    {
        Ok(Some(receipt)) => HttpResponse::json(200, "OK", &receipt),
        Ok(None) => HttpResponse::not_found(),
        Err(_) => HttpResponse::internal_error(),
    }
}

fn get_status(shared: &Shared, tenant_id: &ScoutTenantId, path: &str) -> HttpResponse {
    let Some(enterprise_id) = parse_status_path(path) else {
        return HttpResponse::not_found();
    };
    match shared.store.status(tenant_id, &enterprise_id) {
        Ok(Some(status)) => HttpResponse::json_value(
            200,
            "OK",
            serde_json::json!({
                "tenant_id": status.tenant_id,
                "enterprise_id": status.enterprise_id,
                "anchor_manifest_id": status.anchor_manifest_id,
                "trust_generation": status.trust_generation,
                "accepted_batches": status.accepted_batches,
                "batch_accumulator_root": status.batch_accumulator_root,
                "next_sequence": status.next_sequence,
                "last_receipt_id": status.last_receipt_id,
            }),
        ),
        Ok(None) => HttpResponse::not_found(),
        Err(_) => HttpResponse::internal_error(),
    }
}

fn parse_receipt_path(path: &str) -> Option<(EnterpriseId, EnterpriseBatchId)> {
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() != 8
        || segments[..4] != ["", "v1", "scout", "enterprises"]
        || segments[5] != "batches"
        || segments[7] != "receipt"
    {
        return None;
    }
    Some((
        EnterpriseId::new(segments[4]).ok()?,
        EnterpriseBatchId::new(segments[6]).ok()?,
    ))
}

fn parse_status_path(path: &str) -> Option<EnterpriseId> {
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() != 6
        || segments[..4] != ["", "v1", "scout", "enterprises"]
        || segments[5] != "status"
    {
        return None;
    }
    EnterpriseId::new(segments[4]).ok()
}

fn request_has_body(head: &RequestHead) -> bool {
    !head.initial_body.is_empty()
        || head
            .headers
            .get("content-length")
            .is_some_and(|value| value != "0")
        || head.headers.contains_key("transfer-encoding")
}

fn is_json_content_type(value: &str) -> bool {
    let media_type = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media_type == "application/json"
        || (media_type.starts_with("application/") && media_type.ends_with("+json"))
}

fn unix_time_ms() -> Result<u64, String> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock precedes Unix epoch")?
        .as_millis();
    u64::try_from(milliseconds).map_err(|_| "system clock exceeds Scout timestamp range".into())
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
