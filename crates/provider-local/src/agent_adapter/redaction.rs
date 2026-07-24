use clark_agent as ca;
use serde_json::Value;

const REDACTED_TEXT: &str = "[redacted computer input]";
const REDACTED_VALUE: &str = "[redacted computer value]";
const REDACTED_KEY: &str = "[redacted character key]";

pub(crate) fn persisted_tool_args(tool_name: &str, args: &Value) -> Value {
    let mut redacted = args.clone();
    let Some(object) = redacted.as_object_mut() else {
        return redacted;
    };
    match tool_name {
        "computer_type_text" => redact_field(object, "text", REDACTED_TEXT),
        "computer_set_value" => redact_field(object, "value", REDACTED_VALUE),
        "computer_keypress"
            if object
                .get("key")
                .and_then(Value::as_str)
                .is_some_and(|key| key.chars().count() == 1) =>
        {
            redact_field(object, "key", REDACTED_KEY)
        }
        _ => {}
    }
    redacted
}

pub(crate) fn restore_runtime_payload(tool_name: &str, original: &Value, updated: Value) -> Value {
    let fields: &[&str] = match tool_name {
        "computer_type_text" => &["text"],
        "computer_set_value" => &["value"],
        "computer_keypress"
            if original
                .get("key")
                .and_then(Value::as_str)
                .is_some_and(|key| key.chars().count() == 1) =>
        {
            &["key"]
        }
        _ => &[],
    };
    let Some(updated_object) = updated.as_object() else {
        return updated;
    };
    let mut restored = updated_object.clone();
    for field in fields {
        if let Some(value) = original.get(*field) {
            restored.insert((*field).to_string(), value.clone());
        }
    }
    Value::Object(restored)
}

pub(crate) fn messages(messages: Vec<ca::AgentMessage>) -> Vec<ca::AgentMessage> {
    messages.into_iter().map(message).collect()
}

pub(crate) fn message(mut message: ca::AgentMessage) -> ca::AgentMessage {
    if let ca::AgentMessage::Assistant { content, .. } = &mut message {
        for block in &mut content.blocks {
            if let ca::AssistantBlock::ToolCall(call) = block {
                call.arguments = persisted_tool_args(&call.name, &call.arguments);
            }
        }
    }
    message
}

pub(crate) fn event(mut event: ca::AgentEvent) -> ca::AgentEvent {
    match &mut event {
        ca::AgentEvent::AgentEnd { messages: items } => redact_messages_in_place(items),
        ca::AgentEvent::ContextTransformApplied { before, after, .. } => {
            redact_messages_in_place(before);
            redact_messages_in_place(after);
        }
        ca::AgentEvent::TurnEnd {
            message: item,
            tool_results,
        } => {
            redact_message_in_place(item);
            redact_messages_in_place(tool_results);
        }
        ca::AgentEvent::MessageStart { message: item }
        | ca::AgentEvent::MessageEnd { message: item } => redact_message_in_place(item),
        ca::AgentEvent::MessageUpdate { partial, chunk } => {
            redact_message_in_place(partial);
            if let ca::AssistantStreamChunk::ToolCallDelta {
                arguments_delta, ..
            } = chunk
            {
                if arguments_delta.is_some() {
                    *arguments_delta = Some("[redacted streaming tool arguments]".to_string());
                }
            }
        }
        ca::AgentEvent::ToolExecutionStart {
            tool_name, args, ..
        } => *args = persisted_tool_args(tool_name, args),
        ca::AgentEvent::ProviderRequestPrepared {
            messages: items, ..
        } => redact_messages_in_place(items),
        ca::AgentEvent::AgentStart
        | ca::AgentEvent::RunIdentified { .. }
        | ca::AgentEvent::TurnStart
        | ca::AgentEvent::ToolExecutionUpdate { .. }
        | ca::AgentEvent::ToolExecutionEnd { .. }
        | ca::AgentEvent::OutputTokensEscalation { .. }
        | ca::AgentEvent::ToolGateApplied { .. }
        | ca::AgentEvent::ToolGateConflictResolved { .. } => {}
    }
    event
}

fn redact_messages_in_place(messages: &mut [ca::AgentMessage]) {
    for message in messages {
        redact_message_in_place(message);
    }
}

fn redact_message_in_place(message: &mut ca::AgentMessage) {
    if let ca::AgentMessage::Assistant { content, .. } = message {
        for block in &mut content.blocks {
            if let ca::AssistantBlock::ToolCall(call) = block {
                call.arguments = persisted_tool_args(&call.name, &call.arguments);
            }
        }
    }
}

fn redact_field(object: &mut serde_json::Map<String, Value>, key: &str, replacement: &str) {
    if object.contains_key(key) {
        object.insert(key.to_string(), Value::String(replacement.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_numeric_and_character_payloads_are_redacted() {
        assert_eq!(
            persisted_tool_args("computer_type_text", &json!({"text": "secret", "pid": 7})),
            json!({"text": REDACTED_TEXT, "pid": 7})
        );
        assert_eq!(
            persisted_tool_args("computer_set_value", &json!({"value": 42.5})),
            json!({"value": REDACTED_VALUE})
        );
        assert_eq!(
            persisted_tool_args("computer_keypress", &json!({"key": "x"})),
            json!({"key": REDACTED_KEY})
        );
        assert_eq!(
            persisted_tool_args("computer_keypress", &json!({"key": "return"})),
            json!({"key": "return"})
        );
        assert_eq!(
            restore_runtime_payload(
                "computer_type_text",
                &json!({"text": "secret", "pid": 7}),
                json!({"text": REDACTED_TEXT, "pid": 9})
            ),
            json!({"text": "secret", "pid": 9})
        );
    }
}
