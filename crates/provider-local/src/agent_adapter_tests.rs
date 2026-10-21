use super::*;

#[test]
fn usage_totals_return_each_calls_cumulative_cost_and_tokens() {
    let totals = UsageTotals::default();
    assert_eq!(
        totals.add(crate::llm::TokenUsage {
            prompt_tokens: 1_000,
            completion_tokens: 100,
            cost_usd: Some(0.01),
        }),
        desktop::RunUsage {
            input_tokens: 1_000,
            output_tokens: 100,
            context_tokens: 1_000,
            cost_usd: Some(0.01),
            context_limit: None,
        }
    );
    assert_eq!(
        totals.add(crate::llm::TokenUsage {
            prompt_tokens: 1_500,
            completion_tokens: 200,
            cost_usd: Some(0.02),
        }),
        desktop::RunUsage {
            input_tokens: 2_500,
            output_tokens: 300,
            context_tokens: 1_500,
            cost_usd: Some(0.03),
            context_limit: None,
        }
    );
}

#[test]
fn malformed_tool_args_use_core_parse_error_marker() {
    let value = parse_tool_args("{bad");
    assert!(ca::detect_arg_parse_error(&value).is_some());
}

#[test]
fn proposed_plan_stream_filter_hides_only_a_complete_framed_block() {
    let mut filter = ProposedPlanStreamFilter::default();
    assert_eq!(filter.feed("before <proposed_"), "before ");
    assert_eq!(filter.feed("plan>secret"), "");
    assert_eq!(filter.feed(" plan</proposed_plan> after"), " after");
    assert_eq!(filter.finish(), "");

    let mut malformed = ProposedPlanStreamFilter::default();
    assert_eq!(malformed.feed("<proposed_plan>secret"), "");
    assert_eq!(malformed.finish(), "<proposed_plan>secret");
}

#[test]
fn tool_title_describes_file_and_web_activity() {
    assert_eq!(
        tool_title("read_file", &json!({"path":"src/main.rs"})),
        "Read src/main.rs"
    );
    assert_eq!(
        tool_title("web_fetch", &json!({"url":"https://example.com/docs"})),
        "Read https://example.com/docs"
    );
    assert_eq!(tool_title("web_fetch", &json!({})), "Reading a web page");
    assert_eq!(
        tool_title("read_skill", &json!({"skill":"github:gh-fix-ci"})),
        "Read skill github:gh-fix-ci"
    );
}

#[test]
fn tool_title_never_falls_back_to_an_internal_identifier() {
    assert_eq!(tool_title("future_internal_tool", &json!({})), "Working");
    assert_eq!(
        tool_title("mcp_github_create_issue", &json!({})),
        "Using a connected service"
    );
}

#[test]
fn tool_title_special_cases_plan_tools() {
    assert_eq!(tool_title("propose_plan", &json!({})), "Proposed a plan");
    assert_eq!(tool_title("update_plan", &json!({})), "Updated the plan");
}

#[test]
fn tool_title_special_cases_image_tools() {
    assert_eq!(
        tool_title("view_image", &json!({"path": "design/mockup.png"})),
        "View image: design/mockup.png"
    );
    assert_eq!(tool_title("generate_image", &json!({})), "Generate image");
    assert_eq!(
        tool_title("generate_image", &json!({"output_path": "images/hero.png"})),
        "Generate image: images/hero.png"
    );
}

#[test]
fn overlong_tool_titles_mark_their_cut_instead_of_ending_mid_token() {
    let command = format!("grep -rn {} src", "x".repeat(120));
    let title = tool_title("bash", &json!({ "command": command }));
    assert_eq!(title.chars().count(), 80);
    assert!(
        title.ends_with('…'),
        "cut titles must be visibly elided: {title}"
    );
    // A title that fits keeps its exact text, unmarked.
    assert_eq!(
        tool_title(
            "bash",
            &json!({ "command": "cargo nextest run -p provider-local" })
        ),
        "cargo nextest run -p provider-local"
    );
}

#[test]
fn produced_image_artifact_preserves_its_typed_preview_uri() {
    let artifact = super::tools::produced_artifact_to_desktop(
        &crate::tools::ProducedArtifact {
            id: "image:images/hero.png".into(),
            title: "hero.png".into(),
            kind: desktop::ArtifactKind::Image,
            mime_type: Some("image/png".into()),
            uri: Some("data:image/png;base64,QUJD".into()),
        },
        &ToolCallId::new("call-1".to_string()),
    );

    assert_eq!(artifact.kind, desktop::ArtifactKind::Image);
    assert_eq!(artifact.mime_type.as_deref(), Some("image/png"));
    assert_eq!(artifact.uri.as_deref(), Some("data:image/png;base64,QUJD"));
    assert_eq!(
        artifact.tool_call.as_ref().map(|id| id.as_str()),
        Some("call-1")
    );
}

#[test]
fn markdown_artifact_only_for_md_inside_the_workspace() {
    let docs = tempfile::tempdir().unwrap();
    let docs_canon = docs.path().canonicalize().unwrap();

    // A .md written into the workspace → an inline markdown artifact.
    let md = docs_canon.join("report.md");
    std::fs::write(&md, "# Hi").unwrap();
    let art = markdown_artifact(md.to_str().unwrap(), "call-1", &docs_canon).expect("md doc");
    assert_eq!(art.kind, desktop::ArtifactKind::File);
    assert_eq!(art.mime_type.as_deref(), Some("text/markdown"));
    assert_eq!(art.uri.as_deref(), Some(md.to_str().unwrap()));
    assert_eq!(art.title, "report.md");
    assert_eq!(art.id, "doc:report.md");

    // Filesystem tools report project-relative locations. Resolve those
    // against the document root so a successful `write_file` immediately
    // becomes the document artifact shown by the artifact workspace.
    let relative =
        markdown_artifact("report.md", "call-2", &docs_canon).expect("relative markdown doc");
    assert_eq!(relative.uri.as_deref(), Some(md.to_str().unwrap()));
    assert_eq!(relative.id, "doc:report.md");

    // A non-markdown file in the workspace → no artifact.
    let txt = docs_canon.join("notes.txt");
    std::fs::write(&txt, "x").unwrap();
    assert!(markdown_artifact(txt.to_str().unwrap(), "c", &docs_canon).is_none());

    // A markdown file outside the workspace → no artifact.
    let outside = tempfile::tempdir().unwrap();
    let out_md = outside.path().canonicalize().unwrap().join("x.md");
    std::fs::write(&out_md, "x").unwrap();
    assert!(markdown_artifact(out_md.to_str().unwrap(), "c", &docs_canon).is_none());
}

#[test]
fn mobile_screenshot_artifact_only_for_images_inside_the_workspace() {
    let docs = tempfile::tempdir().unwrap();
    let docs_canon = docs.path().canonicalize().unwrap();

    let png = docs_canon.join("sim.png");
    std::fs::write(&png, [0u8; 4]).unwrap();
    let art = mobile_screenshot_artifact(png.to_str().unwrap(), "call-1", &docs_canon)
        .expect("png screenshot");
    assert_eq!(art.kind, desktop::ArtifactKind::Image);
    assert_eq!(art.mime_type.as_deref(), Some("image/png"));
    assert_eq!(art.uri.as_deref(), Some(png.to_str().unwrap()));
    assert!(mobile_screenshot_artifact("sim.png", "call-2", &docs_canon).is_some());

    let jpg = docs_canon.join("sim.jpg");
    std::fs::write(&jpg, [0u8; 4]).unwrap();
    let art = mobile_screenshot_artifact(jpg.to_str().unwrap(), "call-1", &docs_canon)
        .expect("jpg screenshot");
    assert_eq!(art.mime_type.as_deref(), Some("image/jpeg"));

    // A non-image file in the workspace → no artifact.
    let txt = docs_canon.join("notes.txt");
    std::fs::write(&txt, "x").unwrap();
    assert!(mobile_screenshot_artifact(txt.to_str().unwrap(), "c", &docs_canon).is_none());

    // A PNG outside the workspace → no artifact.
    let outside = tempfile::tempdir().unwrap();
    let out_png = outside.path().canonicalize().unwrap().join("x.png");
    std::fs::write(&out_png, [0u8; 4]).unwrap();
    assert!(mobile_screenshot_artifact(out_png.to_str().unwrap(), "c", &docs_canon).is_none());
}

#[test]
fn user_chat_message_stays_plain_text_with_no_images() {
    let content = ca::UserContent::Blocks(vec![ca::UserBlock::Text(ca::types::TextContent {
        text: "hello".into(),
    })]);
    let msg = user_chat_message(&content);
    assert_eq!(msg.role, "user");
    match msg.content {
        Some(ChatContent::Text(t)) => assert_eq!(t, "hello"),
        other => panic!("expected plain text content, got {other:?}"),
    }
}

#[test]
fn user_chat_message_forwards_images_as_content_parts() {
    let content = ca::UserContent::Blocks(vec![
        ca::UserBlock::Text(ca::types::TextContent {
            text: "check this out".into(),
        }),
        ca::UserBlock::Image(ca::ImageContent {
            source: "data:image/png;base64,QUJD".into(),
            media_type: Some("image/png".into()),
            alt: None,
        }),
    ]);
    let msg = user_chat_message(&content);
    assert_eq!(msg.role, "user");
    match msg.content {
        Some(ChatContent::Parts(parts)) => {
            assert_eq!(parts.len(), 2);
            assert!(matches!(&parts[0], ContentPart::Text { text } if text == "check this out"));
            assert!(matches!(
                &parts[1],
                ContentPart::ImageUrl { image_url } if image_url.url == "data:image/png;base64,QUJD"
            ));
        }
        other => panic!("expected content-parts, got {other:?}"),
    }
}

#[test]
fn to_wire_messages_injects_synthetic_user_turn_for_tool_result_images() {
    let messages = vec![ca::AgentMessage::ToolResult {
        tool_call_id: "call-1".into(),
        tool_name: "ios_screenshot".into(),
        content: ca::ToolResultContent {
            blocks: vec![
                ca::ToolResultBlock::Text(ca::types::TextContent {
                    text: "Screenshot captured.".into(),
                }),
                ca::ToolResultBlock::Image(ca::ImageContent {
                    source: "data:image/png;base64,QUJD".into(),
                    media_type: Some("image/png".into()),
                    alt: None,
                }),
            ],
        },
        is_error: false,
        narration: None,
        details: None,
        timestamp: None,
    }];
    let wire = to_wire_messages("", &messages);

    // The tool-role message itself stays plain text — the OpenAI-compatible
    // wire format doesn't allow a content-parts array on role: "tool".
    assert_eq!(wire[0].role, "tool");
    match &wire[0].content {
        Some(ChatContent::Text(t)) => assert_eq!(t, "Screenshot captured."),
        other => panic!("expected plain text tool content, got {other:?}"),
    }

    // The image rides in as a synthetic follow-up user turn.
    assert_eq!(wire[1].role, "user");
    match &wire[1].content {
        Some(ChatContent::Parts(parts)) => {
            assert!(parts.iter().any(|p| matches!(
                p,
                ContentPart::ImageUrl { image_url } if image_url.url == "data:image/png;base64,QUJD"
            )));
        }
        other => panic!("expected content-parts with the image, got {other:?}"),
    }
    assert_eq!(wire.len(), 2);
}

#[test]
fn to_wire_messages_skips_synthetic_turn_when_no_images() {
    let messages = vec![ca::AgentMessage::ToolResult {
        tool_call_id: "call-1".into(),
        tool_name: "grep".into(),
        content: ca::ToolResultContent::text("no matches"),
        is_error: false,
        narration: None,
        details: None,
        timestamp: None,
    }];
    let wire = to_wire_messages("", &messages);
    assert_eq!(wire.len(), 1);
    assert_eq!(wire[0].role, "tool");
}

#[test]
fn tool_result_blocks_to_content_maps_image_data_url_to_image_block() {
    let blocks = vec![ca::ToolResultBlock::Image(ca::ImageContent {
        source: "data:image/png;base64,QUJD".into(),
        media_type: Some("image/png".into()),
        alt: Some("a screenshot".into()),
    })];
    let content = tool_result_blocks_to_content(&blocks);
    assert_eq!(content.len(), 1);
    match &content[0] {
        desktop::ContentBlock::Image {
            mime_type,
            data,
            uri,
        } => {
            assert_eq!(mime_type, "image/png");
            assert_eq!(data, "QUJD");
            assert!(uri.is_none());
        }
        other => panic!("expected an Image content block, got {other:?}"),
    }
}

#[test]
fn tool_result_metadata_keeps_ui_images_when_model_images_are_suppressed() {
    let mut result = ca::ToolResult::text("The screenshot was captured.");
    store_tool_images(
        &mut result.details,
        &[crate::tools::ImageAttachment {
            mime_type: "image/png".into(),
            data_base64: "QUJD".into(),
            alt: Some("browser screenshot".into()),
        }],
    );

    let content = tool_result_to_content(&result);
    assert_eq!(content.len(), 2);
    assert!(matches!(
        &content[1],
        desktop::ContentBlock::Image { mime_type, data, uri }
            if mime_type == "image/png" && data == "QUJD" && uri.is_none()
    ));
}

#[test]
fn effect_verification_instructions_stay_out_of_visible_tool_output() {
    let mut result = ca::ToolResult::text(
        "command output\n\n[verification required]\nThis call may have changed durable state.",
    );
    result.details[crate::effects::EFFECT_DETAILS_KEY] = json!({ "id": "effect-1" });

    assert_eq!(
        tool_result_to_content(&result),
        vec![desktop::ContentBlock::text("command output")],
    );
    assert!(matches!(
        &result.content[0],
        ca::ToolResultBlock::Text(text) if text.text.contains("[verification required]")
    ));
}
