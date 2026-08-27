use super::*;

#[test]
fn completed_reasoning_details_do_not_repeat_live_reasoning() {
    let mut turn = AssistantTurn {
        reasoning: "the live thought".into(),
        reasoning_details: vec![json!({
            "type": "reasoning.summary",
            "summary": "the live thought"
        })],
        ..AssistantTurn::default()
    };

    assert!(!should_surface_reasoning_details(&turn));

    turn.reasoning.clear();
    assert!(should_surface_reasoning_details(&turn));
}

#[tokio::test]
async fn desktop_sink_retains_readable_reasoning_details_for_resume_and_compaction() {
    let (send, receive) = async_channel::unbounded();
    let sink = DesktopEventSink::new(
        send,
        RunId::new("reasoning-run"),
        Arc::new(ToolRegistry::new(None)),
        None,
    );
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageUpdate {
            partial: empty_assistant(ca::StopReason::EndTurn, None),
            chunk: ca::AssistantStreamChunk::ReasoningDetails {
                delta: vec![
                    json!({
                        "type": "reasoning.summary",
                        "summary": "durable provider finding",
                        "format": "anthropic-claude-v1",
                        "index": 0
                    }),
                    json!({
                        "type": "reasoning.encrypted",
                        "data": "opaque-provider-payload",
                        "format": "google-gemini-v1",
                        "index": 1
                    }),
                ],
            },
        },
    )
    .await;

    assert!(matches!(
        receive.recv().await.unwrap(),
        desktop::AgentEvent::Trace { .. }
    ));
    assert!(matches!(
        receive.recv().await.unwrap(),
        desktop::AgentEvent::MessageStreamStarted { .. }
    ));
    assert!(matches!(
        receive.recv().await.unwrap(),
        desktop::AgentEvent::MessageChunk {
            delta: desktop::ContentBlock::Thinking { text },
            ..
        } if text == "durable provider finding"
    ));
    assert!(
        receive.try_recv().is_err(),
        "encrypted payload must stay hidden"
    );
}
