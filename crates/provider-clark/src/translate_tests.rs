use super::*;
use serde_json::json;

fn sess() -> SessionId {
    SessionId::new("conv-1")
}

fn run() -> RunId {
    RunId::new("r1")
}

#[test]
fn message_stream_delta_becomes_agent_text() {
    let ev = json!({"type":"message_stream_delta","data":{"delta":"pong"}});
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::MessageChunk { role, delta, .. } => {
            assert_eq!(role, Role::Agent);
            assert_eq!(delta, ContentBlock::text("pong"));
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn file_write_tool_call_maps_to_edit_work_line() {
    let ev = json!({
        "type":"tool_call",
        "data":{"tool_call_id":"t1","tool_name":"file",
                "arguments":{"action":"write","path":"/home/user/workspace/t.txt","content":"a\nb"}}
    });
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::ToolCall { call, .. } => {
            assert_eq!(call.kind, ToolKind::Edit);
            assert_eq!(call.status, ToolStatus::InProgress);
            assert_eq!(call.title, "Write t.txt");
            assert_eq!(call.locations[0].path, "/home/user/workspace/t.txt");
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn file_read_kind_and_browser_and_search_titles() {
    let read = json!({"type":"tool_call","data":{"tool_call_id":"a","tool_name":"file","arguments":{"action":"read","path":"/x/y.md"}}});
    assert!(matches!(
        event_to_agent(&read, &run(), &sess()),
        Some(AgentEvent::ToolCall { call, .. }) if call.kind == ToolKind::Read && call.title == "Read y.md"
    ));
    let search = json!({"type":"tool_call","data":{"tool_call_id":"b","tool_name":"web_search","arguments":{"query":"joke of the day"}}});
    assert!(matches!(
        event_to_agent(&search, &run(), &sess()),
        Some(AgentEvent::ToolCall { call, .. }) if call.kind == ToolKind::Search
    ));
}

#[test]
fn tool_result_prefers_complete_content_over_excerpt() {
    let ev = json!({
        "type":"tool_result",
        "data":{"tool_call_id":"t1","is_error":false,
                "result":{"success":true,"content":"wrote 6 bytes","excerpt":"a\nb\nc\n"}}
    });
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::ToolCallUpdate { id, patch, .. } => {
            assert_eq!(id.as_str(), "t1");
            assert_eq!(patch.status, Some(ToolStatus::Completed));
            assert_eq!(
                patch.append_content,
                vec![ContentBlock::text("wrote 6 bytes")]
            );
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn tool_result_preserves_every_structured_content_block() {
    let payload = "x".repeat(32_000);
    let ev = json!({
        "type":"tool_result",
        "data":{"tool_call_id":"t1","result":{"success":true,"content":[
            {"type":"text","text": payload},
            {"type":"image","mimeType":"image/png","data":"QUJD"},
            {"type":"resource","resource":{"uri":"attachment://full.bin","mimeType":"application/octet-stream","blob":"REVG"}}
        ],"excerpt":"short preview"}}
    });
    let AgentEvent::ToolCallUpdate { patch, .. } = event_to_agent(&ev, &run(), &sess()).unwrap()
    else {
        panic!("expected tool result");
    };
    assert_eq!(patch.append_content[0], ContentBlock::text(payload));
    assert!(matches!(
        &patch.append_content[1],
        ContentBlock::Image { data, .. } if data == "QUJD"
    ));
    assert!(matches!(
        &patch.append_content[2],
        ContentBlock::Resource { data: Some(data), .. } if data == "REVG"
    ));
}

#[test]
fn assistant_thinking_becomes_private_reasoning() {
    let ev = json!({
        "type": "assistant_thinking",
        "data": {"delta": "consider the prior tool evidence"}
    });
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::MessageChunk { role, delta, .. } => {
            assert_eq!(role, Role::Agent);
            assert_eq!(
                delta,
                ContentBlock::thinking("consider the prior tool evidence")
            );
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn execution_plan_maps_phases_with_status() {
    let ev = json!({
        "type":"execution_plan_committed",
        "data":{"phases":[
            {"id":1,"title":"Write file","status":"running"},
            {"id":2,"title":"Read file","status":"pending"}
        ]}
    });
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::ExecutionChecklistUpdated { checklist, .. } => {
            assert_eq!(checklist.steps.len(), 2);
            assert_eq!(checklist.steps[0].status, ChecklistStatus::InProgress);
            assert_eq!(checklist.steps[1].status, ChecklistStatus::Pending);
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn workspace_focus_maps_surface() {
    let ev = json!({"type":"workspace_focus","data":{"surface":"files","path":"/x/y.txt"}});
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::Surface { focus } => {
            assert_eq!(focus.surface, WorkspaceSurfaceKind::Files);
            assert_eq!(focus.path.as_deref(), Some("/x/y.txt"));
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn run_completed_finishes_done() {
    let ev = json!({"type":"run_completed","data":{"loop_outcome":"done"}});
    assert!(matches!(
        event_to_agent(&ev, &run(), &sess()),
        Some(AgentEvent::RunFinished { outcome, .. }) if outcome.status == RunStatus::Done
    ));
}

#[test]
fn run_completed_emits_terminal_artifacts_before_finish() {
    let ev = json!({
        "type": "run_completed",
        "data": {
            "loop_outcome": "done",
            "result_envelope": {
                "kind": "result",
                "payload": {
                    "artifacts": [
                        {
                            "id": "/api/artifacts/conv-1/report.pdf",
                            "title": "report.pdf",
                            "kind": "pdf",
                            "mime_type": "application/pdf",
                            "url": "/api/artifacts/conv-1/report.pdf"
                        },
                        {
                            "id": "published-site",
                            "title": "Website",
                            "kind": "website",
                            "url": "/sites/conv-1/index.html"
                        }
                    ]
                }
            }
        }
    });

    let events = events_to_agent(&ev, &run(), &sess());
    assert_eq!(events.len(), 3);
    match &events[0] {
        AgentEvent::Artifact { artifact, .. } => {
            assert_eq!(artifact.id, "/api/artifacts/conv-1/report.pdf");
            assert_eq!(artifact.kind, ArtifactKind::Pdf);
            assert_eq!(
                artifact.uri.as_deref(),
                Some("/api/artifacts/conv-1/report.pdf")
            );
        }
        other => panic!("got {other:?}"),
    }
    match &events[1] {
        AgentEvent::Artifact { artifact, .. } => {
            assert_eq!(artifact.id, "site");
            assert_eq!(artifact.kind, ArtifactKind::Website);
            assert_eq!(artifact.mime_type.as_deref(), Some("text/html"));
            assert_eq!(artifact.uri.as_deref(), Some("/sites/conv-1/index.html"));
        }
        other => panic!("got {other:?}"),
    }
    assert!(matches!(&events[2], AgentEvent::RunFinished { .. }));
}

#[test]
fn confirmation_requested_becomes_a_permission_gate() {
    let ev = json!({
        "type": "confirmation_requested",
        "data": {
            "action_id": "act-9",
            "description": "Send this email to the vendor?",
            "draft_preview": "Subject: Order update\n\nHi — confirming quantities…",
            "tool_call_id": "t7",
            "choices": [
                {"label": "Send it", "description": "", "value": "approve"},
                {"label": "Cancel", "description": "", "value": "reject"}
            ]
        }
    });
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::PermissionRequest { request } => {
            // The action id round-trips as the request id — `respond`
            // sends it back verbatim in the confirm command.
            assert_eq!(request.id.as_str(), "act-9");
            assert_eq!(request.session.as_str(), "conv-1");
            assert_eq!(request.title, "Send this email to the vendor?");
            assert_eq!(request.tool_call.as_ref().map(|t| t.as_str()), Some("t7"));
            assert!(request.detail.as_deref().unwrap().contains("Order update"));
            assert_eq!(request.risk.as_deref(), Some("confirm"));
            let opts: Vec<_> = request
                .options
                .iter()
                .map(|o| (o.id.as_str(), o.label.as_str()))
                .collect();
            assert_eq!(opts, vec![("approve", "Send it"), ("reject", "Cancel")]);
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn confirmation_without_choices_gets_default_labels() {
    let ev = json!({
        "type": "confirmation_requested",
        "data": {"action_id": "act-1", "description": "Proceed?"}
    });
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::PermissionRequest { request } => {
            assert!(request.tool_call.is_none());
            assert!(request.detail.is_none());
            let labels: Vec<_> = request.options.iter().map(|o| o.label.as_str()).collect();
            assert_eq!(labels, vec!["Confirm", "Cancel"]);
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn message_ask_surfaces_the_question_and_ends_the_turn() {
    let ev = json!({
        "type": "message_ask",
        "data": {"question": "Which vendor should I contact first?"}
    });
    let events = events_to_agent(&ev, &run(), &sess());
    assert_eq!(events.len(), 2);
    match &events[0] {
        AgentEvent::MessageChunk { role, delta, .. } => {
            assert_eq!(*role, Role::Agent);
            assert_eq!(
                *delta,
                ContentBlock::text("Which vendor should I contact first?")
            );
        }
        other => panic!("got {other:?}"),
    }
    match &events[1] {
        AgentEvent::RunFinished { outcome, .. } => {
            assert_eq!(outcome.status, RunStatus::Done);
            assert_eq!(outcome.stop_reason.as_deref(), Some("message_ask"));
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn internal_events_ignored() {
    for t in ["runtime_checkpoint", "tool_gate_applied", "llm_response"] {
        assert!(event_to_agent(&json!({"type": t, "data": {}}), &run(), &sess()).is_none());
    }
}

#[test]
fn tool_titles_never_leak_internal_tool_names() {
    // create_artifact → no "create_artifact"/underscore in the label.
    let art = json!({"type":"tool_call","data":{"tool_call_id":"x","tool_name":"create_artifact","arguments":{"name":"index.html"}}});
    match event_to_agent(&art, &run(), &sess()).unwrap() {
        AgentEvent::ToolCall { call, .. } => {
            assert_eq!(call.kind, ToolKind::Edit);
            assert!(!call.title.contains('_'), "leaked: {}", call.title);
            assert!(
                !call.title.to_lowercase().contains("artifact"),
                "leaked: {}",
                call.title
            );
            assert!(call.title.contains("index.html"));
        }
        other => panic!("got {other:?}"),
    }
    // publish → no "publish" in the label.
    let publish = json!({"type":"tool_call","data":{"tool_call_id":"y","tool_name":"publish_website","arguments":{}}});
    match event_to_agent(&publish, &run(), &sess()).unwrap() {
        AgentEvent::ToolCall { call, .. } => {
            assert_eq!(call.kind, ToolKind::Fetch);
            assert!(
                !call.title.to_lowercase().contains("publish"),
                "leaked: {}",
                call.title
            );
        }
        other => panic!("got {other:?}"),
    }
    // `web_fetch` is a page read, even if a malformed public label repeats
    // its internal identifier. An empty URL still gets a meaningful label.
    let fetch = json!({"type":"tool_call","data":{"tool_call_id":"z","tool_name":"web_fetch","public_activity_label":"web_fetch","arguments":{"url":"https://example.com/docs"}}});
    match event_to_agent(&fetch, &run(), &sess()).unwrap() {
        AgentEvent::ToolCall { call, .. } => {
            assert_eq!(call.kind, ToolKind::Fetch);
            assert_eq!(call.title, "Read https://example.com/docs");
        }
        other => panic!("got {other:?}"),
    }
    let untargeted_fetch = json!({"type":"tool_call","data":{"tool_call_id":"z2","tool_name":"web_fetch","activity_label":"Web fetch","arguments":{"url":"  "}}});
    match event_to_agent(&untargeted_fetch, &run(), &sess()).unwrap() {
        AgentEvent::ToolCall { call, .. } => {
            assert_eq!(call.kind, ToolKind::Fetch);
            assert_eq!(call.title, "Reading a web page");
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn image_tools_get_typed_kinds_and_user_facing_titles() {
    assert_eq!(tool_kind("view_image", None), ToolKind::ViewImage);
    assert_eq!(
        tool_title(
            ToolKind::ViewImage,
            None,
            &json!({"path": "design/mockup.png"}),
        ),
        "Viewed mockup.png"
    );
    assert_eq!(tool_kind("image_generation", None), ToolKind::GenerateImage);
    assert_eq!(
        tool_title(
            ToolKind::GenerateImage,
            None,
            &json!({"path": "images/hero.png"}),
        ),
        "Generated hero.png"
    );
}

#[test]
fn backend_public_label_is_used_when_present() {
    let ev = json!({
        "type":"tool_call",
        "data":{"tool_call_id":"z","tool_name":"create_artifact",
                "public_activity_label":"Building the homepage","arguments":{}}
    });
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::ToolCall { call, .. } => assert_eq!(call.title, "Building the homepage"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn subagent_map_event_becomes_fanout_tile() {
    let ev = json!({
        "type":"subagent_event",
        "data":{
            "scope":{"spawning_tool":"subagent_map","row_index":2,"parent_tool_call_id":"map-1"},
            "phase":"run_completed",
            "summary":"Summarized auth.rs"
        }
    });
    match event_to_agent(&ev, &run(), &sess()).unwrap() {
        AgentEvent::FanOut { parent, agent, .. } => {
            assert_eq!(parent.as_str(), "map-1");
            assert_eq!(agent.id, "2");
            assert_eq!(agent.status, FanOutStatus::Done);
            assert_eq!(agent.label, "Summarized auth.rs");
            assert_eq!(agent.activity.as_deref(), Some("Complete"));
            assert_eq!(agent.result.as_deref(), Some("Summarized auth.rs"));
            assert!(agent.updated_at_ms.is_some());
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn single_subagent_event_still_appends_summary() {
    // Non-map subagent (e.g. website builder) keeps the tool-call summary path.
    let ev = json!({
        "type":"subagent_event",
        "data":{
            "scope":{"spawning_tool":"create_website","child_storage_id":"conv:call-1"},
            "summary":"Wrote index.html"
        }
    });
    assert!(matches!(
        event_to_agent(&ev, &run(), &sess()),
        Some(AgentEvent::ToolCallUpdate { id, .. }) if id.as_str() == "call-1"
    ));
}
