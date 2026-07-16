//! OpenAI-compatible streaming chat-completions client — the model seam.
//!
//! This is the only place that knows the model wire format. It speaks the
//! ubiquitous `POST {base}/chat/completions` contract (OpenRouter, vLLM,
//! llama.cpp, LM Studio, a future Clark passthrough, …): streamed
//! `chat.completion.chunk` SSE frames carrying assistant text deltas and
//! fragmented tool-call deltas. The parser reassembles those into a single
//! [`AssistantTurn`]; text streams out live via a callback for the UI.

use std::collections::BTreeMap;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::config::LocalConfig;

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
        }
    }
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(ChatContent::text(content)),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
    fn simple(role: &str, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(ChatContent::text(text)),
            tool_calls: Vec::new(),
            tool_call_id: None,
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

/// Token/cost accounting from the final streamed chunk (OpenRouter shape,
/// forwarded verbatim by the Clark passthrough).
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
    /// Display-only; never sent back to the model on the next turn.
    pub reasoning: String,
}

/// Why a model call ended without producing a turn.
#[derive(Debug)]
pub enum LlmError {
    /// The caller's cancellation token fired mid-request.
    Cancelled,
    /// The account is out of Clark credits (403). The UI prompts an upgrade.
    InsufficientCredits,
    /// Any other failure (transport, non-2xx, decode).
    Message(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Cancelled => f.write_str("model request cancelled"),
            LlmError::InsufficientCredits => f.write_str("insufficient_credits"),
            LlmError::Message(m) => f.write_str(m),
        }
    }
}

/// Streaming chat client bound to one endpoint/model.
#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
    headers: Vec<(String, String)>,
    temperature: Option<f32>,
    /// Reasoning-effort override forwarded to the passthrough ("low" … "xhigh").
    /// `None` → the server applies the model's default.
    reasoning_effort: Option<String>,
}

impl LlmClient {
    /// Same endpoint/auth, different model — for background side-calls (e.g.
    /// memory extraction) that shouldn't inherit a weaker session model.
    pub fn with_model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    pub fn new(config: &LocalConfig) -> Result<Self, String> {
        let mut client = Self::from_parts(
            &config.base_url,
            &config.model,
            config.api_key.clone(),
            config.headers.clone().into_iter().collect(),
            config.temperature,
        )?;
        client.reasoning_effort = config.reasoning_effort.clone();
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
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| format!("llm client build failed: {e}"))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key,
            headers,
            temperature,
            reasoning_effort: None,
        })
    }

    /// One-shot completion with NO client tools (the agentic Clark path): send an
    /// optional system + a user message, return the assembled assistant text.
    pub async fn complete(
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

    /// One-shot completion with image(s) attached — mirrors [`Self::complete`],
    /// swapping [`ChatMessage::user`] for [`ChatMessage::user_with_images`].
    /// Used by the vision-fallback path: a separate call to a vision-capable
    /// Clark model, not part of the coding model's own turn.
    pub async fn describe_images(
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

    fn body(&self, messages: &[ChatMessage], tools: &[ToolSchema]) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(tools).unwrap_or(Value::Null);
            body["tool_choice"] = json!("auto");
        }
        if let Some(t) = self.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(effort) = &self.reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }
        body
    }

    /// Stream one chat completion. `on_text` receives assistant text deltas as
    /// they arrive; `on_reasoning` receives hidden reasoning deltas (GLM's
    /// `delta.reasoning` / the `reasoning_content` alias) so the UI can surface
    /// a live Thinking block instead of silence. The assembled turn (text +
    /// tool calls + reasoning) is returned at the end. Honors `cancel` both
    /// before and during the stream.
    pub async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        cancel: &CancellationToken,
        mut on_text: impl FnMut(&str),
        mut on_reasoning: impl FnMut(&str),
    ) -> Result<AssistantTurn, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        let url = format!("{}/chat/completions", self.base_url);
        let mut req = self.http.post(&url).json(&self.body(messages, tools));
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        for (k, v) in &self.headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = tokio::select! {
            _ = cancel.cancelled() => return Err(LlmError::Cancelled),
            r = req.send() => r.map_err(|e| LlmError::Message(format!("model request failed: {e}")))?,
        };
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            // Clark returns 403 with a credits message when the wallet is empty.
            if status.as_u16() == 403 && body.to_lowercase().contains("credit") {
                return Err(LlmError::InsufficientCredits);
            }
            return Err(LlmError::Message(format!(
                "model endpoint returned {status}: {}",
                body.chars().take(500).collect::<String>()
            )));
        }

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut acc = Accumulator::default();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Cancelled),
                n = stream.next() => n,
            };
            match next {
                None => break,
                Some(Err(e)) => {
                    return Err(LlmError::Message(format!("model stream error: {e}")));
                }
                Some(Ok(bytes)) => {
                    buf.extend_from_slice(&bytes);
                    if drain_lines(&mut buf, &mut acc, &mut on_text, &mut on_reasoning) {
                        break; // saw [DONE]
                    }
                }
            }
        }
        Ok(acc.finish())
    }
}

/// Extract complete `\n`-terminated lines from `buf`, feeding each SSE `data:`
/// frame to the accumulator. Returns `true` once a `[DONE]` sentinel is seen.
fn drain_lines(
    buf: &mut Vec<u8>,
    acc: &mut Accumulator,
    on_text: &mut impl FnMut(&str),
    on_reasoning: &mut impl FnMut(&str),
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
            acc.push_chunk(&chunk, on_text, on_reasoning);
        }
    }
    done
}

/// Reassembles streamed `chat.completion.chunk`s into one [`AssistantTurn`].
/// Tool calls arrive fragmented (an opening delta with `index`/`id`/`name`, then
/// a run of `arguments` string fragments) and are buffered per index.
#[derive(Default)]
struct Accumulator {
    text: String,
    reasoning: String,
    tool_calls: BTreeMap<u64, PartialToolCall>,
    finish_reason: Option<String>,
    usage: Option<TokenUsage>,
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl Accumulator {
    fn push_chunk(
        &mut self,
        chunk: &Value,
        on_text: &mut impl FnMut(&str),
        on_reasoning: &mut impl FnMut(&str),
    ) {
        // The final chunk carries the whole call's usage (include_usage is set
        // upstream by the passthrough). Read it before the choices guard — some
        // providers ship usage in a chunk with no/empty choices.
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(TokenUsage {
                prompt_tokens: usage
                    .get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                completion_tokens: usage
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                cost_usd: usage.get("cost").and_then(Value::as_f64),
            });
        }
        let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
            return;
        };
        for choice in choices {
            let delta = choice.get("delta").unwrap_or(&Value::Null);
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    self.text.push_str(content);
                    on_text(content);
                }
            }
            // Hidden reasoning: GLM/OpenRouter streams it as `delta.reasoning`;
            // some providers use `delta.reasoning_content`. Forward each delta
            // live so the UI can render a Thinking block instead of silence.
            // Prefer `reasoning`, fall back to the `reasoning_content` alias.
            let reasoning = delta
                .get("reasoning")
                .and_then(Value::as_str)
                .or_else(|| delta.get("reasoning_content").and_then(Value::as_str));
            if let Some(reasoning) = reasoning {
                if !reasoning.is_empty() {
                    self.reasoning.push_str(reasoning);
                    on_reasoning(reasoning);
                }
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for tc in calls {
                    let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let entry = self.tool_calls.entry(index).or_default();
                    if let Some(id) = tc.get("id").and_then(Value::as_str) {
                        if !id.is_empty() {
                            entry.id = id.to_string();
                        }
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(Value::as_str) {
                            if !name.is_empty() {
                                entry.name.push_str(name);
                            }
                        }
                        if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                            entry.arguments.push_str(args);
                        }
                    }
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(reason.to_string());
            }
        }
    }

    fn finish(self) -> AssistantTurn {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .filter(|(_, tc)| !tc.name.is_empty())
            .enumerate()
            .map(|(i, (index, tc))| {
                let id = if tc.id.is_empty() {
                    format!("call_{index}_{i}")
                } else {
                    tc.id
                };
                WireToolCall::function(id, tc.name, tc.arguments)
            })
            .collect();
        AssistantTurn {
            text: self.text,
            tool_calls,
            finish_reason: self.finish_reason,
            usage: self.usage,
            reasoning: self.reasoning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(frames: &[&str]) -> AssistantTurn {
        let mut acc = Accumulator::default();
        let mut sink = |_: &str| {};
        let mut rsink = |_: &str| {};
        for f in frames {
            let v: Value = serde_json::from_str(f).unwrap();
            acc.push_chunk(&v, &mut sink, &mut rsink);
        }
        acc.finish()
    }

    #[test]
    fn accumulates_streamed_text() {
        let mut collected = String::new();
        let mut acc = Accumulator::default();
        for c in ["Hel", "lo ", "world"] {
            let v = json!({"choices":[{"delta":{"content": c}}]});
            acc.push_chunk(&v, &mut |s: &str| collected.push_str(s), &mut |_| {});
        }
        let turn = acc.finish();
        assert_eq!(turn.text, "Hello world");
        assert_eq!(collected, "Hello world");
        assert!(turn.tool_calls.is_empty());
    }

    #[test]
    fn accumulates_streamed_reasoning_and_forwards_it_live() {
        let mut collected = String::new();
        let mut acc = Accumulator::default();
        // GLM/OpenRouter shape: reasoning rides in `delta.reasoning`, separate
        // from `content`. Stream two reasoning deltas, then visible text.
        for (r, c) in [("Think", ""), ("ing…", ""), ("", "Answer")] {
            let mut delta = json!({});
            if !r.is_empty() {
                delta["reasoning"] = json!(r);
            }
            if !c.is_empty() {
                delta["content"] = json!(c);
            }
            let v = json!({"choices":[{"delta":delta}]});
            acc.push_chunk(&v, &mut |_| {}, &mut |s: &str| collected.push_str(s));
        }
        let turn = acc.finish();
        assert_eq!(turn.reasoning, "Thinking…");
        assert_eq!(turn.text, "Answer");
        assert_eq!(
            collected, "Thinking…",
            "reasoning deltas fire the callback live"
        );
    }

    #[test]
    fn reasoning_content_alias_is_also_read() {
        let mut acc = Accumulator::default();
        // Some providers stream reasoning as `delta.reasoning_content`.
        let v = json!({"choices":[{"delta":{"reasoning_content":"alt"}}]});
        acc.push_chunk(&v, &mut |_| {}, &mut |_| {});
        assert_eq!(acc.finish().reasoning, "alt");
    }

    #[test]
    fn reasoning_is_preferred_over_reasoning_content_when_both_present() {
        let mut acc = Accumulator::default();
        let v =
            json!({"choices":[{"delta":{"reasoning":"primary","reasoning_content":"secondary"}}]});
        acc.push_chunk(&v, &mut |_| {}, &mut |_| {});
        assert_eq!(acc.finish().reasoning, "primary");
    }

    #[test]
    fn captures_usage_from_the_final_chunk() {
        // Real shape: last chunk carries usage (choices present but empty delta).
        let turn = feed(&[
            r#"{"choices":[{"delta":{"content":"hi"}}]}"#,
            r#"{"choices":[{"delta":{"content":""},"finish_reason":"stop"}],"usage":{"prompt_tokens":14,"completion_tokens":181,"total_tokens":195,"cost":0.00055602}}"#,
        ]);
        let usage = turn.usage.expect("usage captured");
        assert_eq!(usage.prompt_tokens, 14);
        assert_eq!(usage.completion_tokens, 181);
        assert!((usage.cost_usd.unwrap() - 0.00055602).abs() < 1e-9);

        // Usage-only trailer chunk (no/empty choices) is also honored.
        let turn = feed(&[
            r#"{"choices":[{"delta":{"content":"x"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#,
        ]);
        let usage = turn.usage.expect("trailer usage captured");
        assert_eq!(usage.prompt_tokens, 7);
        assert_eq!(usage.cost_usd, None);
    }

    #[test]
    fn reassembles_fragmented_tool_call() {
        let turn = feed(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read_file","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a.rs\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert_eq!(turn.tool_calls.len(), 1);
        let tc = &turn.tool_calls[0];
        assert_eq!(tc.id, "call_a");
        assert_eq!(tc.function.name, "read_file");
        assert_eq!(tc.function.arguments, r#"{"path":"a.rs"}"#);
        assert_eq!(turn.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn reassembles_two_parallel_tool_calls_by_index() {
        let turn = feed(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c0","function":{"name":"read_file","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c1","function":{"name":"bash","arguments":"{}"}}]}}]}"#,
        ]);
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].id, "c0");
        assert_eq!(turn.tool_calls[1].id, "c1");
        assert_eq!(turn.tool_calls[1].function.name, "bash");
    }

    #[test]
    fn synthesizes_id_when_missing() {
        let turn = feed(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"grep","arguments":"{}"}}]}}]}"#,
        ]);
        assert_eq!(turn.tool_calls.len(), 1);
        assert!(!turn.tool_calls[0].id.is_empty());
    }

    #[test]
    fn drain_lines_handles_split_frames_and_done() {
        let mut acc = Accumulator::default();
        let mut sink = |_: &str| {};
        let mut rsink = |_: &str| {};
        // A frame split across two network chunks, then [DONE].
        let mut buf = Vec::new();
        buf.extend_from_slice(b"data: {\"choices\":[{\"delta\":{\"content\":\"hi");
        assert!(!drain_lines(&mut buf, &mut acc, &mut sink, &mut rsink));
        buf.extend_from_slice(b"\"}}]}\n");
        assert!(!drain_lines(&mut buf, &mut acc, &mut sink, &mut rsink));
        buf.extend_from_slice(b"data: [DONE]\n");
        assert!(drain_lines(&mut buf, &mut acc, &mut sink, &mut rsink));
        assert_eq!(acc.finish().text, "hi");
    }

    #[test]
    fn text_content_serializes_as_a_bare_string_like_before_multimodal_support() {
        let msg = ChatMessage::user("hello");
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["content"], json!("hello"));
    }

    #[test]
    fn tool_message_content_is_always_a_bare_string() {
        let msg = ChatMessage::tool("call_1", "done");
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], json!("tool"));
        assert_eq!(v["content"], json!("done"));
    }

    #[test]
    fn user_with_images_serializes_as_a_content_parts_array() {
        let msg = ChatMessage::user_with_images(
            "look at this",
            vec!["data:image/png;base64,QUJD".to_string()],
        );
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], json!("user"));
        assert_eq!(
            v["content"],
            json!([
                {"type": "text", "text": "look at this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJD"}},
            ])
        );
    }
}
