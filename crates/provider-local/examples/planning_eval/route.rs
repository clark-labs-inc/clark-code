use crate::model::RouteReceipt;
use crate::retry::{receipt, retry_after, retryable_status, wait_with_progress, ROUTE_DELAYS};
use serde_json::{json, Value};

const FREE_ROUTE: &str = "clark-code:free";
const EXPECTED_EFFECTIVE_MODEL: &str = "~deepseek/deepseek-v4-flash-latest";
const EXPECTED_CATALOG_LABEL: &str = "Free";

pub struct LiveConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub profile: String,
    pub reasoning_effort: String,
}

pub fn offline_route() -> RouteReceipt {
    RouteReceipt {
        requested_model: "offline-reference".into(),
        effective_model: "deterministic-reference".into(),
        product_route: "none".into(),
        free_tier_verified: false,
        verification_method: "no model call".into(),
        catalog_tier_id: None,
        catalog_model_option_id: None,
        catalog_label: None,
        probe_input_tokens: 0,
        probe_output_tokens: 0,
        probe_upstream_cost_usd: 0.0,
        probe_retries: Vec::new(),
    }
}

pub async fn verify_free_route(config: &mut LiveConfig) -> Result<RouteReceipt, String> {
    let requested_model = config.model.clone();
    if config.model != FREE_ROUTE {
        return Err(format!(
            "live benchmark requires the included {FREE_ROUTE} route; got {}",
            config.model
        ));
    }
    let client = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(std::time::Duration::from_secs(60)),
        ..Default::default()
    })
    .map_err(|error| format!("model catalog client failed: {error}"))?;
    let models_url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let catalog_response = client
        .get(models_url)
        .bearer_auth(&config.api_key)
        .send()
        .await
        .map_err(|error| format!("model catalog request failed: {error}"))?;
    let catalog_status = catalog_response.status();
    let catalog: Value = catalog_response.json().await.map_err(|error| {
        format!("model catalog returned invalid JSON ({catalog_status}): {error}")
    })?;
    if !catalog_status.is_success() {
        return Err(format!(
            "model catalog rejected ({catalog_status}): {}",
            safe_error(&catalog)
        ));
    }
    let model = catalog
        .get("data")
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .find(|model| model.get("id").and_then(Value::as_str) == Some("clark-code"))
        })
        .ok_or("authenticated catalog does not advertise the Clark Code tier")?;
    let option = model
        .pointer("/clark/model_options")
        .and_then(Value::as_array)
        .and_then(|options| {
            options
                .iter()
                .find(|option| option.get("id").and_then(Value::as_str) == Some(FREE_ROUTE))
        })
        .ok_or("authenticated catalog does not advertise the Clark Code Free option")?;
    let tier_id = option.pointer("/clark/tier_id").and_then(Value::as_str);
    let option_id = option
        .pointer("/clark/model_option_id")
        .and_then(Value::as_str);
    let label = option.pointer("/clark/label").and_then(Value::as_str);
    if tier_id != Some("clark-code")
        || option_id != Some("free")
        || label != Some(EXPECTED_CATALOG_LABEL)
    {
        return Err(format!(
            "DeepSeek catalog entry is not explicitly Clark Code Free (tier={tier_id:?}, option={option_id:?}, label={label:?})"
        ));
    }
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let request_body = json!({
        "model": config.model,
        "messages": [{"role": "user", "content": "Reply with OK."}],
        "temperature": 0,
        "max_tokens": 1,
        "stream": false
    });
    let mut retries = Vec::new();
    let mut attempt = 1;
    let response = loop {
        let result = client
            .post(&url)
            .bearer_auth(&config.api_key)
            .json(&request_body)
            .send()
            .await;
        match result {
            Ok(response)
                if retryable_status(response.status().as_u16())
                    && attempt <= ROUTE_DELAYS.len() =>
            {
                let status = response.status();
                let delay = retry_after(response.headers()).unwrap_or(ROUTE_DELAYS[attempt - 1]);
                let reason = response
                    .json::<Value>()
                    .await
                    .ok()
                    .map(|body| safe_error(&body))
                    .unwrap_or_else(|| status.to_string());
                let waited = wait_with_progress("route_probe", delay).await;
                retries.push(receipt(
                    "route_probe",
                    attempt,
                    status.to_string(),
                    reason,
                    delay,
                    waited,
                ));
                attempt += 1;
            }
            Ok(response) => break response,
            Err(error) => {
                return Err(format!("free-route probe transport failed: {error}"));
            }
        }
    };
    let status = response.status();
    let headers = response.headers().clone();
    let body: Value = response
        .json()
        .await
        .map_err(|error| format!("free-route probe returned invalid JSON ({status}): {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "free-route probe rejected ({status}): {}",
            safe_error(&body)
        ));
    }
    let effective_model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or("free-route probe omitted effective model")?
        .to_string();
    if !is_expected_effective_model(&effective_model) {
        return Err(format!(
            "free route resolved to {effective_model}, expected {EXPECTED_EFFECTIVE_MODEL} or its concrete dated snapshot"
        ));
    }
    let free_header = headers
        .get("x-clark-free-tier")
        .and_then(|value| value.to_str().ok());
    if matches!(free_header, Some(value) if value != "true" && value != "1") {
        return Err(format!(
            "free-route probe explicitly denied free-tier status: {free_header:?}"
        ));
    }
    Ok(RouteReceipt {
        requested_model,
        effective_model,
        product_route: FREE_ROUTE.into(),
        free_tier_verified: true,
        verification_method: "authenticated catalog tier mapping + response model".into(),
        catalog_tier_id: tier_id.map(str::to_string),
        catalog_model_option_id: option_id.map(str::to_string),
        catalog_label: label.map(str::to_string),
        probe_input_tokens: body
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        probe_output_tokens: body
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        probe_upstream_cost_usd: body
            .pointer("/usage/cost")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        probe_retries: retries,
    })
}

fn normalize_model(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

fn is_expected_effective_model(value: &str) -> bool {
    if normalize_model(value) == normalize_model(EXPECTED_EFFECTIVE_MODEL) {
        return true;
    }
    let normalized = value.trim().to_ascii_lowercase();
    let Some(snapshot) = normalized.strip_prefix("deepseek/deepseek-v4-flash-") else {
        return false;
    };
    matches!(snapshot.len(), 4 | 8) && snapshot.bytes().all(|byte| byte.is_ascii_digit())
}

fn safe_error(body: &Value) -> String {
    body.pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("request rejected")
        .chars()
        .take(240)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_route_does_not_claim_live_free_tier_verification() {
        let receipt = offline_route();
        assert_eq!(receipt.product_route, "none");
        assert!(!receipt.free_tier_verified);
        assert_eq!(receipt.verification_method, "no model call");
    }

    #[test]
    fn deepseek_route_normalization_is_strict_but_format_tolerant() {
        assert_eq!(
            normalize_model("~deepseek/deepseek-v4-flash-latest"),
            normalize_model("DEEPSEEK-DEEPSEEK-V4-FLASH-LATEST")
        );
        assert_ne!(
            normalize_model("~deepseek/deepseek-v4-flash-latest"),
            normalize_model("deepseek/deepseek-v4-flash")
        );
        assert!(is_expected_effective_model(
            "deepseek/deepseek-v4-flash-0731"
        ));
        assert!(is_expected_effective_model(
            "deepseek/deepseek-v4-flash-20260731"
        ));
        assert!(!is_expected_effective_model(
            "deepseek/deepseek-v4-pro-0731"
        ));
        assert!(!is_expected_effective_model(
            "deepseek/deepseek-v4-flash-preview"
        ));
        assert_eq!(EXPECTED_CATALOG_LABEL, "Free");
    }
}
