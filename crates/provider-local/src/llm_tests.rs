use super::*;

#[test]
fn user_agent_identifies_desktop_version_and_platform() {
    let user_agent = desktop_user_agent(env!("CARGO_PKG_VERSION"));
    assert!(user_agent.starts_with(&format!("agent-desktop/{} (", env!("CARGO_PKG_VERSION"))));
    assert!(user_agent.ends_with(&format!(" {})", std::env::consts::ARCH)));
}

#[test]
fn user_agent_accepts_the_composed_product_version() {
    assert!(desktop_user_agent("0.1.154").starts_with("agent-desktop/0.1.154 ("));
}

#[test]
fn response_start_deadline_bounds_only_a_request_that_never_starts() {
    assert_eq!(
        DEFAULT_MODEL_RESPONSE_START_TIMEOUT,
        Duration::from_secs(2 * 60)
    );
}

fn feed(frames: &[&str]) -> AssistantTurn {
    let mut acc = Accumulator::default();
    let mut sink = |_: &str| {};
    let mut rsink = |_: &str| {};
    for f in frames {
        let v: Value = serde_json::from_str(f).unwrap();
        acc.push_chunk(&v, &mut sink, &mut rsink, &mut |_| {});
    }
    acc.finish()
}

#[test]
fn accumulates_streamed_text() {
    let mut collected = String::new();
    let mut acc = Accumulator::default();
    for c in ["Hel", "lo ", "world"] {
        let v = json!({"choices":[{"delta":{"content": c}}]});
        acc.push_chunk(
            &v,
            &mut |s: &str| collected.push_str(s),
            &mut |_| {},
            &mut |_| {},
        );
    }
    let turn = acc.finish();
    assert_eq!(turn.text, "Hello world");
    assert_eq!(collected, "Hello world");
    assert!(turn.tool_calls.is_empty());
}

#[test]
fn captures_generation_and_resolved_model_from_stream() {
    let turn = feed(&[
        r#"{"id":"gen-123","model":"free/model-v1","provider":"example","choices":[{"delta":{"content":"done"}}]}"#,
    ]);
    let metadata = turn.response_metadata.expect("response metadata captured");
    assert_eq!(metadata.generation_id.as_deref(), Some("gen-123"));
    assert_eq!(metadata.resolved_model.as_deref(), Some("free/model-v1"));
    assert_eq!(metadata.provider.as_deref(), Some("example"));
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
        acc.push_chunk(
            &v,
            &mut |_| {},
            &mut |s: &str| collected.push_str(s),
            &mut |_| {},
        );
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
    acc.push_chunk(&v, &mut |_| {}, &mut |_| {}, &mut |_| {});
    assert_eq!(acc.finish().reasoning, "alt");
}

#[test]
fn reasoning_is_preferred_over_reasoning_content_when_both_present() {
    let mut acc = Accumulator::default();
    let v = json!({"choices":[{"delta":{"reasoning":"primary","reasoning_content":"secondary"}}]});
    acc.push_chunk(&v, &mut |_| {}, &mut |_| {}, &mut |_| {});
    assert_eq!(acc.finish().reasoning, "primary");
}

#[test]
fn accumulates_reasoning_details_in_stream_order_without_rewriting() {
    let first = json!({
        "type": "reasoning.summary",
        "summary": "First finding",
        "id": "r1",
        "format": "anthropic-claude-v1",
        "index": 0
    });
    let second = json!({
        "type": "reasoning.encrypted",
        "data": "opaque-provider-payload",
        "id": "r2",
        "format": "google-gemini-v1",
        "index": 1
    });
    let turn = feed(&[
        &json!({"choices":[{"delta":{"reasoning_details":[first.clone()]}}]}).to_string(),
        &json!({"choices":[{"delta":{"reasoning_details":[second.clone()]}}]}).to_string(),
    ]);
    assert_eq!(turn.reasoning_details, vec![first, second]);
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
fn captures_in_band_openrouter_stream_error() {
    let mut acc = Accumulator::default();
    let v = json!({
        "error": {
            "code": 502,
            "message": "Provider disconnected unexpectedly",
            "metadata": {"error_type": "provider_unavailable"}
        },
        "choices": [{"delta": {"content": ""}, "finish_reason": "error"}]
    });
    acc.push_chunk(&v, &mut |_| {}, &mut |_| {}, &mut |_| {});
    assert_eq!(
        acc.stream_error
            .as_ref()
            .map(|error| error.message.as_str()),
        Some("model stream error (502 provider_unavailable): Provider disconnected unexpectedly")
    );
    assert_eq!(acc.finish_reason.as_deref(), Some("error"));
}

#[test]
fn host_model_policy_overrides_a_weaker_session_reasoning_value() {
    let mut client = LlmClient::from_parts(
        "https://api.example.test/v1",
        "managed-model-large",
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    client
        .model_reasoning_efforts
        .insert("managed-model-large".into(), "max".into());
    for configured in [None, Some("high"), Some("xhigh")] {
        client.reasoning_effort = configured.map(str::to_string);
        assert_eq!(client.body(&[], &[])["reasoning_effort"], json!("max"));
    }
}

#[test]
fn every_host_advertised_model_uses_its_own_reasoning_policy() {
    for (model, expected) in [
        ("managed-model-standard", "high"),
        ("managed-model-large", "xhigh"),
        ("managed-model-research", "max"),
    ] {
        let mut client =
            LlmClient::from_parts("https://api.example.test/v1", model, None, Vec::new(), None)
                .unwrap();
        client.reasoning_effort = Some("low".to_string());
        client
            .model_reasoning_efforts
            .insert(model.to_string(), expected.to_string());
        assert_eq!(client.body(&[], &[])["reasoning_effort"], json!(expected));
    }
}

#[test]
fn host_output_ceiling_is_sent_on_every_model_request() {
    let mut client = LlmClient::from_parts(
        "https://api.example.test/v1",
        "managed-model-standard",
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    client.max_output_tokens = Some(16_384);

    assert_eq!(client.body(&[], &[])["max_tokens"], json!(16_384));
}

#[test]
fn openrouter_uses_unified_reasoning_and_strict_response_contracts() {
    let mut client = LlmClient::from_parts(
        "https://openrouter.ai/api/v1",
        "vendor/reasoning-model",
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    let response_format = json!({
        "type": "json_schema",
        "json_schema": {
            "name": "hypothesis",
            "strict": true,
            "schema": {"type": "object"}
        }
    });
    let provider = json!({"require_parameters": true});
    client.reasoning_effort = Some("max".to_string());
    client.response_format = Some(response_format.clone());
    client.provider_preferences = Some(provider.clone());

    let body = client.body(&[], &[]);
    assert_eq!(
        body["reasoning"],
        json!({"effort": "max", "exclude": false})
    );
    assert!(body.get("reasoning_effort").is_none());
    assert_eq!(body["response_format"], response_format);
    assert_eq!(body["provider"], provider);
}

#[test]
fn forced_tool_choice_uses_auto_or_named_singleton_on_the_wire() {
    let client = LlmClient::from_parts(
        "https://api.example.test/v1",
        "test-model",
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    let tools = [ToolSchema::function(
        "final_answer",
        "Deliver the answer.",
        json!({"type": "object"}),
    )];

    assert_eq!(client.body(&[], &tools)["tool_choice"], json!("auto"));
    assert_eq!(
        client.body_forced_tool(&[], &tools)["tool_choice"],
        json!("auto")
    );
    assert_eq!(
        client.body_requiring_named_tool(&[], &tools, "final_answer")["tool_choice"],
        json!({
            "type": "function",
            "function": { "name": "final_answer" },
        })
    );
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
    assert!(!drain_lines(
        &mut buf,
        &mut acc,
        &mut sink,
        &mut rsink,
        &mut |_| {},
    ));
    buf.extend_from_slice(b"\"}}]}\n");
    assert!(!drain_lines(
        &mut buf,
        &mut acc,
        &mut sink,
        &mut rsink,
        &mut |_| {},
    ));
    buf.extend_from_slice(b"data: [DONE]\n");
    assert!(drain_lines(
        &mut buf,
        &mut acc,
        &mut sink,
        &mut rsink,
        &mut |_| {},
    ));
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
