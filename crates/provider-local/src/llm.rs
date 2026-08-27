//! OpenAI-compatible streaming chat-completions client — the model seam.
//!
//! This is the only place that knows the model wire format. It speaks the
//! ubiquitous `POST {base}/chat/completions` contract (OpenRouter, vLLM,
//! llama.cpp, LM Studio, a future Clark Code passthrough, …): streamed
//! `chat.completion.chunk` SSE frames carrying assistant text deltas and
//! fragmented tool-call deltas. The parser publishes guarded text and reasoning
//! words as they arrive, while it reassembles tool calls and the complete
//! [`AssistantTurn`] for validation before execution or transcript settlement.

use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::config::LocalConfig;

mod accumulator;
pub(crate) mod output_quarantine;
mod reasoning_receipt;
mod recovery;
mod retry;

use accumulator::{retry_after_from_metadata, Accumulator};
pub use reasoning_receipt::{ReasoningPayloadReceipt, ReasoningReplayReceipt};
pub(crate) use retry::StreamObservers;

pub(crate) use recovery::{now_ms, ProviderFailureContext};

/// Bound the wait for response headers without imposing a deadline on a healthy
/// long-running reasoning stream.
const DEFAULT_MODEL_RESPONSE_START_TIMEOUT: Duration = Duration::from_secs(2 * 60);
fn desktop_user_agent(version: &str) -> String {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!(
        "agent-desktop/{} ({} {})",
        version,
        platform,
        std::env::consts::ARCH,
    )
}

/// A single message in the running transcript, serialized straight to the wire.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChatContent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<WireToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Provider-native reasoning replayed on assistant messages of the
    /// in-flight tool exchange (an OpenAI-compatible `reasoning` field).
    /// Reasoning-capable models keep their chain across tool calls this way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// OpenRouter-normalized structured reasoning blocks. The sequence is
    /// replayed byte-for-byte on the assistant message during a tool exchange.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_details: Vec<Value>,
}

/// `content` on the OpenAI-compatible wire format is either a plain string or
/// a multimodal content-parts array — never both. `Text` serializes as a bare
/// JSON string, identical to every message before multimodal support existed;
/// `Parts` is only used where an image actually needs to ride along (and only
/// on `role: "user"` — the spec doesn't allow parts arrays on `role: "tool"`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl ChatContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrlRef },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageUrlRef {
    pub url: String,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self::simple("system", text)
    }
    pub fn developer(text: impl Into<String>) -> Self {
        Self::simple("developer", text)
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self::simple("user", text)
    }
    /// A user-role message carrying text plus one or more images — the only
    /// role the OpenAI-compatible wire format allows a content-parts array
    /// on. `image_urls` may be `data:` URLs or external URLs.
    pub fn user_with_images(text: impl Into<String>, image_urls: Vec<String>) -> Self {
        let mut parts = vec![ContentPart::Text { text: text.into() }];
        parts.extend(image_urls.into_iter().map(|url| ContentPart::ImageUrl {
            image_url: ImageUrlRef { url },
        }));
        Self {
            role: "user".into(),
            content: Some(ChatContent::Parts(parts)),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning: None,
            reasoning_details: Vec::new(),
        }
    }
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(ChatContent::text(content)),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            reasoning: None,
            reasoning_details: Vec::new(),
        }
    }
    fn simple(role: &str, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(ChatContent::text(text)),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning: None,
            reasoning_details: Vec::new(),
        }
    }
}

/// A tool call exactly as the wire represents it (arguments stay a JSON string).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: WireFunction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireFunction {
    pub name: String,
    /// Raw JSON arguments string (may be `""` for a no-arg call).
    pub arguments: String,
}

impl WireToolCall {
    pub fn function(id: impl Into<String>, name: impl Into<String>, arguments: String) -> Self {
        Self {
            id: id.into(),
            kind: "function".into(),
            function: WireFunction {
                name: name.into(),
                arguments,
            },
        }
    }
}

/// A tool the model may call (advertised in the request `tools` array).
#[derive(Clone, Debug, Serialize)]
pub struct ToolSchema {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionSchema,
}

#[derive(Clone, Debug, Serialize)]
pub struct FunctionSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSchema {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            kind: "function",
            function: FunctionSchema {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

pub(crate) struct StreamChatOptions<'a> {
    pub(crate) cancel: &'a CancellationToken,
    pub(crate) force_tool_call: bool,
    /// After an auto-mode provider returns prose on a host-forced turn, the
    /// adapter can retry with one named delivery tool. This remains `None` for
    /// ordinary turns so the model is free to choose the next work tool.
    pub(crate) forced_tool_name: Option<&'a str>,
}

/// Token/cost accounting from the final streamed chunk (OpenRouter shape,
/// forwarded verbatim by Clark Code passthrough).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TokenUsage {
    /// Prompt size of this call — the live context footprint.
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Upstream USD cost when the provider reports it.
    pub cost_usd: Option<f64>,
}

/// The fully-assembled result of one model turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssistantTurn {
    pub text: String,
    pub tool_calls: Vec<WireToolCall>,
    pub finish_reason: Option<String>,
    /// Usage reported by the stream's final chunk, when present.
    pub usage: Option<TokenUsage>,
    /// Hidden reasoning the model streamed in `delta.reasoning` (GLM/OpenRouter)
    /// or `delta.reasoning_content` (some providers) — separate from `text`.
    /// Shown in the UI, kept as a typed `Reasoning` block in history, and
    /// replayed on the wire with its assistant turn so subsequent requests
    /// retain the provider's complete reasoning state.
    pub reasoning: String,
    /// Complete OpenRouter `reasoning_details[]` values in streamed order.
    /// Unknown future item shapes remain intact for provider replay.
    pub reasoning_details: Vec<Value>,
    /// Stable transport identities for joining a successful model turn to
    /// OpenRouter/Clark Code diagnostics and cache-routing receipts.
    pub response_metadata: Option<ProviderResponseMetadata>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderResponseMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_model: Option<String>,
    /// Clark Code-managed model alias used after a pre-output compatibility
    /// rejection of the requested model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_route: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transient_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_age_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_ttl_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    /// Hash-only receipt for reasoning returned by this provider response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_capture: Option<ReasoningPayloadReceipt>,
    /// Hash-only receipts for prior assistant reasoning replayed on this request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_replays: Vec<ReasoningReplayReceipt>,
}

/// Why a model call ended without producing a turn.
#[derive(Debug)]
pub enum LlmError {
    /// The caller's cancellation token fired mid-request.
    Cancelled,
    /// The selected provider rejected the request for insufficient usage access.
    InsufficientCredits,
    /// The host-managed API rejected the desktop access key (401).
    PlatformKeyRejected(String),
    /// The configured gateway or upstream provider returned an application-level failure.
    Provider(String),
    /// The provider returned successful responses but repeatedly declined the
    /// required structured-tool boundary, including the singleton named-tool
    /// repair. This is deterministic protocol incompatibility, not transport.
    ToolProtocolExhausted(String),
    /// Provider output failed the desktop isolation boundary before any of the
    /// response was published, persisted, or offered to a tool.
    OutputQuarantined {
        reason: &'static str,
        metadata: Box<ProviderResponseMetadata>,
    },
    /// The provider rejected the request because its context was too large.
    ContextOverflow(String),
    /// A retryable provider failure with presentation-safe structured context.
    Recoverable(ProviderFailureContext),
}

impl LlmError {
    pub(crate) fn provider_failure(&self) -> Option<&ProviderFailureContext> {
        match self {
            Self::Recoverable(context) => Some(context),
            _ => None,
        }
    }

    pub(crate) fn quarantine_receipt(&self) -> Option<(&'static str, &ProviderResponseMetadata)> {
        match self {
            Self::OutputQuarantined { reason, metadata } => Some((reason, metadata)),
            _ => None,
        }
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Cancelled => f.write_str("model request cancelled"),
            LlmError::InsufficientCredits => f.write_str("insufficient_credits"),
            LlmError::PlatformKeyRejected(message)
            | LlmError::Provider(message)
            | LlmError::ToolProtocolExhausted(message)
            | LlmError::ContextOverflow(message) => f.write_str(message),
            LlmError::OutputQuarantined { .. } => {
                f.write_str("model response failed data-isolation validation")
            }
            LlmError::Recoverable(context) => f.write_str(&context.message),
        }
    }
}

/// Streaming chat client bound to one endpoint/model.
#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    response_start_timeout: Duration,
    base_url: String,
    model: String,
    api_key: Option<String>,
    headers: Vec<(String, String)>,
    /// Clark Code conversation UUID forwarded through the gateway for routing.
    session_id: Option<String>,
    temperature: Option<f32>,
    /// Host-owned response ceiling. This bounds active-but-nonproductive
    /// reasoning streams that transport-idle timeouts cannot distinguish from
    /// useful progress.
    max_output_tokens: Option<u32>,
    /// Reasoning-effort override forwarded to the passthrough ("low" … "xhigh").
    /// `None` → the server applies the model's default.
    reasoning_effort: Option<String>,
    model_reasoning_efforts: HashMap<String, String>,
    model_fallback: Option<crate::config::ModelFallbackPolicy>,
    /// Provider-owned strict response schema for this session.
    response_format: Option<Value>,
    /// Provider routing preferences supplied by the trusted host.
    provider_preferences: Option<Value>,
}

impl LlmClient {
    /// Same endpoint/auth, different model — for background side-calls (e.g.
    /// memory extraction) that shouldn't inherit a weaker session model.
    pub fn with_model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    /// Same endpoint/auth, different reasoning policy. `None` deliberately
    /// clears a conversation-level override instead of inheriting a value that
    /// may be invalid for a host-pinned model.
    pub fn with_reasoning_effort(mut self, reasoning_effort: Option<&str>) -> Self {
        self.reasoning_effort = reasoning_effort.map(str::to_string);
        self
    }

    /// Bind subsequent model calls to one Clark Code conversation.
    pub fn with_session_id(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    #[cfg(test)]
    pub(crate) fn session_id_for_test(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn new(config: &LocalConfig) -> Result<Self, String> {
        let mut client = Self::from_parts_with_client_version(
            &config.base_url,
            &config.model,
            config.api_key.clone(),
            config.headers.clone().into_iter().collect(),
            config.temperature,
            &config.client_version,
        )?;
        client.reasoning_effort = config.reasoning_effort.clone();
        client.max_output_tokens = config.max_output_tokens;
        client.model_reasoning_efforts = config
            .models
            .iter()
            .filter_map(|model| {
                model
                    .reasoning_effort
                    .clone()
                    .map(|effort| (model.id.clone(), effort))
            })
            .collect();
        client.model_fallback = config.model_fallback.clone();
        client.response_format = config.response_format.clone();
        client.provider_preferences = config.provider_preferences.clone();
        Ok(client)
    }

    /// Build a client bound to an explicit endpoint/model (used for the agentic
    /// research/memory model, which differs from the coding model).
    pub fn from_parts(
        base_url: &str,
        model: &str,
        api_key: Option<String>,
        headers: Vec<(String, String)>,
        temperature: Option<f32>,
    ) -> Result<Self, String> {
        Self::from_parts_with_client_version(
            base_url,
            model,
            api_key,
            headers,
            temperature,
            env!("CARGO_PKG_VERSION"),
        )
    }

    fn from_parts_with_client_version(
        base_url: &str,
        model: &str,
        api_key: Option<String>,
        headers: Vec<(String, String)>,
        temperature: Option<f32>,
        client_version: &str,
    ) -> Result<Self, String> {
        Self::from_parts_with_response_start_timeout_and_client_version(
            base_url,
            model,
            api_key,
            headers,
            temperature,
            DEFAULT_MODEL_RESPONSE_START_TIMEOUT,
            client_version,
        )
    }

    #[cfg(test)]
    pub(crate) fn from_parts_with_response_start_timeout(
        base_url: &str,
        model: &str,
        api_key: Option<String>,
        headers: Vec<(String, String)>,
        temperature: Option<f32>,
        response_start_timeout: Duration,
    ) -> Result<Self, String> {
        Self::from_parts_with_response_start_timeout_and_client_version(
            base_url,
            model,
            api_key,
            headers,
            temperature,
            response_start_timeout,
            env!("CARGO_PKG_VERSION"),
        )
    }

    fn from_parts_with_response_start_timeout_and_client_version(
        base_url: &str,
        model: &str,
        api_key: Option<String>,
        headers: Vec<(String, String)>,
        temperature: Option<f32>,
        response_start_timeout: Duration,
        client_version: &str,
    ) -> Result<Self, String> {
        let user_agent = desktop_user_agent(client_version);
        let http = desktop_http::build_client(desktop_http::ClientOptions {
            user_agent: Some(&user_agent),
            ..Default::default()
        })
        .map_err(|e| format!("llm client build failed: {e}"))?;
        Ok(Self {
            http,
            response_start_timeout,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key,
            headers,
            session_id: None,
            temperature,
            max_output_tokens: None,
            reasoning_effort: None,
            model_reasoning_efforts: HashMap::new(),
            model_fallback: None,
            response_format: None,
            provider_preferences: None,
        })
    }

    /// One-shot completion with NO client tools (the agentic Clark Code path): send an
    /// optional system + a user message, return the assembled assistant text.
    pub(crate) async fn complete(
        &self,
        system: Option<&str>,
        user: &str,
        cancel: &CancellationToken,
    ) -> Result<String, LlmError> {
        let mut messages = Vec::new();
        if let Some(s) = system {
            messages.push(ChatMessage::system(s));
        }
        messages.push(ChatMessage::user(user));
        let turn = self
            .stream_chat(&messages, &[], cancel, |_| {}, |_| {})
            .await?;
        Ok(turn.text)
    }

    /// Public one-shot seam for deterministic eval/support clients that need
    /// the production transport, retry, and session-affinity policy without
    /// depending on provider-local's internal failure types.
    pub async fn complete_text(
        &self,
        system: Option<&str>,
        user: &str,
        cancel: &CancellationToken,
    ) -> Result<String, String> {
        self.complete(system, user, cancel)
            .await
            .map_err(|error| error.to_string())
    }

    /// One-shot completion with image(s) attached — mirrors [`Self::complete`],
    /// swapping [`ChatMessage::user`] for [`ChatMessage::user_with_images`].
    /// Used by the vision-fallback path: a separate call to a vision-capable
    /// Clark Code model, not part of the coding model's own turn.
    pub(crate) async fn describe_images(
        &self,
        system: &str,
        prompt: &str,
        image_urls: Vec<String>,
        cancel: &CancellationToken,
    ) -> Result<String, LlmError> {
        let messages = vec![
            ChatMessage::system(system),
            ChatMessage::user_with_images(prompt, image_urls),
        ];
        let turn = self
            .stream_chat(&messages, &[], cancel, |_| {}, |_| {})
            .await?;
        Ok(turn.text)
    }

    #[cfg(test)]
    fn body(&self, messages: &[ChatMessage], tools: &[ToolSchema]) -> Value {
        self.body_for_model(&self.model, messages, tools, false, None)
    }

    #[cfg(test)]
    fn body_forced_tool(&self, messages: &[ChatMessage], tools: &[ToolSchema]) -> Value {
        self.body_for_model(&self.model, messages, tools, true, None)
    }

    #[cfg(test)]
    fn body_requiring_named_tool(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        tool_name: &str,
    ) -> Value {
        self.body_for_model(&self.model, messages, tools, true, Some(tool_name))
    }

    fn body_for_model(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        _force_tool_call: bool,
        forced_tool_name: Option<&str>,
    ) -> Value {
        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": true,
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(tools).unwrap_or(Value::Null);
            body["tool_choice"] = if let Some(tool_name) = forced_tool_name {
                json!({
                    "type": "function",
                    "function": { "name": tool_name },
                })
            } else {
                // `force_tool_call` remains a host-side typed recovery
                // contract. Providers do not reliably implement the bare
                // `required` mode, so broad catalogs stay on portable auto;
                // a later named singleton may still be supplied explicitly.
                json!("auto")
            };
        }
        if let Some(t) = self.temperature {
            body["temperature"] = json!(t);
        }
        // Normalize host-advertised per-model policy at the wire seam so
        // persisted settings and direct harnesses cannot weaken it.
        let reasoning_effort = self
            .model_reasoning_efforts
            .get(model)
            .map(String::as_str)
            .or(self.reasoning_effort.as_deref());
        if let Some(effort) = reasoning_effort {
            if self.base_url.contains("openrouter.ai")
                || self.headers.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("x-clark-client-tool-loop") && value == "1"
                })
            {
                body["reasoning"] = json!({
                    "enabled": true,
                    "effort": effort,
                    "exclude": false
                });
            } else {
                body["reasoning_effort"] = json!(effort);
            }
        }
        if let Some(response_format) = &self.response_format {
            body["response_format"] = response_format.clone();
        }
        if let Some(provider_preferences) = &self.provider_preferences {
            body["provider"] = provider_preferences.clone();
        }
        if let Some(max_tokens) = self
            .max_output_tokens
            .or_else(|| crate::config::model_max_output_tokens(model))
        {
            body["max_tokens"] = json!(max_tokens);
        }
        body
    }
}

/// Extract complete `\n`-terminated lines from `buf`, feeding each SSE `data:`
/// frame to the accumulator. Returns `true` once a `[DONE]` sentinel is seen.
fn drain_lines(
    buf: &mut Vec<u8>,
    acc: &mut Accumulator,
    on_text: &mut impl FnMut(&str),
    on_reasoning: &mut impl FnMut(&str),
    on_tool_call: &mut impl FnMut(WireToolCallDelta),
) -> bool {
    let mut done = false;
    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buf.drain(..=pos).collect();
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end_matches(['\r', '\n']);
        let Some(rest) = line.strip_prefix("data:") else {
            continue; // comments / `event:` / `id:` lines: ignore
        };
        let payload = rest.trim();
        if payload.is_empty() {
            continue;
        }
        if payload == "[DONE]" {
            done = true;
            break;
        }
        if let Ok(chunk) = serde_json::from_str::<Value>(payload) {
            acc.push_chunk(&chunk, on_text, on_reasoning, on_tool_call);
        }
    }
    done
}

/// One provider-native tool-call delta, forwarded without waiting for the
/// complete arguments object. The agent loop already has a typed streaming
/// variant for this shape; retaining it here lets terminal tools such as
/// `final_answer` render their string payload while OpenRouter generates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WireToolCallDelta {
    pub index: usize,
    pub id_delta: Option<String>,
    pub name_delta: Option<String>,
    pub arguments_delta: Option<String>,
}

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
