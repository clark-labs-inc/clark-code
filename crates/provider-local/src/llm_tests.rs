use super::*;

#[test]
fn user_agent_identifies_clark_code_version_and_platform() {
    let user_agent = clark_code_user_agent();
    assert!(user_agent.starts_with(&format!("clark-code/{} (", env!("CARGO_PKG_VERSION"))));
    assert!(user_agent.ends_with(&format!(" {})", std::env::consts::ARCH)));
}

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
    let v = json!({"choices":[{"delta":{"reasoning":"primary","reasoning_content":"secondary"}}]});
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
    acc.push_chunk(&v, &mut |_| {}, &mut |_| {});
    assert_eq!(
        acc.stream_error
            .as_ref()
            .map(|error| error.message.as_str()),
        Some("model stream error (502 provider_unavailable): Provider disconnected unexpectedly")
    );
    assert_eq!(acc.finish_reason.as_deref(), Some("error"));
}

#[test]
fn kimi_k3_always_uses_its_mandatory_max_reasoning_contract() {
    let mut client = LlmClient::from_parts(
        "https://api.example.test/v1",
        "clark-code:kimi_k3",
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    for configured in [None, Some("high"), Some("xhigh")] {
        client.reasoning_effort = configured.map(str::to_string);
        assert_eq!(client.body(&[], &[])["reasoning_effort"], json!("max"));
    }
}

#[test]
fn other_models_preserve_the_configured_reasoning_effort() {
    let mut client = LlmClient::from_parts(
        "https://api.example.test/v1",
        "clark-code",
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    client.reasoning_effort = Some("xhigh".to_string());
    assert_eq!(client.body(&[], &[])["reasoning_effort"], json!("xhigh"));
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
