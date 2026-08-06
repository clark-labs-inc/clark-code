use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{auth_origin, Credential};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceStart {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DevicePoll<'a> {
    device_code: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceCredential {
    key: String,
    api_key_id: String,
    account_email: Option<String>,
}

pub async fn login() -> Result<Credential, String> {
    let client = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(Duration::from_secs(30)),
        user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    })
    .map_err(|error| format!("could not initialize Clark network client: {error}"))?;
    let start = client
        .post(format!("{}/api/cli-auth/device/start", auth_origin()))
        .send()
        .await
        .map_err(|error| format!("could not start Clark device sign-in: {error}"))?;
    let status = start.status();
    if !status.is_success() {
        return Err(format!(
            "Clark device sign-in is unavailable ({status}). Use `clark login --api-key` as a fallback."
        ));
    }
    let start: DeviceStart = start
        .json()
        .await
        .map_err(|error| format!("Clark returned an invalid device sign-in response: {error}"))?;
    println!(
        "Open this URL on any device:\n\n  {}\n\nThen enter this one-time code:\n\n  {}\n",
        start.verification_uri, start.user_code
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(start.expires_in.min(15 * 60));
    let interval = Duration::from_secs(start.interval.clamp(2, 15));
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(
                "Clark device sign-in expired; run `clark login --device-code` again".into(),
            );
        }
        tokio::time::sleep(interval).await;
        let response = client
            .post(format!("{}/api/cli-auth/device/token", auth_origin()))
            .json(&DevicePoll {
                device_code: &start.device_code,
            })
            .send()
            .await
            .map_err(|error| format!("could not poll Clark device sign-in: {error}"))?;
        match response.status().as_u16() {
            200 => {
                let credential: DeviceCredential = response.json().await.map_err(|error| {
                    format!("Clark returned an invalid device credential: {error}")
                })?;
                return Ok(Credential {
                    api_key: credential.key,
                    account_email: credential.account_email,
                    api_key_id: Some(credential.api_key_id),
                    created_by: "device_code".into(),
                });
            }
            202 => continue,
            404 | 410 => {
                return Err(
                    "Clark device sign-in expired; run `clark login --device-code` again".into(),
                )
            }
            status => {
                let body = response.text().await.unwrap_or_default();
                return Err(format!(
                    "Clark device sign-in failed (HTTP {status}): {}",
                    body.chars().take(300).collect::<String>()
                ));
            }
        }
    }
}
