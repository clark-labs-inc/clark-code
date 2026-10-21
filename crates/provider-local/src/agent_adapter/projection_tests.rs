use super::*;

#[tokio::test]
async fn desktop_sink_preserves_stream_lifecycle_events_as_trace() {
    let (send, receive) = async_channel::bounded(2);
    let sink = DesktopEventSink::new(
        send,
        RunId::new("run-1"),
        Arc::new(ToolRegistry::new(None)),
        None,
    );
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageStart {
            message: ca::AgentMessage::User {
                content: ca::UserContent::Text("hello".into()),
                timestamp: None,
            },
        },
    )
    .await;

    let event = receive.recv().await.expect("trace event");
    match event {
        desktop::AgentEvent::Trace {
            source, payload, ..
        } => {
            assert_eq!(source, "agent_loop");
            assert_eq!(payload["type"], "message_start");
            assert_eq!(payload["message"]["content"], "hello");
        }
        other => panic!("expected trace event, got {other:?}"),
    }
}

#[tokio::test]
async fn desktop_sink_announces_each_compaction_checkpoint_once() {
    let (send, receive) = async_channel::unbounded();
    let sink = DesktopEventSink::new(
        send,
        RunId::new("run-1"),
        Arc::new(ToolRegistry::new(None)),
        None,
    );
    let before = vec![ca::AgentMessage::User {
        content: ca::UserContent::Text("old context ".repeat(100)),
        timestamp: None,
    }];
    let checkpoint = vec![ca::AgentMessage::User {
        content: ca::UserContent::Text("checkpoint one".into()),
        timestamp: None,
    }];

    for iteration in 1..=2 {
        ca::EventSink::emit(
            &sink,
            ca::AgentEvent::ContextTransformApplied {
                iteration,
                plugin: "checkpoint_compactor",
                before: before.clone(),
                after: checkpoint.clone(),
            },
        )
        .await;
    }

    let events = std::iter::from_fn(|| receive.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, desktop::AgentEvent::MessageChunk { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, desktop::AgentEvent::ContextCompacted { .. }))
            .count(),
        1
    );

    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::ContextTransformApplied {
            iteration: 3,
            plugin: "checkpoint_compactor",
            before,
            after: vec![ca::AgentMessage::User {
                content: ca::UserContent::Text("checkpoint two".into()),
                timestamp: None,
            }],
        },
    )
    .await;
    let new_events = std::iter::from_fn(|| receive.try_recv().ok()).collect::<Vec<_>>();
    assert!(new_events
        .iter()
        .any(|event| matches!(event, desktop::AgentEvent::MessageChunk { .. })));
    assert!(new_events
        .iter()
        .any(|event| matches!(event, desktop::AgentEvent::ContextCompacted { .. })));
}

#[tokio::test]
async fn desktop_sink_marks_text_with_tool_calls_as_commentary() {
    let (send, receive) = async_channel::unbounded();
    let sink = DesktopEventSink::new(
        send,
        RunId::new("run-1"),
        Arc::new(ToolRegistry::new(None)),
        None,
    );
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageEnd {
            message: ca::AgentMessage::Assistant {
                content: ca::AssistantContent::with_tool_calls(
                    Some("I found the config; I’ll inspect its callers next.".into()),
                    vec![ca::ToolCall {
                        id: "call-1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "src/main.rs"}),
                    }],
                ),
                stop_reason: ca::StopReason::ToolUse,
                error_message: None,
                timestamp: None,
                usage: None,
            },
        },
    )
    .await;

    assert!(matches!(
        receive.recv().await.expect("trace event"),
        desktop::AgentEvent::Trace { .. }
    ));
    assert!(matches!(
        receive.recv().await.expect("phase event"),
        desktop::AgentEvent::MessagePhase {
            phase: desktop::MessagePhase::Commentary,
            ..
        }
    ));
}

#[tokio::test]
async fn desktop_sink_preserves_each_upstream_assistant_message_boundary() {
    let (send, receive) = async_channel::unbounded();
    let sink = DesktopEventSink::new(
        send,
        RunId::new("run-1"),
        Arc::new(ToolRegistry::new(None)),
        None,
    );

    for chunks in [
        ["Everything else ", "passes."].as_slice(),
        ["All ", "green."].as_slice(),
    ] {
        ca::EventSink::emit(
            &sink,
            ca::AgentEvent::MessageStart {
                message: empty_assistant(ca::StopReason::EndTurn, None),
            },
        )
        .await;
        for delta in chunks {
            ca::EventSink::emit(
                &sink,
                ca::AgentEvent::MessageUpdate {
                    partial: empty_assistant(ca::StopReason::EndTurn, None),
                    chunk: ca::AssistantStreamChunk::Text {
                        delta: (*delta).into(),
                    },
                },
            )
            .await;
        }
    }

    let events = std::iter::from_fn(|| receive.try_recv().ok())
        .filter(|event| !matches!(event, desktop::AgentEvent::Trace { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, desktop::AgentEvent::MessageStreamStarted { .. }))
            .count(),
        2,
        "each upstream MessageStart must survive normalization: {events:?}",
    );
    let snapshot = agent_core::reduce_all(&events);
    let messages = snapshot
        .timeline
        .iter()
        .filter_map(|item| match item {
            agent_core::TimelineItem::Message { blocks, .. } => Some(blocks),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0],
        &vec![desktop::ContentBlock::text("Everything else passes.")]
    );
    assert_eq!(
        messages[1],
        &vec![desktop::ContentBlock::text("All green.")]
    );
}

#[tokio::test]
async fn desktop_sink_streams_natural_answer_without_a_tool_row_or_duplicate() {
    let (send, receive) = async_channel::unbounded();
    let sink = DesktopEventSink::new(
        send,
        RunId::new("run-1"),
        Arc::new(ToolRegistry::new(None)),
        None,
    );
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageStart {
            message: empty_assistant(ca::StopReason::EndTurn, None),
        },
    )
    .await;
    for text in ["Fixed ", "and verified."] {
        ca::EventSink::emit(
            &sink,
            ca::AgentEvent::MessageUpdate {
                partial: empty_assistant(ca::StopReason::EndTurn, None),
                chunk: ca::AssistantStreamChunk::Text {
                    delta: text.to_string(),
                },
            },
        )
        .await;
    }
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageEnd {
            message: assistant_message(AssistantTurn {
                text: "Fixed and verified.".into(),
                ..Default::default()
            }),
        },
    )
    .await;
    let events = std::iter::from_fn(|| receive.try_recv().ok()).collect::<Vec<_>>();
    let chunks = events
        .iter()
        .filter_map(|event| match event {
            desktop::AgentEvent::MessageChunk {
                delta: desktop::ContentBlock::Text { text },
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(chunks, ["Fixed ", "and verified."]);
    assert!(!events.iter().any(|event| matches!(
        event,
        desktop::AgentEvent::ToolCall { .. } | desktop::AgentEvent::ToolCallUpdate { .. }
    )));
}

#[tokio::test]
async fn desktop_sink_announces_an_ordinary_tool_while_arguments_stream() {
    let (send, receive) = async_channel::unbounded();
    let sink = DesktopEventSink::new(
        send,
        RunId::new("run-1"),
        Arc::new(ToolRegistry::new(None)),
        None,
    );
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageStart {
            message: empty_assistant(ca::StopReason::EndTurn, None),
        },
    )
    .await;
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageUpdate {
            partial: empty_assistant(ca::StopReason::EndTurn, None),
            chunk: ca::AssistantStreamChunk::ToolCallDelta {
                index: 0,
                id_delta: Some("call-1".into()),
                name_delta: Some("read_file".into()),
                arguments_delta: Some("{\"path\":".into()),
            },
        },
    )
    .await;

    let events = std::iter::from_fn(|| receive.try_recv().ok()).collect::<Vec<_>>();
    let call = events
        .iter()
        .find_map(|event| match event {
            desktop::AgentEvent::ToolCall { call, .. } => Some(call),
            _ => None,
        })
        .unwrap_or_else(|| panic!("streamed tool row missing: {events:?}"));
    assert_eq!(call.id, ToolCallId::new("call-1"));
    assert_eq!(call.tool_name.as_deref(), Some("read_file"));
    assert_eq!(call.title, "Reading a file");
    assert_eq!(call.status, desktop::ToolStatus::Pending);
    assert_eq!(call.raw_input, None);

    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::ToolExecutionStart {
            tool_call_id: "call-1".into(),
            tool_name: "read_file".into(),
            args: json!({"path": "src/main.rs"}),
        },
    )
    .await;
    let execution_events = std::iter::from_fn(|| receive.try_recv().ok()).collect::<Vec<_>>();
    let definitive = execution_events
        .iter()
        .find_map(|event| match event {
            desktop::AgentEvent::ToolCallUpdate { id, patch, .. } => Some((id, patch)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("execution tool update missing: {execution_events:?}"));
    assert_eq!(definitive.0, &ToolCallId::new("call-1"));
    assert_eq!(definitive.1.title.as_deref(), Some("Read src/main.rs"));
    assert_eq!(definitive.1.raw_input, Some(json!({"path": "src/main.rs"})));

    let snapshot = agent_core::reduce_all(events.iter().chain(execution_events.iter()));
    assert_eq!(snapshot.tool_calls.len(), 1);
    let projected = &snapshot.tool_calls[&ToolCallId::new("call-1")];
    assert_eq!(projected.title, "Read src/main.rs");
    assert_eq!(projected.raw_input, Some(json!({"path": "src/main.rs"})));
}

#[tokio::test]
async fn desktop_sink_captures_only_canonical_completed_turns() {
    let (send, _receive) = async_channel::unbounded();
    let sink = DesktopEventSink::new(
        send,
        RunId::new("run-1"),
        Arc::new(ToolRegistry::new(None)),
        None,
    );
    let completed = sink.completed_transcript();
    let user = ca::AgentMessage::User {
        content: ca::UserContent::Text("inspect it".into()),
        timestamp: None,
    };
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageEnd {
            message: user.clone(),
        },
    )
    .await;

    // A fully streamed assistant message is still provisional until TurnEnd:
    // max-token retries and tool execution failures both stop before that gate.
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageEnd {
            message: ca::AgentMessage::Assistant {
                content: ca::AssistantContent::text("discarded attempt"),
                stop_reason: ca::StopReason::MaxTokens,
                error_message: None,
                timestamp: None,
                usage: None,
            },
        },
    )
    .await;

    let assistant = ca::AgentMessage::Assistant {
        content: ca::AssistantContent::text("completed turn"),
        stop_reason: ca::StopReason::EndTurn,
        error_message: None,
        timestamp: None,
        usage: None,
    };
    let tool_result = ca::AgentMessage::ToolResult {
        tool_call_id: "call-1".into(),
        tool_name: "read_file".into(),
        content: ca::ToolResultContent::text("contents"),
        is_error: false,
        narration: None,
        details: None,
        timestamp: None,
    };
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::TurnEnd {
            message: assistant.clone(),
            tool_results: vec![tool_result.clone()],
        },
    )
    .await;
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::MessageEnd {
            message: ca::AgentMessage::Assistant {
                content: ca::AssistantContent::text("transport error"),
                stop_reason: ca::StopReason::Error,
                error_message: Some("disconnected".into()),
                timestamp: None,
                usage: None,
            },
        },
    )
    .await;

    assert_eq!(completed.snapshot(), vec![user, assistant, tool_result]);
    assert!(completed.has_final_answer());
}
