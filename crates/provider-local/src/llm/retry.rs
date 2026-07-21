use std::time::Duration;

use futures::StreamExt;
use reqwest::header::RETRY_AFTER;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use agent_core::recovery::{ProviderIncidentCategory, ProviderRetryCounts};

use super::{
    drain_lines, recovery, retry_after_from_metadata, Accumulator, AssistantTurn, ChatMessage,
    LlmClient, LlmError, ProviderFailureContext, ToolSchema,
};

const MAX_RATE_LIMIT_RETRIES: usize = 12;
const MAX_RATE_LIMIT_DELAY: Duration = Duration::from_secs(30);
const MAX_TRANSIENT_RETRIES: usize = 3;
const MAX_TRANSIENT_DELAY: Duration = Duration::from_secs(8);
const MAX_SERVER_TRANSIENT_DELAY: Duration = Duration::from_secs(30);
const MAX_AUTH_RETRIES: usize = 1;
const MAX_REQUEST_ATTEMPTS: usize =
    1 + MAX_RATE_LIMIT_RETRIES + MAX_TRANSIENT_RETRIES + MAX_AUTH_RETRIES;

/// Classify the provider's context-overflow dialect while the response is
/// still at the model transport boundary.
fn is_context_overflow_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("context_length_exceeded")
        || lower.contains("context length")
        || lower.contains("context window")
        || lower.contains("exceeds the maximum number of tokens")
        || (lower.contains("too many tokens") && !lower.contains("rate"))
}

struct RetryableFailure {
    category: ProviderIncidentCategory,
    message: String,
    retry_after: Option<Duration>,
    retry_safe: bool,
    provider_status: Option<u16>,
    provider_error_type: Option<String>,
    provider_request_id: Option<String>,
    output_started: bool,
}

enum AttemptError {
    Terminal(LlmError),
    RateLimited(RetryableFailure),
    Transient(RetryableFailure),
    PlatformKeyRejected(String),
}

impl From<LlmError> for AttemptError {
    fn from(error: LlmError) -> Self {
        Self::Terminal(error)
    }
}

struct RetryState {
    idempotency_key: String,
    request_started_at_ms: u64,
    request_attempts: usize,
    rate_limit_retries: usize,
    transient_retries: usize,
    auth_retries: usize,
}

impl RetryState {
    fn new() -> Self {
        Self {
            idempotency_key: format!("clark-code-{}", Uuid::new_v4()),
            request_started_at_ms: recovery::now_ms(),
            request_attempts: 0,
            rate_limit_retries: 0,
            transient_retries: 0,
            auth_retries: 0,
        }
    }
}

impl LlmClient {
    /// Stream one chat completion, transparently replaying rate-limited calls
    /// only while the attempt has emitted no user-visible output.
    pub async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        cancel: &CancellationToken,
        on_text: impl FnMut(&str),
        on_reasoning: impl FnMut(&str),
    ) -> Result<AssistantTurn, LlmError> {
        self.stream_chat_observed(messages, tools, cancel, on_text, on_reasoning, |_| {})
            .await
    }

    pub(crate) async fn stream_chat_observed(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        cancel: &CancellationToken,
        mut on_text: impl FnMut(&str),
        mut on_reasoning: impl FnMut(&str),
        mut on_retry: impl FnMut(ProviderFailureContext),
    ) -> Result<AssistantTurn, LlmError> {
        // One client key identifies this logical model turn across retries.
        // Whether a configured gateway honors it is provider-specific, so
        // diagnostics keep it separate from the provider's own request ID.
        let mut retry = RetryState::new();
        loop {
            if cancel.is_cancelled() {
                return Err(LlmError::Cancelled);
            }
            retry.request_attempts += 1;
            match self
                .stream_chat_once(
                    messages,
                    tools,
                    cancel,
                    &retry.idempotency_key,
                    &mut on_text,
                    &mut on_reasoning,
                )
                .await
            {
                Ok(turn) => return Ok(turn),
                Err(AttemptError::Terminal(error)) => return Err(error),
                Err(AttemptError::RateLimited(failure))
                    if failure.retry_safe && retry.rate_limit_retries < MAX_RATE_LIMIT_RETRIES =>
                {
                    let delay = rate_limit_delay(failure.retry_after, retry.rate_limit_retries);
                    retry.rate_limit_retries += 1;
                    on_retry(self.failure_context(&failure, &retry));
                    tracing::warn!(
                        model = self.model,
                        retry = retry.rate_limit_retries,
                        max_retries = MAX_RATE_LIMIT_RETRIES,
                        delay_ms = delay.as_millis() as u64,
                        "model rate limited before output; retrying the same turn",
                    );
                    if !delay.is_zero() {
                        tokio::select! {
                            _ = cancel.cancelled() => return Err(LlmError::Cancelled),
                            _ = tokio::time::sleep(delay) => {}
                        }
                    }
                }
                Err(AttemptError::RateLimited(failure)) => {
                    return Err(LlmError::Recoverable(
                        self.failure_context(&failure, &retry),
                    ));
                }
                Err(AttemptError::Transient(failure))
                    if failure.retry_safe && retry.transient_retries < MAX_TRANSIENT_RETRIES =>
                {
                    let delay = transient_delay(failure.retry_after, retry.transient_retries);
                    retry.transient_retries += 1;
                    on_retry(self.failure_context(&failure, &retry));
                    tracing::warn!(
                        model = self.model,
                        retry = retry.transient_retries,
                        max_retries = MAX_TRANSIENT_RETRIES,
                        delay_ms = delay.as_millis() as u64,
                        error = failure.message,
                        "model request failed before output; retrying the same turn",
                    );
                    if !delay.is_zero() {
                        tokio::select! {
                            _ = cancel.cancelled() => return Err(LlmError::Cancelled),
                            _ = tokio::time::sleep(delay) => {}
                        }
                    }
                }
                Err(AttemptError::Transient(failure)) => {
                    return Err(LlmError::Recoverable(
                        self.failure_context(&failure, &retry),
                    ));
                }
                Err(AttemptError::PlatformKeyRejected(_message))
                    if retry.auth_retries < MAX_AUTH_RETRIES =>
                {
                    retry.auth_retries += 1;
                    tracing::warn!(
                        retry = retry.auth_retries,
                        max_retries = MAX_AUTH_RETRIES,
                        "Clark API rejected the current platform key; retrying once",
                    );
                }
                Err(AttemptError::PlatformKeyRejected(message)) => {
                    return Err(LlmError::PlatformKeyRejected(message));
                }
            }
        }
    }

    fn failure_context(
        &self,
        failure: &RetryableFailure,
        retry: &RetryState,
    ) -> ProviderFailureContext {
        ProviderFailureContext {
            category: failure.category,
            message: recovery::redact_provider_detail(&failure.message),
            model: self.model.clone(),
            provider_route: recovery::provider_route(&self.base_url),
            provider_status: failure.provider_status,
            provider_error_type: failure.provider_error_type.clone(),
            idempotency_key: retry.idempotency_key.clone(),
            provider_request_id: failure.provider_request_id.clone(),
            attempts: retry.request_attempts.try_into().unwrap_or(u32::MAX),
            max_attempts: MAX_REQUEST_ATTEMPTS.try_into().unwrap_or(u32::MAX),
            retries: ProviderRetryCounts {
                transient: retry.transient_retries.try_into().unwrap_or(u32::MAX),
                rate_limit: retry.rate_limit_retries.try_into().unwrap_or(u32::MAX),
                authentication: retry.auth_retries.try_into().unwrap_or(u32::MAX),
            },
            output_started: failure.output_started,
            request_started_at_ms: retry.request_started_at_ms,
            observed_at_ms: recovery::now_ms(),
        }
    }

    async fn stream_chat_once(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        cancel: &CancellationToken,
        idempotency_key: &str,
        on_text: &mut impl FnMut(&str),
        on_reasoning: &mut impl FnMut(&str),
    ) -> Result<AssistantTurn, AttemptError> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut request = self
            .http
            .post(&url)
            .header("Idempotency-Key", idempotency_key)
            .json(&self.body(messages, tools));
        if let Some(session_id) = &self.session_id {
            request = request.header("x-session-id", session_id);
        }
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        for (name, value) in &self.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(LlmError::Cancelled.into()),
            response = request.send() => response.map_err(|error| {
                AttemptError::Transient(RetryableFailure {
                    category: if error.is_timeout() {
                        ProviderIncidentCategory::Timeout
                    } else if error.is_connect() {
                        ProviderIncidentCategory::ConnectionLost
                    } else {
                        ProviderIncidentCategory::UpstreamUnavailable
                    },
                    message: format!("model request failed: {error}"),
                    retry_after: None,
                    retry_safe: true,
                    provider_status: None,
                    provider_error_type: None,
                    provider_request_id: None,
                    output_started: false,
                })
            })?,
        };
        let status = response.status();
        let provider_request_id = ["x-request-id", "request-id", "cf-ray"]
            .into_iter()
            .find_map(|name| response.headers().get(name))
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        if !status.is_success() {
            let retry_after = retry_after_header(response.headers());
            let body = tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Cancelled.into()),
                body = response.text() => body.unwrap_or_default(),
            };
            if status.as_u16() == 403 && body.to_lowercase().contains("credit") {
                return Err(LlmError::InsufficientCredits.into());
            }
            let message = format!(
                "model endpoint returned {status}: {}",
                body.chars().take(500).collect::<String>()
            );
            if status.as_u16() == 401 {
                return Err(AttemptError::PlatformKeyRejected(message));
            }
            if status.as_u16() == 429 {
                return Err(AttemptError::RateLimited(RetryableFailure {
                    category: ProviderIncidentCategory::RateLimit,
                    message,
                    retry_after: retry_after.or_else(|| retry_after_body(&body)),
                    retry_safe: true,
                    provider_status: Some(status.as_u16()),
                    provider_error_type: Some("rate_limited".to_string()),
                    provider_request_id,
                    output_started: false,
                }));
            }
            if is_context_overflow_message(&message) {
                return Err(LlmError::ContextOverflow(message).into());
            }
            if is_transient_status(status.as_u16()) {
                return Err(AttemptError::Transient(RetryableFailure {
                    category: if matches!(status.as_u16(), 408 | 504 | 524) {
                        ProviderIncidentCategory::Timeout
                    } else {
                        ProviderIncidentCategory::UpstreamUnavailable
                    },
                    message,
                    retry_after: retry_after.or_else(|| retry_after_body(&body)),
                    retry_safe: true,
                    provider_status: Some(status.as_u16()),
                    provider_error_type: Some("upstream_http_error".to_string()),
                    provider_request_id,
                    output_started: false,
                }));
            }
            return Err(LlmError::Provider(message).into());
        }

        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut accumulator = Accumulator::default();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Cancelled.into()),
                next = stream.next() => next,
            };
            match next {
                None => break,
                Some(Err(error)) => {
                    return Err(AttemptError::Transient(RetryableFailure {
                        category: if error.is_timeout() {
                            ProviderIncidentCategory::Timeout
                        } else {
                            ProviderIncidentCategory::ConnectionLost
                        },
                        message: format!("model stream error: {error}"),
                        retry_after: None,
                        retry_safe: !accumulator.emitted_output(),
                        provider_status: None,
                        provider_error_type: Some("stream_transport".to_string()),
                        provider_request_id: provider_request_id.clone(),
                        output_started: accumulator.emitted_output(),
                    }));
                }
                Some(Ok(bytes)) => {
                    buffer.extend_from_slice(&bytes);
                    if drain_lines(&mut buffer, &mut accumulator, on_text, on_reasoning) {
                        break;
                    }
                }
            }
        }
        if let Some(error) = accumulator.stream_error.take() {
            if error.is_rate_limited() {
                return Err(AttemptError::RateLimited(RetryableFailure {
                    category: ProviderIncidentCategory::RateLimit,
                    message: error.message,
                    retry_after: error.retry_after,
                    retry_safe: !accumulator.emitted_output(),
                    provider_status: error.code,
                    provider_error_type: error.error_type,
                    provider_request_id: provider_request_id.clone(),
                    output_started: accumulator.emitted_output(),
                }));
            }
            if is_context_overflow_message(&error.message) {
                return Err(LlmError::ContextOverflow(error.message).into());
            }
            if error.is_transient() {
                return Err(AttemptError::Transient(RetryableFailure {
                    category: match (error.code, error.error_type.as_deref()) {
                        (Some(408 | 504 | 524), _)
                        | (_, Some("request_timeout" | "upstream_timeout")) => {
                            ProviderIncidentCategory::Timeout
                        }
                        (Some(500..=503), _)
                        | (_, Some("provider_unavailable" | "upstream_error")) => {
                            ProviderIncidentCategory::UpstreamUnavailable
                        }
                        _ => ProviderIncidentCategory::ConnectionLost,
                    },
                    message: error.message,
                    retry_after: error.retry_after,
                    retry_safe: !accumulator.emitted_output(),
                    provider_status: error.code,
                    provider_error_type: error.error_type,
                    provider_request_id,
                    output_started: accumulator.emitted_output(),
                }));
            }
            return Err(LlmError::Provider(error.message).into());
        }
        Ok(accumulator.finish())
    }
}

fn retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn retry_after_body(body: &str) -> Option<Duration> {
    serde_json::from_str::<Value>(body)
        .ok()?
        .get("error")?
        .get("metadata")?
        .as_object()
        .and_then(retry_after_from_metadata)
}

fn rate_limit_delay(server_hint: Option<Duration>, retry: usize) -> Duration {
    server_hint
        .unwrap_or_else(|| Duration::from_secs(2_u64 << retry.min(4)))
        .min(MAX_RATE_LIMIT_DELAY)
}

fn transient_delay(server_hint: Option<Duration>, retry: usize) -> Duration {
    match server_hint {
        Some(delay) => delay.min(MAX_SERVER_TRANSIENT_DELAY),
        None => Duration::from_millis(500_u64 << retry.min(4)).min(MAX_TRANSIENT_DELAY),
    }
}

fn is_transient_status(status: u16) -> bool {
    status == 408 || status == 425 || status == 524 || (500..=504).contains(&status)
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
