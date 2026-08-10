use std::time::{SystemTime, UNIX_EPOCH};

use agent_core::recovery::{ProviderIncidentCategory, ProviderRetryCounts};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderFailureContext {
    pub category: ProviderIncidentCategory,
    pub message: String,
    pub model: String,
    pub provider_route: String,
    pub provider_status: Option<u16>,
    pub provider_error_type: Option<String>,
    pub idempotency_key: String,
    pub provider_request_id: Option<String>,
    pub attempts: u32,
    pub max_attempts: u32,
    pub retries: ProviderRetryCounts,
    pub output_started: bool,
    pub request_started_at_ms: u64,
    pub observed_at_ms: u64,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(crate) fn provider_route(base_url: &str) -> String {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?;
            let path = url.path().trim_end_matches('/');
            Some(if path.is_empty() {
                host.to_string()
            } else {
                format!("{host}{path}")
            })
        })
        .unwrap_or_else(|| "Clark Code model gateway".to_string())
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "token",
        "secret",
        "password",
        "cookie",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn redact_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if sensitive_key(key) {
                    *value = Value::String("[redacted]".to_string());
                } else {
                    redact_json(value);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(redact_json),
        Value::String(value) => *value = redact_text(value),
        _ => {}
    }
}

fn redact_text(message: &str) -> String {
    let mut output = Vec::new();
    let mut redact_next = false;
    for raw in message.split_whitespace() {
        let lower = raw.to_ascii_lowercase();
        if redact_next {
            if lower.trim_matches(|c: char| !c.is_ascii_alphanumeric()) == "bearer" {
                output.push("Bearer".to_string());
                continue;
            }
            output.push("[redacted]".to_string());
            redact_next = false;
            continue;
        }
        if lower.trim_matches(|c: char| !c.is_ascii_alphanumeric()) == "bearer" {
            output.push("Bearer".to_string());
            redact_next = true;
            continue;
        }
        if let Some(query) = raw.find('?') {
            let prefix = &raw[..query];
            if prefix.contains("http://") || prefix.contains("https://") {
                output.push(format!("{prefix}?[redacted]"));
                continue;
            }
        }
        let assignment = raw
            .split_once('=')
            .or_else(|| raw.split_once(':'))
            .filter(|(key, _)| sensitive_key(key));
        if let Some((_, value)) = assignment {
            output.push("[redacted]".to_string());
            redact_next = value.is_empty();
            continue;
        }
        let trailing_label = raw.ends_with(':') || raw.ends_with('=');
        let label = raw.trim_matches(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        });
        if trailing_label && sensitive_key(label) {
            output.push("[redacted]".to_string());
            redact_next = true;
            continue;
        }
        if lower.contains("sk-")
            || lower.contains("ck_live_")
            || lower.contains("ck_test_")
            || lower.starts_with("authorization=")
            || lower.starts_with("authorization:")
        {
            output.push("[redacted]".to_string());
            continue;
        }
        output.push(raw.to_string());
    }
    output.join(" ")
}

/// Persist only a bounded, aggressively sanitized diagnostic. Structured JSON
/// is redacted by key before serialization; unstructured text is scrubbed for
/// credentials, authorization values, and URL queries.
pub(crate) fn redact_provider_detail(message: &str) -> String {
    let bounded = message.chars().take(4_000).collect::<String>();
    let mut sanitized = match serde_json::from_str::<Value>(&bounded) {
        Ok(mut value) => {
            redact_json(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| "provider error".to_string())
        }
        Err(_) => redact_text(&bounded),
    };
    if sanitized.len() > 1_000 {
        let mut end = 1_000;
        while !sanitized.is_char_boundary(end) {
            end -= 1;
        }
        sanitized.truncate(end);
        sanitized.push('…');
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_details_drop_credentials_and_query_values() {
        let detail = redact_provider_detail(
            "Bearer abc sk-secret ck_live_secret (https://gateway.test/path?token=secret reset",
        );
        assert!(!detail.contains("abc"));
        assert!(!detail.contains("secret"));
        assert!(detail.contains("[redacted]"));
        assert!(detail.contains("https://gateway.test/path?[redacted]"));
    }

    #[test]
    fn provider_details_redact_structured_and_assignment_secrets() {
        let json = redact_provider_detail(
            r#"{"api_key":"sk-json","nested":{"access_token":"value"},"message":"safe"}"#,
        );
        assert!(!json.contains("sk-json"));
        assert!(!json.contains("value"));
        assert!(json.contains("safe"));

        let text = redact_provider_detail(
            "Authorization=Bearer-abc access_token=value password: hunter2 token: abc \
             Authorization: Bearer another-secret okay",
        );
        assert!(!text.contains("Bearer-abc"));
        assert!(!text.contains("value"));
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("abc"));
        assert!(!text.contains("another-secret"));
        assert!(text.contains("okay"));
    }

    #[test]
    fn provider_route_keeps_the_route_but_drops_query_credentials() {
        assert_eq!(
            provider_route("https://gateway.test/api/models/v1/?token=secret"),
            "gateway.test/api/models/v1"
        );
    }
}
