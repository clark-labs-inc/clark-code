use super::*;

#[tokio::test]
async fn desktop_sink_streams_a_written_document_into_the_snapshot_as_it_is_typed() {
    // The whole point of the feature: a reader sees the document while the model
    // is still emitting it. That means the sink wiring, not just the decoder —
    // the row has to be announced before the first payload delta can patch it,
    // and the deltas have to survive projection as one growing string.
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

    // Arguments stream in the schema's property order, so the path lands before
    // the content — exactly how a provider emits `write_file`.
    for (id_delta, name_delta, arguments_delta) in [
        (Some("call-w"), Some("write_"), None),
        (None, Some("file"), Some("{\"path\":\"proposal.md\",")),
        (None, None, Some("\"content\":\"# Proposal\\n\\n## Recomm")),
        (None, None, Some("endation\\n\\nStart here. ")),
    ] {
        ca::EventSink::emit(
            &sink,
            ca::AgentEvent::MessageUpdate {
                partial: empty_assistant(ca::StopReason::EndTurn, None),
                chunk: ca::AssistantStreamChunk::ToolCallDelta {
                    index: 0,
                    id_delta: id_delta.map(str::to_string),
                    name_delta: name_delta.map(str::to_string),
                    arguments_delta: arguments_delta.map(str::to_string),
                },
            },
        )
        .await;
    }

    let document = "# Proposal\n\n## Recommendation\n\nStart here. Done.";
    ca::EventSink::emit(
        &sink,
        ca::AgentEvent::ToolExecutionStart {
            tool_call_id: "call-w".into(),
            tool_name: "write_file".into(),
            args: json!({"path": "proposal.md", "content": document}),
        },
    )
    .await;

    let events = std::iter::from_fn(|| receive.try_recv().ok()).collect::<Vec<_>>();

    // The document must have reached the UI in pieces, not one final blob.
    let appended = events
        .iter()
        .filter_map(|event| match event {
            desktop::AgentEvent::ToolCallUpdate { patch, .. } if !patch.append_input.is_empty() => {
                Some(patch.append_input.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        appended.len() > 1,
        "expected the document to arrive incrementally, got {appended:?}"
    );
    assert!(
        appended[0].starts_with("# Proposal"),
        "the first visible piece should be the start of the document, got {:?}",
        appended[0]
    );

    // And projection has to join them into exactly the validated payload.
    let mut snapshot = agent_core::projection::Snapshot::default();
    for event in &events {
        agent_core::projection::apply(&mut snapshot, event);
    }
    let call = snapshot
        .tool_calls
        .get(&agent_core::ids::ToolCallId::new("call-w"))
        .expect("the streamed write should have a tool row");
    assert_eq!(call.streamed_input, document);
    // Streamed input is the tool's argument; its output is a separate channel.
    assert!(call.content.is_empty());
}

#[tokio::test]
async fn desktop_sink_never_streams_a_redacted_tool_payload() {
    // `redaction` scrubs keystroke payloads before persistence. A partial JSON
    // fragment cannot be scrubbed, so those tools must not stream at all.
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
    for (id_delta, name_delta, arguments_delta) in [
        (
            Some("call-t"),
            Some("computer_type_text"),
            Some("{\"text\":\"hunter2 "),
        ),
        (None, None, Some("and more\"}")),
    ] {
        ca::EventSink::emit(
            &sink,
            ca::AgentEvent::MessageUpdate {
                partial: empty_assistant(ca::StopReason::EndTurn, None),
                chunk: ca::AssistantStreamChunk::ToolCallDelta {
                    index: 0,
                    id_delta: id_delta.map(str::to_string),
                    name_delta: name_delta.map(str::to_string),
                    arguments_delta: arguments_delta.map(str::to_string),
                },
            },
        )
        .await;
    }

    let events = std::iter::from_fn(|| receive.try_recv().ok()).collect::<Vec<_>>();
    let mut snapshot = agent_core::projection::Snapshot::default();
    for event in &events {
        agent_core::projection::apply(&mut snapshot, event);
    }
    assert!(events.iter().all(|event| matches!(
        event,
        desktop::AgentEvent::ToolCallUpdate { patch, .. } if patch.append_input.is_empty()
    ) || !matches!(
        event,
        desktop::AgentEvent::ToolCallUpdate { .. }
    )));
    for call in snapshot.tool_calls.values() {
        assert_eq!(call.streamed_input, "");
        assert!(!call.streamed_input.contains("hunter2"));
    }
}
