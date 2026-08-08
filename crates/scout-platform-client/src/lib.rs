//! Portable HTTPS client for a host-configured system-cartography backend.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Url};
use scout_ingest_protocol::cartography::{
    BatchAcceptance, BatchEnvelope, CartographyChangePage, CartographyChangeQuery,
    EvidenceCommitOutcome, EvidenceCommitRequest, EvidenceUploadGrant, EvidenceUploadRequest,
    GraphDeltaPage, GraphDeltaQuery, GraphSnapshotPage, GraphSnapshotQuery, SimulationOverlayPage,
    SimulationOverlayQuery, TaskClaimRequest, TaskClaimResponse,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

pub use enrollment::{
    enroll_machine, CartographyEnrollmentConfig, EnrolledCartographyClient, MachineEnrollment,
    MachineEnrollmentRequest,
};
pub use scout_machine_identity::CollectorMachineIdentity;
pub use session::{ScoutCartographySession, ScoutCartographySessionConfig};

mod enrollment;
mod http;
mod session;
use http::{
    authorization_header, build_http_client, http_error, post_json, request_error,
    validate_remote_url,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 16 * 1024;

/// Secret-bearing configuration for host-authoritative cartography API.
///
/// Deliberately does not implement `Debug`, `Clone`, or serialization.
pub struct CartographyClientConfig {
    base_url: Url,
    authorization: HeaderValue,
    coordinator_public_key: String,
    route_prefix: String,
    timeout: Duration,
}

impl CartographyClientConfig {
    pub fn new(
        platform_base_url: impl AsRef<str>,
        platform_api_key: impl AsRef<str>,
        coordinator_public_key: impl Into<String>,
        route_prefix: impl Into<String>,
    ) -> Result<Self, String> {
        let base_url = Url::parse(platform_base_url.as_ref())
            .map_err(|_| "invalid host platform base URL".to_string())?;
        validate_remote_url(&base_url, "host platform base URL")?;
        let authorization = authorization_header(platform_api_key.as_ref())?;
        let coordinator_public_key = coordinator_public_key.into();
        let route_prefix = validate_route_prefix(route_prefix.into())?;
        if coordinator_public_key.len() != 64
            || !coordinator_public_key
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("cartography coordinator key must be 64 hexadecimal characters".into());
        }
        Ok(Self {
            base_url,
            authorization,
            coordinator_public_key,
            route_prefix,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, String> {
        if timeout.is_zero() {
            return Err("cartography request timeout must be positive".into());
        }
        self.timeout = timeout;
        Ok(self)
    }
}

#[derive(Clone)]
pub struct CartographyClient {
    client: Client,
    base_url: Url,
    authorization: HeaderValue,
    coordinator_public_key: String,
    route_prefix: String,
}

impl CartographyClient {
    pub fn new(config: CartographyClientConfig) -> Result<Self, String> {
        let client = build_http_client(config.timeout)?;
        Ok(Self {
            client,
            base_url: config.base_url,
            authorization: config.authorization,
            coordinator_public_key: config.coordinator_public_key,
            route_prefix: config.route_prefix,
        })
    }

    pub async fn claim_task(
        &self,
        request: &TaskClaimRequest,
    ) -> Result<TaskClaimResponse, String> {
        self.post_json(&self.route("tasks/claim"), request).await
    }

    pub async fn authorize_evidence(
        &self,
        request: &EvidenceUploadRequest,
    ) -> Result<EvidenceUploadGrant, String> {
        let grant: EvidenceUploadGrant = self
            .post_json(&self.route("evidence/uploads"), request)
            .await?;
        let authorization = &grant.authorization;
        let lifecycle_matches = match authorization.status {
            scout_ingest_protocol::cartography::EvidenceStatus::Pending => {
                authorization.version_id.is_none()
                    && grant
                        .upload_url
                        .as_deref()
                        .is_some_and(|url| !url.trim().is_empty())
            }
            scout_ingest_protocol::cartography::EvidenceStatus::Verified => {
                authorization
                    .version_id
                    .as_deref()
                    .is_some_and(|version| !version.is_empty())
                    && grant.upload_url.is_none()
                    && grant.upload_headers.is_empty()
            }
            scout_ingest_protocol::cartography::EvidenceStatus::Rejected
            | scout_ingest_protocol::cartography::EvidenceStatus::Expired => false,
        };
        if authorization.evidence_id != request.evidence_id
            || authorization.organization_id != request.organization_id
            || authorization.workspace_id != request.workspace_id
            || authorization.run_id != request.run_id
            || authorization.source_id != request.source_id
            || authorization.machine_id != request.machine_id
            || authorization.task_id != request.task_id
            || authorization.fence != request.fence
            || authorization.content_type != request.content_type
            || authorization.size_bytes != request.size_bytes
            || authorization.sha256 != request.sha256
            || !lifecycle_matches
        {
            return Err("backend evidence authorization does not match the signed request".into());
        }
        Ok(grant)
    }

    pub async fn upload_evidence(
        &self,
        grant: &EvidenceUploadGrant,
        bytes: &[u8],
    ) -> Result<(), String> {
        let authorization = &grant.authorization;
        if authorization.status != scout_ingest_protocol::cartography::EvidenceStatus::Pending
            || authorization.version_id.is_some()
        {
            return Err("evidence upload requires a pending backend authorization".into());
        }
        if bytes.len() as u64 != authorization.size_bytes {
            return Err("evidence bytes do not match the backend-authorized size".into());
        }
        let digest = hex_lower(&Sha256::digest(bytes));
        if digest != authorization.sha256 {
            return Err("evidence bytes do not match the backend-authorized SHA-256".into());
        }
        if authorization.expires_at_ms <= now_ms()? {
            return Err("evidence upload authorization has expired".into());
        }
        let upload_url = grant
            .upload_url
            .as_deref()
            .ok_or_else(|| "pending evidence authorization has no upload URL".to_string())?;
        let url = Url::parse(upload_url)
            .map_err(|_| "backend returned an invalid evidence upload URL".to_string())?;
        validate_remote_url(&url, "evidence upload URL")?;
        let mut headers = HeaderMap::new();
        for header in &grant.upload_headers {
            let name = HeaderName::from_bytes(header.name.as_bytes())
                .map_err(|_| "backend returned an invalid evidence upload header".to_string())?;
            if matches!(
                name.as_str(),
                "authorization" | "cookie" | "proxy-authorization" | "host"
            ) {
                return Err("backend returned a forbidden evidence upload header".into());
            }
            let value = HeaderValue::from_str(&header.value)
                .map_err(|_| "backend returned an invalid evidence upload header".to_string())?;
            headers.insert(name, value);
        }
        let response = self
            .client
            .put(url)
            .headers(headers)
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|error| request_error(error, "evidence upload"))?;
        if !response.status().is_success() {
            return Err(http_error(response, "evidence upload").await);
        }
        Ok(())
    }

    pub async fn commit_evidence(
        &self,
        request: &EvidenceCommitRequest,
    ) -> Result<EvidenceCommitOutcome, String> {
        let outcome: EvidenceCommitOutcome = self
            .post_json(&self.route("evidence/commits"), request)
            .await?;
        let evidence = &outcome.evidence;
        if evidence.evidence_id != request.evidence_id
            || evidence.organization_id != request.organization_id
            || evidence.workspace_id != request.workspace_id
            || evidence.run_id != request.run_id
            || evidence.machine_id != request.machine_id
            || evidence.task_id != request.task_id
            || evidence.fence != request.fence
        {
            return Err("backend evidence commit does not match the signed request".into());
        }
        Ok(outcome)
    }

    pub async fn ingest_batch(&self, envelope: &BatchEnvelope) -> Result<BatchAcceptance, String> {
        let acceptance: BatchAcceptance = self.post_json(&self.route("batches"), envelope).await?;
        acceptance.receipt.verify(&self.coordinator_public_key)?;
        if acceptance.receipt.organization_id != envelope.organization_id
            || acceptance.receipt.workspace_id != envelope.workspace_id
            || acceptance.receipt.batch_id != envelope.batch_id
        {
            return Err("backend receipt does not acknowledge the submitted batch".into());
        }
        Ok(acceptance)
    }

    pub async fn query_snapshot(
        &self,
        query: &GraphSnapshotQuery,
    ) -> Result<GraphSnapshotPage, String> {
        let page: GraphSnapshotPage = self
            .post_json(&self.route("snapshots/query"), query)
            .await?;
        let expected_effective_at_ms = query
            .effective_at_ms
            .or_else(|| query.cursor.as_ref().map(|cursor| cursor.effective_at_ms));
        let expected_known_at_ms = query
            .known_at_ms
            .or_else(|| query.cursor.as_ref().map(|cursor| cursor.known_at_ms));
        if page.organization_id != query.organization_id
            || page.workspace_id != query.workspace_id
            || expected_effective_at_ms.is_some_and(|value| value != page.effective_at_ms)
            || expected_known_at_ms.is_some_and(|value| value != page.known_at_ms)
        {
            return Err("backend snapshot does not match the requested boundary".into());
        }
        Ok(page)
    }

    pub async fn query_delta(&self, query: &GraphDeltaQuery) -> Result<GraphDeltaPage, String> {
        let page: GraphDeltaPage = self.post_json(&self.route("deltas/query"), query).await?;
        let expected_to_effective_at_ms = query.to_effective_at_ms.or_else(|| {
            query
                .cursor
                .as_ref()
                .map(|cursor| cursor.to_effective_at_ms)
        });
        let expected_from_known_at_ms = query
            .from_known_at_ms
            .or_else(|| query.cursor.as_ref().map(|cursor| cursor.from_known_at_ms));
        let expected_to_known_at_ms = query
            .to_known_at_ms
            .or_else(|| query.cursor.as_ref().map(|cursor| cursor.to_known_at_ms));
        if page.organization_id != query.organization_id
            || page.workspace_id != query.workspace_id
            || page.from_snapshot.organization_id != query.organization_id
            || page.from_snapshot.workspace_id != query.workspace_id
            || page.from_snapshot.effective_at_ms != query.from_effective_at_ms
            || expected_from_known_at_ms
                .is_some_and(|value| value != page.from_snapshot.known_at_ms)
            || expected_to_effective_at_ms
                .is_some_and(|value| value != page.to_snapshot.effective_at_ms)
            || expected_to_known_at_ms.is_some_and(|value| value != page.to_snapshot.known_at_ms)
        {
            return Err("backend delta does not match the requested temporal boundary".into());
        }
        Ok(page)
    }

    pub async fn query_simulation_overlay(
        &self,
        query: &SimulationOverlayQuery,
    ) -> Result<SimulationOverlayPage, String> {
        let page: SimulationOverlayPage = self
            .post_json(&self.route("simulation-overlays/query"), query)
            .await?;
        if page.overlay.organization_id != query.organization_id
            || page.overlay.workspace_id != query.workspace_id
            || page.overlay.stable_key != query.stable_key
            || query
                .version
                .is_some_and(|version| version != page.overlay.version)
            || query.cursor.as_ref().is_some_and(|cursor| {
                cursor.simulation_id != page.overlay.id
                    || cursor.content_sha256 != page.overlay.content_sha256
            })
        {
            return Err("backend simulation overlay does not match the requested identity".into());
        }
        Ok(page)
    }

    pub async fn query_changes(
        &self,
        query: &CartographyChangeQuery,
    ) -> Result<CartographyChangePage, String> {
        let page: CartographyChangePage =
            self.post_json(&self.route("changes/query"), query).await?;
        let expected_next = page
            .changes
            .last()
            .map(|change| change.sequence)
            .unwrap_or(query.after_sequence);
        if page.organization_id != query.organization_id
            || page.workspace_id != query.workspace_id
            || page.changes.iter().any(|change| {
                change.organization_id != query.organization_id
                    || change.workspace_id != query.workspace_id
                    || change.sequence <= query.after_sequence
            })
            || page
                .changes
                .windows(2)
                .any(|pair| pair[0].sequence >= pair[1].sequence)
            || page.next_after_sequence != expected_next
        {
            return Err("backend change page does not match the requested boundary".into());
        }
        Ok(page)
    }

    async fn post_json<Request, Response>(
        &self,
        path: &str,
        request: &Request,
    ) -> Result<Response, String>
    where
        Request: Serialize + ?Sized,
        Response: DeserializeOwned,
    {
        post_json(
            &self.client,
            &self.base_url,
            &self.authorization,
            path,
            request,
        )
        .await
    }

    fn route(&self, suffix: &str) -> String {
        format!("{}/{suffix}", self.route_prefix)
    }
}

fn validate_route_prefix(prefix: String) -> Result<String, String> {
    let prefix = prefix.trim_end_matches('/').to_string();
    if !prefix.starts_with('/')
        || prefix.len() < 2
        || prefix.contains('?')
        || prefix.contains('#')
        || prefix.contains("..")
    {
        return Err("cartography route prefix must be an absolute URL path".into());
    }
    Ok(prefix)
}

fn now_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|_| "system clock precedes Unix time".into())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use scout_ingest_protocol::cartography::{
        EvidenceStatus, EvidenceUploadAuthorization, EvidenceUploadGrant,
    };
    use uuid::Uuid;

    use super::{CartographyClient, CartographyClientConfig};

    fn client() -> CartographyClient {
        CartographyClient::new(
            CartographyClientConfig::new(
                "http://127.0.0.1:9",
                "test-key",
                "ab".repeat(32),
                "/v1/cartography",
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn upload_fails_locally_before_network_when_digest_is_wrong() {
        let grant = EvidenceUploadGrant {
            authorization: EvidenceUploadAuthorization {
                evidence_id: format!("evidence:{}", "a".repeat(64)),
                organization_id: Uuid::nil(),
                workspace_id: Uuid::nil(),
                run_id: Uuid::nil(),
                source_id: Uuid::nil(),
                machine_id: Uuid::nil(),
                task_id: Uuid::nil(),
                fence: 1,
                bucket: "test-bucket".into(),
                key: "test-key".into(),
                content_type: "application/zstd".into(),
                size_bytes: 3,
                sha256: "ab".repeat(32),
                version_id: None,
                expires_at_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64
                    + 60_000,
                status: EvidenceStatus::Pending,
            },
            upload_url: Some("http://127.0.0.1:9/upload".into()),
            upload_headers: Vec::new(),
        };
        assert!(client().upload_evidence(&grant, b"bad").await.is_err());
    }

    #[test]
    fn remote_http_and_embedded_credentials_are_rejected() {
        assert!(CartographyClientConfig::new(
            "http://example.com",
            "test-key",
            "ab".repeat(32),
            "/v1/cartography",
        )
        .is_err());
        assert!(CartographyClientConfig::new(
            "https://user:password@example.com",
            "test-key",
            "ab".repeat(32),
            "/v1/cartography",
        )
        .is_err());
    }
}
