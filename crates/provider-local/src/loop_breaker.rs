//! Progress advisory for an agent re-running the same action and getting the
//! same result.
//!
//! ## Why
//!
//! The local loop normally runs until a natural end-of-turn, cancellation, or
//! a typed provider/tool failure. Nothing in the core loop looks at whether
//! successive turns make *progress*, so a model that re-runs the same
//! diagnostic in a circle — `ls`, re-seed, lock-check, `ls`, … — can keep
//! consuming work without converging.
//!
//! Clark Code's *cloud* loop already guards against this with typed stop reasons
//! (`identical_tool_call`, `fruitless_tool_streak`, `stale_evidence`, …).
//! This plugin ports the idea to the *local* loop using only the public
//! `agent-loop` plugin hooks, so it needs no fork of the pinned core.
//!
//! ## Signal (why keying on the result matters)
//!
//! A call is flagged only when the *same tool with the same arguments* has
//! already produced the *same result* several times in the recent window.
//! Keying on the result — not just the call — is what keeps legitimate
//! repetition safe:
//!
//! - Re-running `cargo test` after each edit is fine: the output changes,
//!   so the results are not identical.
//! - Re-running `ls` on an unchanging empty directory is not fine: the
//!   result is byte-identical every time and advances nothing.
//!
//! This is strictly smarter than opencode's "3 identical tool inputs in one
//! message" doom-loop check (which is per-message and result-blind) and than
//! zcode (which has no semantic loop detection at all).
//!
//! ## Behavior
//!
//! Once a call has yielded the same result [`Self::nudge_at`] times, append a
//! one-line note to the result the model sees, telling it to change approach
//! or use a bounded wait if it is intentionally polling. The host never blocks
//! the tool from this heuristic: an autonomous run may legitimately observe
//! unchanged external state many times over days or weeks. Explicit user
//! cancellation and typed provider/tool failures remain the stop authorities.

use std::collections::HashMap;

use agent_loop as ca;
use async_trait::async_trait;
use ca::plugin::{
    AfterToolCall, AfterToolCallContext, AfterToolDecision, Plugin, PluginCapabilities,
};
use ca::tool::ToolResult;
use ca::types::{AgentMessage, TextContent, ToolResultBlock};
use serde_json::Value;

/// Same tool+args+result seen this many times ⇒ start nudging (soft). Set
/// high enough that a short intentional poll loop just gets a heads-up, not
/// a false alarm.
const DEFAULT_NUDGE_AT: usize = 3;
/// Only consider this many most-recent completed tool calls when counting
/// repeats, so an early identical pair can't haunt a long, healthy run.
const DEFAULT_WINDOW: usize = 30;

/// Marker prefixing every note this plugin injects. Used both as the
/// user-visible tag and as the delimiter that [`output_identity`] strips so
/// the plugin's own annotations never change a result's identity (otherwise
/// each nudge would make the next result look "different" and defeat
/// detection).
const GUARD_MARK: &str = "[agent:loop-guard]";

/// Advises on repeated same-action/same-result observations. See module docs.
pub struct LoopProgressAdvisory {
    nudge_at: usize,
    window: usize,
}

impl Default for LoopProgressAdvisory {
    fn default() -> Self {
        Self {
            nudge_at: DEFAULT_NUDGE_AT,
            window: DEFAULT_WINDOW,
        }
    }
}

impl LoopProgressAdvisory {
    pub fn new() -> Self {
        Self::default()
    }

    /// One prior tool execution, reduced to what the detector compares on.
    fn prior_calls(messages: &[AgentMessage]) -> Vec<PriorCall> {
        // First pass: map every tool_call_id to its call fingerprint. The
        // arguments live on the assistant `ToolCall` block, not on the
        // `ToolResult`, so we can only fingerprint a result by joining it
        // back to the call that produced it.
        let mut fp_by_id: HashMap<&str, String> = HashMap::new();
        for m in messages {
            if let AgentMessage::Assistant { content, .. } = m {
                for call in content.tool_calls() {
                    fp_by_id.insert(call.id.as_str(), fingerprint(&call.name, &call.arguments));
                }
            }
        }
        // Second pass: emit completed calls in transcript order.
        let mut out = Vec::new();
        for m in messages {
            if let AgentMessage::ToolResult {
                tool_call_id,
                content,
                ..
            } = m
            {
                // Identical error output is captured by `identity` just like
                // identical success output, so `is_error` needs no special case.
                if let Some(fp) = fp_by_id.get(tool_call_id.as_str()) {
                    out.push(PriorCall {
                        fp: fp.clone(),
                        identity: output_identity(&content.plain_text()).to_string(),
                    });
                }
            }
        }
        out
    }

    /// How many times, within the recent window, the fingerprint `fp` has
    /// already produced exactly `identity`.
    fn repeat_count(&self, messages: &[AgentMessage], fp: &str, identity: &str) -> usize {
        let all = Self::prior_calls(messages);
        let start = all.len().saturating_sub(self.window);
        all[start..]
            .iter()
            .filter(|c| c.fp == fp && c.identity == identity)
            .count()
    }
}

/// One completed tool call, reduced to its comparison keys.
struct PriorCall {
    fp: String,
    identity: String,
}

impl Plugin for LoopProgressAdvisory {
    fn name(&self) -> &'static str {
        "loop_progress_advisory"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities {
            after_tool_call: true,
            ..PluginCapabilities::default()
        }
    }
}

#[async_trait]
impl AfterToolCall for LoopProgressAdvisory {
    async fn on_after_tool_call(&self, ctx: AfterToolCallContext<'_>) -> AfterToolDecision {
        let fp = fingerprint(&ctx.tool_call.name, ctx.args);
        let identity = output_identity(&result_plain_text(ctx.result)).to_string();
        // History does not yet include this result, so prior + 1 (this one)
        // is the running total of identical results.
        let total = self.repeat_count(ctx.messages, &fp, &identity) + 1;
        if total < self.nudge_at {
            return AfterToolDecision::passthrough();
        }
        let note = format!(
            "\n\n{GUARD_MARK} Heads up: this exact action has now returned the same result \
             {total} times. If you are waiting for something to change, add a short bounded wait \
             or a different check rather than repeating the identical call. Otherwise this isn't \
             making progress — try a materially different approach, or stop and tell the user what \
             you found and what's blocking you."
        );
        let mut result = ctx.result.clone();
        result
            .content
            .push(ToolResultBlock::Text(TextContent { text: note }));
        AfterToolDecision::override_result(result)
    }
}

/// Stable identity for a tool call: name + canonicalized arguments. Two
/// calls collide iff they would do the same thing.
fn fingerprint(name: &str, args: &Value) -> String {
    format!("{name}\u{0}{}", canonical(args))
}

/// Deterministic JSON string with object keys sorted, so semantically equal
/// arguments produce equal fingerprints regardless of key order.
fn canonical(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut s = String::from("{");
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&Value::String((*k).clone()).to_string());
                s.push(':');
                s.push_str(&canonical(&map[*k]));
            }
            s.push('}');
            s
        }
        Value::Array(a) => {
            let mut s = String::from("[");
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&canonical(e));
            }
            s.push(']');
            s
        }
        other => other.to_string(),
    }
}

/// A result's identity for repeat comparison: the tool's real output with
/// any loop-guard annotation stripped (so this plugin's own notes never
/// change what counts as "the same result").
fn output_identity(text: &str) -> &str {
    match text.find(GUARD_MARK) {
        Some(i) => text[..i].trim_end(),
        None => text.trim_end(),
    }
}

/// Concatenate the text blocks of an execution result (images ignored).
fn result_plain_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|b| match b {
            ToolResultBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ca::tool::{ToolCall, ToolResult};
    use ca::types::{AssistantContent, StopReason, ToolResultContent};
    use serde_json::json;

    /// Build an (assistant tool-call, tool-result) message pair for a call
    /// with the given name/args/output, so tests can assemble transcripts
    /// the way the loop stores them.
    fn call_pair(
        id: &str,
        name: &str,
        args: Value,
        output: &str,
        is_error: bool,
    ) -> Vec<AgentMessage> {
        let assistant = AgentMessage::Assistant {
            content: AssistantContent::with_tool_calls(
                None,
                vec![ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: args,
                }],
            ),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: None,
            usage: None,
        };
        let result = AgentMessage::ToolResult {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            content: ToolResultContent::text(output),
            is_error,
            narration: None,
            details: None,
            timestamp: None,
        };
        vec![assistant, result]
    }

    fn history(pairs: impl IntoIterator<Item = Vec<AgentMessage>>) -> Vec<AgentMessage> {
        pairs.into_iter().flatten().collect()
    }

    #[test]
    fn repeated_observations_have_no_before_tool_veto() {
        let advisory = LoopProgressAdvisory::new();
        let capabilities = advisory.capabilities();
        assert!(!capabilities.before_tool_call);
        assert!(capabilities.after_tool_call);
    }

    #[tokio::test]
    async fn nudge_fires_at_threshold_and_preserves_output() {
        let breaker = LoopProgressAdvisory::new(); // nudge_at = 3
        let args = json!({"cmd": "ls"});
        // 2 prior identical → this (3rd) result should be nudged.
        let messages = history(
            (0..2).map(|i| call_pair(&format!("c{i}"), "shell", args.clone(), "same\n", false)),
        );
        let call = ToolCall {
            id: "c2".into(),
            name: "shell".into(),
            arguments: args.clone(),
        };
        let result = ToolResult::text("same\n");
        let assistant = AgentMessage::Assistant {
            content: AssistantContent::text(""),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: None,
            usage: None,
        };
        let ctx = AfterToolCallContext {
            assistant_message: &assistant,
            tool_call: &call,
            args: &args,
            result: &result,
            is_error: false,
            messages: &messages,
        };
        let decision = breaker.on_after_tool_call(ctx).await;
        let overridden = decision
            .result
            .expect("3rd identical result should be annotated");
        let text = result_plain_text(&overridden);
        assert!(text.starts_with("same\n"), "real output preserved");
        assert!(text.contains(GUARD_MARK), "nudge appended");
        // And the appended note must not change the result's identity, so
        // detection keeps working on subsequent turns.
        assert_eq!(output_identity(&text), "same");
    }

    #[tokio::test]
    async fn no_nudge_before_threshold() {
        let breaker = LoopProgressAdvisory::new();
        let args = json!({"cmd": "ls"});
        let messages: Vec<AgentMessage> = Vec::new(); // first call ever
        let call = ToolCall {
            id: "c0".into(),
            name: "shell".into(),
            arguments: args.clone(),
        };
        let result = ToolResult::text("hello\n");
        let assistant = AgentMessage::Assistant {
            content: AssistantContent::text(""),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: None,
            usage: None,
        };
        let ctx = AfterToolCallContext {
            assistant_message: &assistant,
            tool_call: &call,
            args: &args,
            result: &result,
            is_error: false,
            messages: &messages,
        };
        let decision = breaker.on_after_tool_call(ctx).await;
        assert!(
            decision.result.is_none(),
            "first result must pass through untouched"
        );
    }

    #[test]
    fn canonical_is_key_order_independent() {
        assert_eq!(
            fingerprint("t", &json!({"a": 1, "b": 2})),
            fingerprint("t", &json!({"b": 2, "a": 1})),
        );
    }

    #[test]
    fn output_identity_strips_guard_note() {
        let annotated = format!("real output\n\n{GUARD_MARK} some note");
        assert_eq!(output_identity(&annotated), "real output");
    }
}
