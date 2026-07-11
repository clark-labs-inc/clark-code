use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_core::AgentEvent;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commands::clark_rest_base;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudTrajectoryConfig {
    pub endpoint: String,
    pub token: String,
    pub title: String,
    pub provider: String,
    pub project: Option<String>,
    pub repository_fingerprint: Option<String>,
    pub remote_host: Option<String>,
    pub mode: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Clone)]
pub struct CloudTrajectoryClient {
    conversation_id: String,
    config: CloudTrajectoryConfig,
    http: reqwest::Client,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Conversation<'a> {
    title: &'a str,
    provider: &'a str,
    project: Option<&'a str>,
    repository_fingerprint: Option<&'a str>,
    remote_host: Option<&'a str>,
    mode: Option<&'a str>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventRecord {
    event_id: uuid::Uuid,
    run_id: Option<String>,
    event_kind: String,
    recorded_at_unix_ms: i64,
    payload: Value,
}

#[derive(Serialize)]
struct AppendRequest<'a> {
    conversation: Conversation<'a>,
    events: &'a [EventRecord],
}

impl CloudTrajectoryClient {
    pub fn new(conversation_id: String, config: CloudTrajectoryConfig) -> Self {
        Self {
            conversation_id,
            config,
            http: reqwest::Client::new(),
        }
    }

    pub async fn append(&self, events: &[AgentEvent]) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let records = events
            .iter()
            .enumerate()
            .map(|(offset, event)| {
                let event_value = serde_json::to_value(event)
                    .map_err(|error| format!("serialize trajectory event: {error}"))?;
                let event_kind = event_kind(&event_value);
                let run_id = event_value
                    .get("run")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Ok(EventRecord {
                    event_id: uuid::Uuid::new_v4(),
                    run_id,
                    event_kind,
                    recorded_at_unix_ms: now + offset as i64,
                    payload: json!({
                        "schemaVersion": 1,
                        "sessionId": self.conversation_id,
                        "appVersion": env!("CARGO_PKG_VERSION"),
                        "platform": std::env::consts::OS,
                        "arch": std::env::consts::ARCH,
                        "metadata": self.config.metadata.clone(),
                        "event": event_value,
                    }),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let request = AppendRequest {
            conversation: Conversation {
                title: &self.config.title,
                provider: &self.config.provider,
                project: self.config.project.as_deref(),
                repository_fingerprint: self.config.repository_fingerprint.as_deref(),
                remote_host: self.config.remote_host.as_deref(),
                mode: self.config.mode.as_deref(),
            },
            events: &records,
        };
        let url = format!(
            "{}/api/desktop/conversations/{}/trajectory",
            clark_rest_base(&self.config.endpoint),
            urlencoding::encode(&self.conversation_id)
        );

        let mut last_error = String::new();
        for (attempt, delay) in [0_u64, 250, 1_000].into_iter().enumerate() {
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            match self
                .http
                .post(&url)
                .bearer_auth(&self.config.token)
                .json(&request)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => {
                    let status = response.status();
                    let detail = response.text().await.unwrap_or_default();
                    last_error = format!(
                        "trajectory endpoint returned {status}: {}",
                        detail.chars().take(300).collect::<String>()
                    );
                    if status.is_client_error()
                        && status != StatusCode::TOO_MANY_REQUESTS
                        && status != StatusCode::REQUEST_TIMEOUT
                    {
                        break;
                    }
                }
                Err(error) => last_error = format!("trajectory request failed: {error}"),
            }
            if attempt == 2 {
                break;
            }
        }
        Err(last_error)
    }
}

fn event_kind(event: &Value) -> String {
    let outer = event
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if outer != "trace" {
        return outer.to_string();
    }
    let source = event
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("provider");
    let inner = event
        .pointer("/payload/type")
        .and_then(Value::as_str)
        .unwrap_or("event");
    format!("trace.{source}.{inner}")
}

#[cfg(test)]
mod tests {
    use super::event_kind;
    use serde_json::json;

    #[test]
    fn trace_kind_preserves_provider_event_type() {
        assert_eq!(
            event_kind(&json!({
                "event": "trace",
                "source": "clark_agent",
                "payload": {"type": "context_transform_applied"}
            })),
            "trace.clark_agent.context_transform_applied"
        );
    }
}
