use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScienceAccess {
    organization_id: String,
    state: String,
}

pub async fn preflight(api_key: &str, organization_id: &str) -> Result<(), String> {
    preflight_at(&crate::auth::platform_api_base(), api_key, organization_id).await
}

async fn preflight_at(
    platform_api_base: &str,
    api_key: &str,
    organization_id: &str,
) -> Result<(), String> {
    let client = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(Duration::from_secs(20)),
        user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    })
    .map_err(|error| {
        science_unavailable(&format!(
            "could not initialize Clark science cloud client: {error}"
        ))
    })?;
    let mut url = url::Url::parse(&format!(
        "{}/science/access",
        platform_api_base.trim_end_matches('/')
    ))
    .map_err(|error| {
        science_unavailable(&format!("Clark science cloud URL is invalid: {error}"))
    })?;
    url.query_pairs_mut()
        .append_pair("organizationId", organization_id);
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|error| science_unavailable(&error.to_string()))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(science_unavailable(&format!(
            "Clark science access returned {status}: {}",
            body.chars().take(500).collect::<String>()
        )));
    }
    let access: ScienceAccess = serde_json::from_str(&body).map_err(|error| {
        science_unavailable(&format!(
            "Clark returned invalid science access state: {error}"
        ))
    })?;
    validate_access(access, organization_id)
}

fn validate_access(access: ScienceAccess, expected_organization_id: &str) -> Result<(), String> {
    if access.state != "ready" || access.organization_id != expected_organization_id {
        return Err(science_unavailable(
            "Clark science access did not confirm the selected organization",
        ));
    }
    Ok(())
}

fn science_unavailable(detail: &str) -> String {
    format!(
        "Clark science artifact synchronization is required but unavailable: {detail}. No worker or model was started."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_must_be_ready_for_the_exact_selected_organization() {
        let organization_id = uuid::Uuid::from_u128(1).to_string();
        assert!(validate_access(
            ScienceAccess {
                organization_id: organization_id.clone(),
                state: "ready".into(),
            },
            &organization_id,
        )
        .is_ok());

        let error = validate_access(
            ScienceAccess {
                organization_id: uuid::Uuid::from_u128(2).to_string(),
                state: "ready".into(),
            },
            &organization_id,
        )
        .unwrap_err();
        assert!(error.contains("No worker or model was started"));
    }
}
