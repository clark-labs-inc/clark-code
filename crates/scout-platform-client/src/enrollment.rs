use std::time::Duration;

use reqwest::header::HeaderValue;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::http::{authorization_header, build_http_client, post_json, validate_remote_url};
use super::{hex_lower, validate_route_prefix, CartographyClient, DEFAULT_TIMEOUT};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineEnrollmentRequest {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub public_key: String,
    pub platform: String,
    pub architecture: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineEnrollment {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub signer_id: String,
    pub public_key: String,
    pub platform: String,
    pub architecture: String,
    pub coordinator_public_key: String,
}

/// Secret-bearing enrollment configuration.
///
/// Enrollment is deliberately separate from collection because it requires
/// active organization-administrator authority and returns the coordinator
/// key that pins all later acceptance-receipt verification.
pub struct CartographyEnrollmentConfig {
    pub(super) base_url: Url,
    authorization: HeaderValue,
    timeout: Duration,
    route_prefix: String,
}

impl CartographyEnrollmentConfig {
    pub fn new(
        platform_base_url: impl AsRef<str>,
        platform_api_key: impl AsRef<str>,
        route_prefix: impl Into<String>,
    ) -> Result<Self, String> {
        let base_url = Url::parse(platform_base_url.as_ref())
            .map_err(|_| "invalid host platform base URL".to_string())?;
        validate_remote_url(&base_url, "host platform base URL")?;
        let authorization = authorization_header(platform_api_key.as_ref())?;
        let route_prefix = validate_route_prefix(route_prefix.into())?;
        Ok(Self {
            base_url,
            authorization,
            timeout: DEFAULT_TIMEOUT,
            route_prefix,
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

pub struct EnrolledCartographyClient {
    pub enrollment: MachineEnrollment,
    pub client: CartographyClient,
}

pub async fn enroll_machine(
    config: CartographyEnrollmentConfig,
    request: &MachineEnrollmentRequest,
) -> Result<EnrolledCartographyClient, String> {
    validate_request(request)?;
    let http_client = build_http_client(config.timeout)?;
    let enrollment: MachineEnrollment = post_json(
        &http_client,
        &config.base_url,
        &config.authorization,
        &format!("{}/machines/enroll", config.route_prefix),
        request,
    )
    .await?;
    validate_response(request, &enrollment)?;
    Ok(EnrolledCartographyClient {
        client: CartographyClient {
            client: http_client,
            base_url: config.base_url,
            authorization: config.authorization,
            coordinator_public_key: enrollment.coordinator_public_key.clone(),
            route_prefix: config.route_prefix,
        },
        enrollment,
    })
}

fn validate_request(request: &MachineEnrollmentRequest) -> Result<(), String> {
    if request.organization_id.is_nil() || request.workspace_id.is_nil() {
        return Err("machine enrollment requires non-nil organization and workspace ids".into());
    }
    decode_public_key(&request.public_key)?;
    validate_portable_namespace("collector platform", &request.platform)?;
    validate_portable_namespace("collector architecture", &request.architecture)
}

fn validate_response(
    request: &MachineEnrollmentRequest,
    enrollment: &MachineEnrollment,
) -> Result<(), String> {
    let public_key = decode_public_key(&request.public_key)?;
    let expected_signer_id = format!("signer:{}", hex_lower(&Sha256::digest(public_key)));
    if enrollment.id.is_nil()
        || enrollment.organization_id != request.organization_id
        || enrollment.workspace_id != request.workspace_id
        || enrollment.public_key != request.public_key.to_ascii_lowercase()
        || enrollment.signer_id != expected_signer_id
        || enrollment.platform != request.platform
        || enrollment.architecture != request.architecture
    {
        return Err("backend machine enrollment does not match the requested binding".into());
    }
    decode_public_key(&enrollment.coordinator_public_key)
        .map(|_| ())
        .map_err(|_| "backend returned an invalid coordinator public key".into())
}

fn validate_portable_namespace(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || value.trim() != value
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(format!("{label} must use a lowercase portable namespace"));
    }
    Ok(())
}

fn decode_public_key(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("collector public key must be 64 hexadecimal characters".into());
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("invalid hexadecimal value".into()),
    }
}
