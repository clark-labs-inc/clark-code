use std::{collections::BTreeMap, time::Duration};

use serde_json::Value;

use super::{AssistantTurn, ProviderResponseMetadata, TokenUsage, WireToolCall, WireToolCallDelta};

/// Reassembles streamed `chat.completion.chunk`s into one [`AssistantTurn`].
/// Tool calls arrive fragmented (an opening delta with `index`/`id`/`name`, then
/// a run of `arguments` string fragments) and are buffered per index.
#[derive(Default)]
pub(super) struct Accumulator {
    text: String,
    reasoning: String,
    reasoning_details: Vec<Value>,
    tool_calls: BTreeMap<u64, PartialToolCall>,
    pub(super) finish_reason: Option<String>,
    usage: Option<TokenUsage>,
    pub(super) stream_error: Option<StreamFailure>,
    response_metadata: Option<ProviderResponseMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StreamFailure {
    pub(super) message: String,
    pub(super) code: Option<u16>,
    pub(super) error_type: Option<String>,
    pub(super) retry_after: Option<Duration>,
}

impl StreamFailure {
    pub(super) fn is_rate_limited(&self) -> bool {
        self.code == Some(429) || self.error_type.as_deref() == Some("rate_limit_exceeded")
    }

    pub(super) fn is_transient(&self) -> bool {
        self.code.is_some_and(|code| {
            code == 408 || code == 425 || code == 524 || (500..=504).contains(&code)
        }) || matches!(
            self.error_type.as_deref(),
            Some(
                "provider_unavailable" | "upstream_error" | "upstream_timeout" | "request_timeout"
            )
        )
    }
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl Accumulator {
    pub(super) fn push_chunk(
        &mut self,
        chunk: &Value,
        on_text: &mut impl FnMut(&str),
        on_reasoning: &mut impl FnMut(&str),
        on_tool_call: &mut impl FnMut(WireToolCallDelta),
    ) {
        let metadata = self
            .response_metadata
            .get_or_insert_with(ProviderResponseMetadata::default);
        if metadata.generation_id.is_none() {
            metadata.generation_id = chunk.get("id").and_then(Value::as_str).map(str::to_string);
        }
        if metadata.resolved_model.is_none() {
            metadata.resolved_model = chunk
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if metadata.provider.is_none() {
            metadata.provider = chunk
                .get("provider")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        // OpenRouter cannot change the HTTP status after committing an SSE
        // response, so provider failures arrive in-band as a top-level `error`
        // object (usually alongside `finish_reason: "error"`). Preserve that
        // contract instead of silently discarding the only event and later
        // misreporting it as an empty assistant turn.
        if let Some(error) = chunk.get("error").and_then(Value::as_object) {
            let code_label = error.get("code").map(|value| match value {
                Value::String(value) => value.clone(),
                other => other.to_string(),
            });
            let code = error.get("code").and_then(|value| match value {
                Value::Number(value) => value.as_u64().and_then(|value| value.try_into().ok()),
                Value::String(value) => value.parse().ok(),
                _ => None,
            });
            let error_type = error
                .get("metadata")
                .and_then(Value::as_object)
                .and_then(|metadata| metadata.get("error_type"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let retry_after = error
                .get("metadata")
                .and_then(Value::as_object)
                .and_then(retry_after_from_metadata);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("provider stream failed");
            let label = [code_label.as_deref(), error_type.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            let message = if label.is_empty() {
                format!("model stream error: {message}")
            } else {
                format!("model stream error ({label}): {message}")
            };
            self.stream_error = Some(StreamFailure {
                message,
                code,
                error_type,
                retry_after,
            });
        }
        // The final chunk carries the whole call's usage (include_usage is set
        // upstream by the passthrough). Read it before the choices guard — some
        // providers ship usage in a chunk with no/empty choices.
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            if let Some(details) = usage
                .get("prompt_tokens_details")
                .and_then(Value::as_object)
            {
                metadata.cached_prompt_tokens =
                    details.get("cached_tokens").and_then(Value::as_u64);
                metadata.cache_write_tokens =
                    details.get("cache_write_tokens").and_then(Value::as_u64);
            }
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
            if let Some(reason) = choice.get("native_finish_reason").and_then(Value::as_str) {
                metadata.native_finish_reason = Some(reason.to_string());
            }
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
            if let Some(details) = delta.get("reasoning_details").and_then(Value::as_array) {
                self.reasoning_details.extend(details.iter().cloned());
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for tc in calls {
                    let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let entry = self.tool_calls.entry(index).or_default();
                    let id_delta = tc
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string);
                    if let Some(id) = &id_delta {
                        if !id.is_empty() {
                            entry.id.push_str(id);
                        }
                    }
                    let mut name_delta = None;
                    let mut arguments_delta = None;
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(Value::as_str) {
                            if !name.is_empty() {
                                entry.name.push_str(name);
                                name_delta = Some(name.to_string());
                            }
                        }
                        if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                            if !args.is_empty() {
                                entry.arguments.push_str(args);
                                arguments_delta = Some(args.to_string());
                            }
                        }
                    }
                    if id_delta.is_some() || name_delta.is_some() || arguments_delta.is_some() {
                        on_tool_call(WireToolCallDelta {
                            index: index.try_into().unwrap_or(usize::MAX),
                            id_delta,
                            name_delta,
                            arguments_delta,
                        });
                    }
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(reason.to_string());
            }
        }
    }

    pub(super) fn finish(self) -> AssistantTurn {
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
            reasoning_details: self.reasoning_details,
            response_metadata: self.response_metadata,
        }
    }

    pub(super) fn emitted_output(&self) -> bool {
        !self.text.is_empty()
            || !self.reasoning.is_empty()
            || !self.reasoning_details.is_empty()
            || !self.tool_calls.is_empty()
    }

    pub(super) fn emitted_tool_call(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    pub(super) fn native_network_error(&self) -> bool {
        self.response_metadata
            .as_ref()
            .and_then(|metadata| metadata.native_finish_reason.as_deref())
            .is_some_and(|reason| reason.eq_ignore_ascii_case("network_error"))
    }
}

pub(super) fn retry_after_from_metadata(
    metadata: &serde_json::Map<String, Value>,
) -> Option<Duration> {
    ["retry_after_seconds", "retry_after_seconds_raw"]
        .into_iter()
        .filter_map(|key| metadata.get(key))
        .find_map(|value| match value {
            Value::Number(value) => value
                .as_u64()
                .or_else(|| value.as_f64().map(|seconds| seconds.ceil() as u64)),
            Value::String(value) => value.parse().ok(),
            _ => None,
        })
        .map(Duration::from_secs)
}
