use super::*;
use crate::llm::{AssistantTurn, LlmError, ProviderFailureContext};

fn recoverable(category: agent_core::recovery::ProviderIncidentCategory) -> LlmError {
    LlmError::Recoverable(ProviderFailureContext {
        category,
        message: "provider interrupted".into(),
        model: "test-model".into(),
        provider_route: "gateway.test".into(),
        provider_status: None,
        provider_error_type: None,
        idempotency_key: "request-1".into(),
        provider_request_id: Some("upstream-1".into()),
        attempts: 2,
        max_attempts: 17,
        retries: agent_core::recovery::ProviderRetryCounts {
            transient: 1,
            ..Default::default()
        },
        output_started: false,
        request_started_at_ms: 1,
        observed_at_ms: 2,
    })
}

fn turn(text: &str, reasoning: &str) -> AssistantTurn {
    AssistantTurn {
        text: text.to_string(),
        tool_calls: Vec::new(),
        finish_reason: Some("stop".into()),
        usage: None,
        reasoning: reasoning.to_string(),
        reasoning_details: Vec::new(),
        response_metadata: None,
    }
}

fn assistant_call(id: &str) -> ca::AgentMessage {
    ca::AgentMessage::Assistant {
        content: ca::AssistantContent::with_tool_calls(
            None,
            vec![ca::ToolCall {
                id: id.into(),
                name: "shell".into(),
                arguments: json!({}),
            }],
        ),
        stop_reason: ca::StopReason::ToolUse,
        error_message: None,
        timestamp: None,
        usage: None,
    }
}

fn tool_result(id: &str) -> ca::AgentMessage {
    ca::AgentMessage::ToolResult {
        tool_call_id: id.into(),
        tool_name: "shell".into(),
        content: ca::ToolResultContent::text("ok"),
        is_error: false,
        narration: None,
        details: None,
        timestamp: None,
    }
}

#[test]
fn duplicate_tool_call_ids_are_normalized_with_their_results_on_the_wire() {
    let messages = [
        assistant_call("shell:89"),
        tool_result("shell:89"),
        ca::AgentMessage::User {
            content: ca::UserContent::Text("continue".into()),
            timestamp: None,
        },
        assistant_call("shell:89"),
        tool_result("shell:89"),
    ];

    let wire = to_wire_messages("", &messages);

    assert_eq!(wire[0].tool_calls[0].id, "shell:89");
    assert_eq!(wire[1].tool_call_id.as_deref(), Some("shell:89"));
    assert_eq!(wire[3].tool_calls[0].id, "agent_loop_call_1");
    assert_eq!(wire[4].tool_call_id.as_deref(), Some("agent_loop_call_1"));
}

#[test]
fn assistant_message_keeps_reasoning_as_typed_block_out_of_plain_text() {
    let message = assistant_message(turn("the answer", "step by step thinking"));
    let ca::AgentMessage::Assistant { content, .. } = &message else {
        panic!("expected assistant message");
    };
    assert_eq!(
        reasoning_text(content).as_deref(),
        Some("step by step thinking")
    );
    assert_eq!(content.plain_text(), "the answer");
}

#[test]
fn reasoning_details_replay_unmodified_during_the_tool_exchange() {
    let details = vec![
        json!({
            "type": "reasoning.summary",
            "summary": "Inspect the exact provider contract.",
            "id": "summary-1",
            "format": "anthropic-claude-v1",
            "index": 0
        }),
        json!({
            "type": "reasoning.encrypted",
            "data": "opaque-signed-payload",
            "id": "encrypted-1",
            "format": "google-gemini-v1",
            "index": 1
        }),
    ];
    let mut turn = turn("working", "plaintext fallback");
    turn.reasoning_details = details.clone();
    let message = assistant_message(turn);
    let ca::AgentMessage::Assistant { content, .. } = &message else {
        panic!("expected assistant message");
    };
    assert_eq!(content.reasoning_details_values(), details);

    let wire = to_wire_messages("", &[message]);
    assert_eq!(wire[0].reasoning_details, details);
    assert_eq!(wire[0].reasoning, None);
}

#[test]
fn malformed_provider_reasoning_replays_byte_exact_during_the_tool_exchange() {
    let malformed = "The\n check is running i\nn the backg\nro\nund.";
    let raw_only = assistant_message(turn("working", malformed));
    let raw_wire = to_wire_messages("", &[raw_only]);
    assert_eq!(raw_wire[0].reasoning.as_deref(), Some(malformed));

    let details = vec![
        json!({
            "type": "reasoning.text",
            "text": "The\n check is running i\nn the ",
            "format": "unknown",
            "index": 0
        }),
        json!({
            "type": "reasoning.text",
            "text": "backg\nro\nund.",
            "format": "unknown",
            "index": 1
        }),
    ];
    let mut turn = turn("working", malformed);
    turn.reasoning_details = details.clone();
    let message = assistant_message(turn);

    let ca::AgentMessage::Assistant { content, .. } = &message else {
        panic!("expected assistant message");
    };
    assert_eq!(reasoning_text(content).as_deref(), Some(malformed));
    assert_eq!(content.reasoning_details_values(), details);

    let wire = to_wire_messages("", &[message]);
    assert_eq!(wire[0].reasoning_details, details);
    assert_eq!(wire[0].reasoning, None);
}

#[test]
fn reasoning_replays_only_for_the_in_flight_exchange() {
    let old_assistant = assistant_message(turn("old turn", "old reasoning"));
    let user = ca::AgentMessage::User {
        content: ca::UserContent::Text("new question".into()),
        timestamp: None,
    };
    let live_assistant = assistant_message(turn("working on it", "live reasoning"));

    let wire = to_wire_messages("sys", &[old_assistant, user, live_assistant]);

    assert_eq!(wire.len(), 4);
    assert_eq!(
        wire[1].reasoning, None,
        "reasoning from before the last user message must not replay"
    );
    assert_eq!(wire[3].reasoning.as_deref(), Some("live reasoning"));
}

#[test]
fn collaboration_instruction_uses_developer_role_on_the_wire() {
    let messages = [crate::planning::developer_instruction_message(
        "Plan Mode is active".into(),
    )];
    let wire = to_wire_messages("system", &messages);
    assert_eq!(wire.len(), 2);
    assert_eq!(wire[0].role, "system");
    assert_eq!(wire[1].role, "developer");
    assert!(matches!(
        &wire[1].content,
        Some(ChatContent::Text(text)) if text == "Plan Mode is active"
    ));
}

#[test]
fn typed_llm_failures_map_to_typed_stream_errors() {
    let (kind, _) = stream_error(LlmError::ContextOverflow("too large".into()));
    assert!(matches!(kind, ca::stream::StreamErrorKind::ContextOverflow));

    let (kind, _) = stream_error(recoverable(
        agent_core::recovery::ProviderIncidentCategory::ConnectionLost,
    ));
    assert!(matches!(kind, ca::stream::StreamErrorKind::Transient));

    let (kind, _) = stream_error(recoverable(
        agent_core::recovery::ProviderIncidentCategory::RateLimit,
    ));
    assert!(matches!(
        kind,
        ca::stream::StreamErrorKind::ProviderRateLimited
    ));

    let (kind, message) = stream_error(LlmError::PlatformKeyRejected("401 Unauthorized".into()));
    assert!(matches!(kind, ca::stream::StreamErrorKind::Fatal));
    assert!(message.starts_with("platform_key_rejected:"));

    let (kind, message) = stream_error(LlmError::InsufficientCredits);
    assert!(matches!(kind, ca::stream::StreamErrorKind::Fatal));
    assert_eq!(
        message,
        "insufficient_credits: the selected provider declined this run for insufficient usage access."
    );
    assert!(!message.contains("Your"));

    let (kind, message) = stream_error(LlmError::OutputQuarantined {
        reason: "reserved_protocol_marker",
        metadata: Box::default(),
    });
    assert!(matches!(kind, ca::stream::StreamErrorKind::Fatal));
    assert!(message.contains("data-isolation checks"));
    assert!(!message.contains("reserved_protocol_marker"));
}

#[test]
fn model_tool_result_preserves_large_text_byte_for_byte() {
    let expected = format!("begin:{}:end", "0123456789abcdef".repeat(8_000));
    let result = tool_result_from_outcome(
        crate::tools::ToolOutcome::ok(expected.clone()),
        false,
        false,
    );
    assert!(matches!(
        result.content.as_slice(),
        [ca::ToolResultBlock::Text(text)] if text.text == expected
    ));
}
