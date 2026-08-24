use std::time::Duration;

use futures::StreamExt;
use reqwest::header::RETRY_AFTER;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use agent_core::recovery::{ProviderIncidentCategory, ProviderRetryCounts};
use agent_core::{classify_provider_access_failure, RunFailureKind};

use super::{
    drain_lines, output_quarantine, recovery, retry_after_from_metadata, Accumulator,
    AssistantTurn, ChatMessage, LlmClient, LlmError, ProviderFailureContext, StreamChatOptions,
    ToolSchema, WireToolCallDelta,
};

const MAX_RATE_LIMIT_RETRIES: usize = 12;
const MAX_RATE_LIMIT_DELAY: Duration = Duration::from_secs(30);
const MAX_TRANSIENT_RETRIES: usize = 3;
/// A broken response body is replayed once because the quarantined attempt has
/// not published text or executed a tool. More replays make an upstream stream
/// outage feel like an endless task instead of a short invisible recovery.
const MAX_STREAM_TRANSIENT_RETRIES: usize = 1;
/// OpenRouter may normalize a provider-side network failure into HTTP 200 plus
/// `finish_reason=stop`. Its typed native reason remains safe to replay before
/// output and gets two bounded retries so free/cold routes can recover.
const MAX_NATIVE_NETWORK_RETRIES: usize = 2;
const MAX_TRANSIENT_DELAY: Duration = Duration::from_secs(8);
const MAX_SERVER_TRANSIENT_DELAY: Duration = Duration::from_secs(30);
const MAX_AUTH_RETRIES: usize = 1;
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
    UnsupportedModel(String),
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

pub(crate) struct StreamObservers<OnText, OnReasoning, OnToolCall, OnRetry> {
    on_text: OnText,
    on_reasoning: OnReasoning,
    on_tool_call: OnToolCall,
    on_retry: OnRetry,
}

impl<OnText, OnReasoning, OnToolCall, OnRetry>
    StreamObservers<OnText, OnReasoning, OnToolCall, OnRetry>
{
    pub(crate) fn new(
        on_text: OnText,
        on_reasoning: OnReasoning,
        on_tool_call: OnToolCall,
        on_retry: OnRetry,
    ) -> Self {
        Self {
            on_text,
            on_reasoning,
            on_tool_call,
            on_retry,
        }
    }
}

struct StreamAttempt<'a, OnText, OnReasoning, OnToolCall>
where
    OnText: FnMut(&str),
    OnReasoning: FnMut(&str),
    OnToolCall: FnMut(WireToolCallDelta),
{
    messages: &'a [ChatMessage],
    tools: &'a [ToolSchema],
    cancel: &'a CancellationToken,
    request_model: &'a str,
    force_tool_call: bool,
    forced_tool_name: Option<&'a str>,
    idempotency_key: &'a str,
    on_text: &'a mut OnText,
    on_reasoning: &'a mut OnReasoning,
    on_tool_call: &'a mut OnToolCall,
}

impl RetryState {
    fn new() -> Self {
        Self {
            idempotency_key: format!("model-request-{}", Uuid::new_v4()),
            request_started_at_ms: recovery::now_ms(),
            request_attempts: 0,
            rate_limit_retries: 0,
            transient_retries: 0,
            auth_retries: 0,
        }
    }
}

impl LlmClient {
    /// Stream one chat completion, transparently replaying retryable transport
    /// failures and one exact host-configured model-compatibility rejection only before
    /// the attempt has emitted user-visible output.
    pub(crate) async fn stream_chat(
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
        on_text: impl FnMut(&str),
        on_reasoning: impl FnMut(&str),
        on_retry: impl FnMut(ProviderFailureContext),
    ) -> Result<AssistantTurn, LlmError> {
        self.stream_chat_observed_with_tool_choice(
            messages,
            tools,
            StreamChatOptions {
                cancel,
                force_tool_call: false,
                forced_tool_name: None,
            },
            StreamObservers::new(on_text, on_reasoning, |_| {}, on_retry),
        )
        .await
    }

    pub(crate) async fn stream_chat_observed_with_tool_choice<
        OnText,
        OnReasoning,
        OnToolCall,
        OnRetry,
    >(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        options: StreamChatOptions<'_>,
        mut observers: StreamObservers<OnText, OnReasoning, OnToolCall, OnRetry>,
    ) -> Result<AssistantTurn, LlmError>
    where
        OnText: FnMut(&str),
        OnReasoning: FnMut(&str),
        OnToolCall: FnMut(WireToolCallDelta),
        OnRetry: FnMut(ProviderFailureContext),
    {
        let StreamChatOptions {
            cancel,
            force_tool_call,
            forced_tool_name,
        } = options;
        // One client key identifies an unchanged request body across transport
        // retries. Whether a configured gateway honors it is provider-specific,
        // so diagnostics keep it separate from the provider's own request ID.
        let mut retry = RetryState::new();
        let mut request_model = self.model.clone();
        let mut fallback_model = None;
        loop {
            if cancel.is_cancelled() {
                return Err(LlmError::Cancelled);
            }
            retry.request_attempts += 1;
            match self
                .stream_chat_once(StreamAttempt {
                    messages,
                    tools,
                    cancel,
                    request_model: &request_model,
                    force_tool_call,
                    forced_tool_name,
                    idempotency_key: &retry.idempotency_key,
                    on_text: &mut observers.on_text,
                    on_reasoning: &mut observers.on_reasoning,
                    on_tool_call: &mut observers.on_tool_call,
                })
                .await
            {
                Ok(mut turn) => {
                    let metadata = turn.response_metadata.get_or_insert_with(Default::default);
                    metadata.request_attempts =
                        Some(retry.request_attempts.try_into().unwrap_or(u32::MAX));
                    metadata.rate_limit_retries =
                        Some(retry.rate_limit_retries.try_into().unwrap_or(u32::MAX));
                    metadata.transient_retries =
                        Some(retry.transient_retries.try_into().unwrap_or(u32::MAX));
                    metadata.authentication_retries =
                        Some(retry.auth_retries.try_into().unwrap_or(u32::MAX));
                    metadata.fallback_model = fallback_model.clone();
                    metadata.fallback_reason = fallback_model
                        .as_ref()
                        .and(self.model_fallback.as_ref())
                        .map(|policy| policy.reason.clone());
                    return Ok(turn);
                }
                Err(AttemptError::Terminal(error)) => return Err(error),
                Err(AttemptError::UnsupportedModel(_message))
                    if fallback_model.is_none()
                        && self
                            .model_fallback
                            .as_ref()
                            .is_some_and(|policy| self.model != policy.model) =>
                {
                    let policy = self
                        .model_fallback
                        .as_ref()
                        .expect("guard requires fallback policy");
                    tracing::warn!(
                        requested_model = self.model,
                        fallback_model = policy.model,
                        "managed model endpoint rejected the selected alias; retrying once with the host fallback model",
                    );
                    request_model = policy.model.clone();
                    fallback_model = Some(request_model.clone());
                    // The body changes with the model alias, so this is a new
                    // idempotent request lane rather than a transport replay.
                    retry.idempotency_key = format!("model-fallback-{}", Uuid::new_v4());
                }
                Err(AttemptError::UnsupportedModel(message)) => {
                    return Err(LlmError::Provider(message));
                }
                Err(AttemptError::RateLimited(failure))
                    if failure.retry_safe && retry.rate_limit_retries < MAX_RATE_LIMIT_RETRIES =>
                {
                    let delay = rate_limit_delay(failure.retry_after, retry.rate_limit_retries);
                    retry.rate_limit_retries += 1;
                    (observers.on_retry)(self.failure_context(&failure, &retry, &request_model));
                    tracing::warn!(
                        model = request_model,
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
                    return Err(LlmError::Recoverable(self.failure_context(
                        &failure,
                        &retry,
                        &request_model,
                    )));
                }
                Err(AttemptError::Transient(failure))
                    if failure.retry_safe
                        && retry.transient_retries < transient_retry_limit(&failure) =>
                {
                    let max_retries = transient_retry_limit(&failure);
                    let delay = transient_delay(failure.retry_after, retry.transient_retries);
                    retry.transient_retries += 1;
                    (observers.on_retry)(self.failure_context(&failure, &retry, &request_model));
                    tracing::warn!(
                        model = request_model,
                        retry = retry.transient_retries,
                        max_retries,
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
                    return Err(LlmError::Recoverable(self.failure_context(
                        &failure,
                        &retry,
                        &request_model,
                    )));
                }
                Err(AttemptError::PlatformKeyRejected(_message))
                    if retry.auth_retries < MAX_AUTH_RETRIES =>
                {
                    retry.auth_retries += 1;
                    tracing::warn!(
                        retry = retry.auth_retries,
                        max_retries = MAX_AUTH_RETRIES,
                        "Clark Code API rejected the current platform key; retrying once",
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
        request_model: &str,
    ) -> ProviderFailureContext {
        ProviderFailureContext {
            category: failure.category,
            message: recovery::redact_provider_detail(&failure.message),
            model: request_model.to_string(),
            provider_route: recovery::provider_route(&self.base_url),
            provider_status: failure.provider_status,
            provider_error_type: failure.provider_error_type.clone(),
            idempotency_key: retry.idempotency_key.clone(),
            provider_request_id: failure.provider_request_id.clone(),
            attempts: retry.request_attempts.try_into().unwrap_or(u32::MAX),
            max_attempts: max_attempts_for_failure(failure, retry)
                .try_into()
                .unwrap_or(u32::MAX),
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

    async fn stream_chat_once<OnText, OnReasoning, OnToolCall>(
        &self,
        attempt: StreamAttempt<'_, OnText, OnReasoning, OnToolCall>,
    ) -> Result<AssistantTurn, AttemptError>
    where
        OnText: FnMut(&str),
        OnReasoning: FnMut(&str),
        OnToolCall: FnMut(WireToolCallDelta),
    {
        let StreamAttempt {
            messages,
            tools,
            cancel,
            request_model,
            force_tool_call,
            forced_tool_name,
            idempotency_key,
            on_text,
            on_reasoning,
            on_tool_call,
        } = attempt;
        let url = format!("{}/chat/completions", self.base_url);
        let mut request = self.http.post(&url).json(&self.body_for_model(
            request_model,
            messages,
            tools,
            force_tool_call,
            forced_tool_name,
        ));
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
            response = tokio::time::timeout(self.response_start_timeout, request.send()) => {
                match response {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) => return Err(AttemptError::Transient(RetryableFailure {
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
                    })),
                    Err(_) => return Err(AttemptError::Transient(RetryableFailure {
                        category: ProviderIncidentCategory::Timeout,
                        message: format!(
                            "model response did not start within {} seconds",
                            self.response_start_timeout.as_secs()
                        ),
                        retry_after: None,
                        retry_safe: true,
                        provider_status: None,
                        provider_error_type: Some("response_start_timeout".to_string()),
                        provider_request_id: None,
                        output_started: false,
                    })),
                }
            },
        };
        let status = response.status();
        let http_version = format!("{:?}", response.version());
        let provider_request_id = ["x-request-id", "request-id", "cf-ray"]
            .into_iter()
            .find_map(|name| response.headers().get(name))
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let cache_status = response_header(&response, "x-openrouter-cache-status");
        let cache_age_seconds = response_header(&response, "x-openrouter-cache-age")
            .and_then(|value| value.parse().ok());
        let cache_ttl_seconds = response_header(&response, "x-openrouter-cache-ttl")
            .and_then(|value| value.parse().ok());
        if !status.is_success() {
            let retry_after = retry_after_header(response.headers());
            let body = tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Cancelled.into()),
                body = response.text() => body.unwrap_or_default(),
            };
            let access_failure = classify_provider_access_failure(Some(status.as_u16()), &body);
            if access_failure == Some(RunFailureKind::InsufficientCredits) {
                return Err(LlmError::InsufficientCredits.into());
            }
            let message = format!(
                "model endpoint returned {status}: {}",
                body.chars().take(500).collect::<String>()
            );
            if access_failure == Some(RunFailureKind::PlatformKeyRejected) {
                return Err(AttemptError::PlatformKeyRejected(message));
            }
            if status.as_u16() == 400
                && self
                    .model_fallback
                    .as_ref()
                    .is_some_and(|policy| is_configured_model_rejection(&body, policy))
            {
                return Err(AttemptError::UnsupportedModel(message));
            }
            if access_failure == Some(RunFailureKind::RateLimited) {
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
        // Publish completed words as their SSE deltas arrive. The open word is
        // retained so reserved provider-control markers remain quarantined
        // even when their bytes are fragmented across network frames. Tool
        // Structured reasoning remains staged in `accumulator` until the
        // complete turn passes validation below. Tool deltas also stay
        // assembled there while a typed observer may project terminal-answer
        // arguments incrementally.
        let mut text_guard = output_quarantine::StreamingGuard::new(on_text);
        let mut reasoning_guard = output_quarantine::StreamingGuard::new(on_reasoning);
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
                        // Replaying is safe only while every received word is
                        // still held by the guard. Once visible output exists,
                        // a retry would duplicate the assistant response.
                        // Tool-call deltas now cross the typed stream boundary
                        // too. Replaying after any such delta would append a
                        // duplicate partial `final_answer` (or corrupt another
                        // observer's tool-call projection) to the same message.
                        retry_safe: !text_guard.published()
                            && !reasoning_guard.published()
                            && !accumulator.emitted_tool_call(),
                        provider_status: None,
                        provider_error_type: Some("stream_transport".to_string()),
                        provider_request_id: provider_request_id.clone(),
                        output_started: accumulator.emitted_output(),
                    }));
                }
                Some(Ok(bytes)) => {
                    buffer.extend_from_slice(&bytes);
                    if drain_lines(
                        &mut buffer,
                        &mut accumulator,
                        &mut |delta| text_guard.push(delta),
                        &mut |delta| reasoning_guard.push(delta),
                        on_tool_call,
                    ) {
                        break;
                    }
                }
            }
        }
        if accumulator.native_network_error() {
            let output_started = accumulator.emitted_output();
            return Err(AttemptError::Transient(RetryableFailure {
                category: ProviderIncidentCategory::ConnectionLost,
                message: "model stream ended with native_finish_reason=network_error".to_string(),
                retry_after: None,
                retry_safe: !text_guard.published()
                    && !reasoning_guard.published()
                    && !accumulator.emitted_tool_call(),
                provider_status: Some(status.as_u16()),
                provider_error_type: Some("native_network_error".to_string()),
                provider_request_id,
                output_started,
            }));
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
        let mut turn = accumulator.finish();
        let metadata = turn.response_metadata.get_or_insert_with(Default::default);
        metadata.requested_model = Some(self.model.clone());
        metadata.provider_request_id = provider_request_id;
        metadata.idempotency_key = Some(idempotency_key.to_string());
        metadata.session_id = self.session_id.clone();
        metadata.provider_route = Some(recovery::provider_route(&self.base_url));
        metadata.http_version = Some(http_version);
        metadata.cache_status = cache_status;
        metadata.cache_age_seconds = cache_age_seconds;
        metadata.cache_ttl_seconds = cache_ttl_seconds;
        if let Some(violation) = output_quarantine::inspect(&turn, messages) {
            let metadata = Box::new(turn.response_metadata.take().unwrap_or_default());
            tracing::error!(
                reason = violation.code(),
                generation_id = metadata.generation_id.as_deref().unwrap_or("unknown"),
                resolved_model = metadata.resolved_model.as_deref().unwrap_or("unknown"),
                provider = metadata.provider.as_deref().unwrap_or("unknown"),
                "provider response rejected before settlement",
            );
            return Err(LlmError::OutputQuarantined {
                reason: violation.code(),
                metadata,
            }
            .into());
        }
        reasoning_guard.flush();
        text_guard.flush();
        Ok(turn)
    }
}

fn response_header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
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

fn is_configured_model_rejection(body: &str, policy: &crate::config::ModelFallbackPolicy) -> bool {
    let Ok(payload) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    let Some(error) = payload.get("error") else {
        return false;
    };
    error.get("type").and_then(Value::as_str) == Some(policy.error_type.as_str())
        && error.get("param").and_then(Value::as_str) == Some(policy.error_param.as_str())
        && error
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.eq_ignore_ascii_case(&policy.error_message))
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

fn transient_retry_limit(failure: &RetryableFailure) -> usize {
    match failure.provider_error_type.as_deref() {
        Some("stream_transport") => MAX_STREAM_TRANSIENT_RETRIES,
        Some("native_network_error") => MAX_NATIVE_NETWORK_RETRIES,
        _ => MAX_TRANSIENT_RETRIES,
    }
}

fn max_attempts_for_failure(failure: &RetryableFailure, retry: &RetryState) -> usize {
    if !failure.retry_safe {
        return retry.request_attempts;
    }
    let remaining = match failure.category {
        ProviderIncidentCategory::RateLimit => {
            MAX_RATE_LIMIT_RETRIES.saturating_sub(retry.rate_limit_retries)
        }
        _ => transient_retry_limit(failure).saturating_sub(retry.transient_retries),
    };
    retry.request_attempts.saturating_add(remaining)
}

fn is_transient_status(status: u16) -> bool {
    status == 408 || status == 425 || status == 524 || (500..=504).contains(&status)
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "retry_streaming_tests.rs"]
mod streaming_tests;

#[cfg(test)]
#[path = "native_finish_retry_tests.rs"]
mod native_finish_retry_tests;
