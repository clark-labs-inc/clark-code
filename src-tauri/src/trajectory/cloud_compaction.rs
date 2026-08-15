use serde_json::Value;

/// Remove cumulative prompt/history snapshots before they enter the durable
/// cloud outbox. The canonical local runtime still owns its typed history and
/// opt-in eval prompt dumps; cloud trajectory rows carry only request geometry.
pub(super) fn compact_event(event_kind: &str, event: &mut Value) {
    let Some(payload) = event.get_mut("payload") else {
        return;
    };
    let has_data_envelope = payload.get("data").is_some();
    let target = if has_data_envelope {
        payload.get_mut("data").expect("checked data envelope")
    } else {
        payload
    };
    let Some(data) = target.as_object_mut() else {
        return;
    };

    if event_kind.ends_with("provider_request_prepared") {
        let message_count = data.get("messages").and_then(Value::as_array).map(Vec::len);
        let tool_count = data.get("tools").and_then(Value::as_array).map(Vec::len);
        let system_prompt_bytes = data
            .get("system_prompt")
            .and_then(Value::as_str)
            .map(str::len);
        let omitted = data.remove("system_prompt").is_some()
            | data.remove("messages").is_some()
            | data.remove("tools").is_some();
        if omitted {
            data.insert("request_content_omitted".into(), Value::Bool(true));
            insert_count(data, "message_count", message_count);
            insert_count(data, "tool_count", tool_count);
            insert_count(data, "system_prompt_bytes", system_prompt_bytes);
        }
    } else if event_kind.ends_with("context_transform_applied") {
        let before_count = data.get("before").and_then(Value::as_array).map(Vec::len);
        let after_count = data.get("after").and_then(Value::as_array).map(Vec::len);
        let omitted = data.remove("before").is_some() | data.remove("after").is_some();
        if omitted {
            data.insert("history_omitted".into(), Value::Bool(true));
            insert_count(data, "before_count", before_count);
            insert_count(data, "after_count", after_count);
        }
    }
}

fn insert_count(
    data: &mut serde_json::Map<String, Value>,
    key: &'static str,
    value: Option<usize>,
) {
    if let Some(value) = value {
        data.insert(key.into(), Value::from(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_request_upload_is_geometry_only() {
        let mut event = json!({
            "event": "trace",
            "source": "agent_loop",
            "payload": {
                "type": "provider_request_prepared",
                "data": {
                    "iteration": 2,
                    "system_prompt": "stable system",
                    "messages": [{"role": "user", "content": "new turn"}],
                    "tools": [{"name": "search"}],
                    "summary": {"input_bytes": 500}
                }
            }
        });

        compact_event("trace.agent_loop.provider_request_prepared", &mut event);

        let data = &event["payload"]["data"];
        assert_eq!(data["request_content_omitted"], true);
        assert_eq!(data["message_count"], 1);
        assert_eq!(data["tool_count"], 1);
        assert_eq!(data["system_prompt_bytes"], 13);
        assert_eq!(data["summary"]["input_bytes"], 500);
        assert!(data.get("system_prompt").is_none());
        assert!(data.get("messages").is_none());
        assert!(data.get("tools").is_none());
    }

    #[test]
    fn transform_upload_does_not_repeat_history() {
        let mut event = json!({
            "event": "trace",
            "payload": {
                "type": "context_transform_applied",
                "data": {
                    "before": [{"role": "user"}, {"role": "assistant"}],
                    "after": [{"role": "user"}],
                    "plugin": "token_budget"
                }
            }
        });

        compact_event("trace.agent_loop.context_transform_applied", &mut event);

        let data = &event["payload"]["data"];
        assert_eq!(data["history_omitted"], true);
        assert_eq!(data["before_count"], 2);
        assert_eq!(data["after_count"], 1);
        assert_eq!(data["plugin"], "token_budget");
        assert!(data.get("before").is_none());
        assert!(data.get("after").is_none());
    }
}
