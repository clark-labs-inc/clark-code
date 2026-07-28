use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode, Url};
use scout_ingest_protocol::{IngestReceipt, IngestRequest};

use crate::CentralIngestTransport;

const INGEST_PATH: &str = "/v1/scout/enterprise-batches";
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RECEIPT_BODY_BYTES: usize = 256 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;
const MAX_ERROR_DETAIL_CHARS: usize = 512;

/// Private HTTP configuration for Scout central ingestion.
///
/// Deliberately does not implement `Debug` or serialization. The Platform API
/// key is stored only in a sensitive `Authorization` header value.
pub struct ReqwestTransportConfig {
    endpoint: Url,
    authorization: HeaderValue,
    request_timeout: Duration,
}

impl ReqwestTransportConfig {
    pub fn new(
        platform_base_url: impl AsRef<str>,
        platform_api_key: impl AsRef<str>,
    ) -> Result<Self, String> {
        let mut endpoint = Url::parse(platform_base_url.as_ref())
            .map_err(|_| "invalid Clark Platform base URL".to_string())?;
        validate_platform_url(&endpoint)?;
        endpoint.set_path(INGEST_PATH);
        endpoint.set_query(None);
        endpoint.set_fragment(None);

        let platform_api_key = platform_api_key.as_ref();
        if platform_api_key.is_empty() || platform_api_key.trim() != platform_api_key {
            return Err("Clark Platform API key must be a non-empty HTTP header value".into());
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {platform_api_key}"))
            .map_err(|_| "Clark Platform API key is not a valid HTTP header value".to_string())?;
        authorization.set_sensitive(true);

        Ok(Self {
            endpoint,
            authorization,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        })
    }

    pub fn with_request_timeout(mut self, request_timeout: Duration) -> Result<Self, String> {
        if request_timeout.is_zero() {
            return Err("Scout ingestion request timeout must be positive".into());
        }
        self.request_timeout = request_timeout;
        Ok(self)
    }
}

#[derive(Clone)]
pub struct ReqwestCentralIngestTransport {
    client: Client,
    endpoint: Url,
    authorization: HeaderValue,
}

impl ReqwestCentralIngestTransport {
    pub fn new(config: ReqwestTransportConfig) -> Result<Self, String> {
        let client = Client::builder()
            .connect_timeout(config.request_timeout.min(Duration::from_secs(10)))
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "failed to initialize Scout ingestion HTTP client".to_string())?;
        Ok(Self {
            client,
            endpoint: config.endpoint,
            authorization: config.authorization,
        })
    }
}

#[async_trait]
impl CentralIngestTransport for ReqwestCentralIngestTransport {
    async fn submit(&self, request: &IngestRequest) -> Result<IngestReceipt, String> {
        request.validate()?;
        let batch_id = request.bundle.signed_batch.batch.batch_id.as_str();
        let idempotency_key = HeaderValue::from_str(batch_id)
            .map_err(|_| "Scout batch id is not a valid Idempotency-Key value".to_string())?;

        let response = self
            .client
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .header("Idempotency-Key", idempotency_key)
            .header(ACCEPT, "application/json")
            .json(request)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();

        if !status.is_success() {
            return Err(http_status_error(status, response).await);
        }
        require_json_content_type(&response)?;
        let body = read_bounded_body(response, MAX_RECEIPT_BODY_BYTES, "receipt").await?;
        decode_strict_receipt(&body)
    }
}

fn validate_platform_url(endpoint: &Url) -> Result<(), String> {
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err("Clark Platform base URL must not contain credentials".into());
    }
    match endpoint.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_host(endpoint.host_str()) => Ok(()),
        _ => Err("Clark Platform base URL must use HTTPS".into()),
    }
}

fn is_loopback_host(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Scout central-ingestion request timed out".into()
    } else if error.is_connect() {
        "failed to connect to Scout central ingestion".into()
    } else {
        "Scout central-ingestion request failed".into()
    }
}

async fn http_status_error(status: StatusCode, response: reqwest::Response) -> String {
    match read_bounded_body(response, MAX_ERROR_BODY_BYTES, "error").await {
        Ok(body) => {
            let detail = bounded_error_detail(&body);
            if detail.is_empty() {
                format!("Scout central ingestion returned HTTP {status}")
            } else {
                format!("Scout central ingestion returned HTTP {status}: {detail}")
            }
        }
        Err(error) => format!("Scout central ingestion returned HTTP {status}: {error}"),
    }
}

fn require_json_content_type(response: &reqwest::Response) -> Result<(), String> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    match content_type {
        Some("application/json") => Ok(()),
        Some(value) if value.starts_with("application/") && value.ends_with("+json") => Ok(()),
        _ => Err("Scout central ingestion returned a non-JSON receipt".into()),
    }
}

async fn read_bounded_body(
    mut response: reqwest::Response,
    limit: usize,
    body_kind: &str,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!(
            "Scout central-ingestion {body_kind} body exceeds {limit} bytes"
        ));
    }

    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(limit);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| format!("failed to read Scout central-ingestion {body_kind} body"))?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(format!(
                "Scout central-ingestion {body_kind} body exceeds {limit} bytes"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn decode_strict_receipt(body: &[u8]) -> Result<IngestReceipt, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let mut unknown_fields = Vec::new();
    let receipt = serde_ignored::deserialize(&mut deserializer, |path| {
        unknown_fields.push(path.to_string());
    })
    .map_err(|error| format!("invalid Scout central-ingestion receipt JSON: {error}"))?;
    deserializer
        .end()
        .map_err(|error| format!("invalid Scout central-ingestion receipt JSON: {error}"))?;
    if let Some(field) = unknown_fields.first() {
        return Err(format!(
            "Scout central-ingestion receipt contains unknown field `{field}`"
        ));
    }
    Ok(receipt)
}

fn bounded_error_detail(body: &[u8]) -> String {
    String::from_utf8_lossy(body)
        .chars()
        .take(MAX_ERROR_DETAIL_CHARS)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}
