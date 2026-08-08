use std::net::IpAddr;
use std::time::Duration;

use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION};
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{MAX_ERROR_BYTES, MAX_RESPONSE_BYTES};

pub(super) fn authorization_header(api_key: &str) -> Result<HeaderValue, String> {
    if api_key.is_empty() || api_key.trim() != api_key {
        return Err("host platform API key must be a non-empty header value".into());
    }
    let mut authorization = HeaderValue::from_str(&format!("Bearer {api_key}"))
        .map_err(|_| "host platform API key is not a valid header value".to_string())?;
    authorization.set_sensitive(true);
    Ok(authorization)
}

pub(super) fn build_http_client(timeout: Duration) -> Result<Client, String> {
    desktop_http::build_client(desktop_http::ClientOptions {
        request_timeout: Some(timeout),
        ..Default::default()
    })
    .map_err(|_| "failed to initialize system-cartography HTTP client".to_string())
}

pub(super) async fn post_json<Request, Response>(
    client: &Client,
    base_url: &Url,
    authorization: &HeaderValue,
    path: &str,
    request: &Request,
) -> Result<Response, String>
where
    Request: Serialize + ?Sized,
    Response: DeserializeOwned,
{
    let response = client
        .post(endpoint(base_url, path))
        .header(AUTHORIZATION, authorization.clone())
        .header(ACCEPT, "application/json")
        .json(request)
        .send()
        .await
        .map_err(|error| request_error(error, "system-cartography request"))?;
    if !response.status().is_success() {
        return Err(http_error(response, "system-cartography request").await);
    }
    require_json(&response)?;
    let body = read_bounded(response, MAX_RESPONSE_BYTES).await?;
    let mut deserializer = serde_json::Deserializer::from_slice(&body);
    let mut unknown_fields = Vec::new();
    let response = serde_ignored::deserialize(&mut deserializer, |path| {
        unknown_fields.push(path.to_string());
    })
    .map_err(|error| format!("invalid system-cartography response JSON: {error}"))?;
    deserializer
        .end()
        .map_err(|error| format!("invalid system-cartography response JSON: {error}"))?;
    if let Some(field) = unknown_fields.first() {
        return Err(format!(
            "system-cartography response contains unknown field `{field}`"
        ));
    }
    Ok(response)
}

pub(super) fn validate_remote_url(url: &Url, label: &str) -> Result<(), String> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{label} must not contain credentials"));
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback(url.host_str()) => Ok(()),
        _ => Err(format!("{label} must use HTTPS")),
    }
}

fn endpoint(base_url: &Url, path: &str) -> Url {
    let mut endpoint = base_url.clone();
    endpoint.set_path(path);
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    endpoint
}

fn is_loopback(host: Option<&str>) -> bool {
    host.is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

fn require_json(response: &reqwest::Response) -> Result<(), String> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if content_type.is_some_and(|value| {
        value == "application/json"
            || (value.starts_with("application/") && value.ends_with("+json"))
    }) {
        Ok(())
    } else {
        Err("system-cartography response is not JSON".into())
    }
}

async fn read_bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("system-cartography response exceeds {limit} bytes"));
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or_default()
            .min(limit),
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "failed to read system-cartography response".to_string())?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(format!("system-cartography response exceeds {limit} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) async fn http_error(response: reqwest::Response, operation: &str) -> String {
    let status = response.status();
    let body = read_bounded(response, MAX_ERROR_BYTES)
        .await
        .unwrap_or_default();
    let detail = String::from_utf8_lossy(&body)
        .chars()
        .take(512)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if detail.trim().is_empty() {
        format!("{operation} returned HTTP {status}")
    } else {
        format!("{operation} returned HTTP {status}: {}", detail.trim())
    }
}

pub(super) fn request_error(error: reqwest::Error, operation: &str) -> String {
    if error.is_timeout() {
        format!("{operation} timed out")
    } else if error.is_connect() {
        format!("failed to connect for {operation}")
    } else {
        format!("{operation} failed")
    }
}
