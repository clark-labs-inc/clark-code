//! Conversion from the provider-agnostic persisted transcript into the local
//! loop's canonical typed history.

use agent_core::{ContentBlock, ResumeItem, ResumeTranscript, Role, ToolKind, ToolStatus};
use agent_loop as ca;

mod content;

use content::{
    assistant_blocks, assistant_content, tool_result_block, tool_result_content, user_block,
    user_content,
};

/// Rebuild model context from durable history. Image bytes remain in the
/// provider-agnostic resume transcript for UI/history fidelity, but a text-only
/// coding route must never inherit historical multimodal blocks. Live tool
/// execution already applies the same policy before appending results.
pub(crate) fn to_agent_messages(
    resume: Option<&ResumeTranscript>,
    native_image_support: bool,
) -> Vec<ca::AgentMessage> {
    let mut messages = Vec::new();
    let Some(resume) = resume else {
        return messages;
    };
    if resume.truncated {
        messages.push(ca::AgentMessage::Custom {
            kind: "resume_boundary".into(),
            payload: serde_json::json!({
                "notice": "Earlier conversation items were omitted by the bounded resume window."
            }),
            timestamp: None,
        });
    }
    for item in &resume.items {
        match item {
            ResumeItem::Message { role, blocks } => match role {
                Role::User => {
                    if let Some(content) = user_content(blocks, native_image_support) {
                        messages.push(ca::AgentMessage::User {
                            content,
                            timestamp: None,
                        });
                    }
                }
                Role::Agent => {
                    let content = assistant_content(blocks);
                    if !content.blocks.is_empty() {
                        messages.push(ca::AgentMessage::Assistant {
                            content,
                            stop_reason: ca::StopReason::EndTurn,
                            error_message: None,
                            timestamp: None,
                            usage: None,
                        });
                    }
                }
                Role::System => {}
            },
            ResumeItem::ToolCall {
                id,
                tool_name,
                title,
                kind,
                status,
                locations,
                arguments,
                content,
            } => {
                let name = tool_name
                    .clone()
                    .unwrap_or_else(|| "historical_tool".into());
                messages.push(ca::AgentMessage::Assistant {
                    content: ca::AssistantContent::with_tool_calls(
                        None,
                        vec![ca::ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            arguments: arguments.clone().unwrap_or(serde_json::Value::Null),
                        }],
                    ),
                    stop_reason: ca::StopReason::ToolUse,
                    error_message: None,
                    timestamp: None,
                    usage: None,
                });
                let result =
                    tool_result_content(content, native_image_support).unwrap_or_else(|| {
                        ca::ToolResultContent::text(format!("{title} ({status:?})"))
                    });
                messages.push(ca::AgentMessage::ToolResult {
                    tool_call_id: id.clone(),
                    tool_name: name,
                    content: result,
                    is_error: matches!(status, ToolStatus::Failed | ToolStatus::Cancelled),
                    narration: Some(title.clone()),
                    details: Some(serde_json::json!({
                        "kind": kind,
                        "status": status,
                        "locations": locations,
                        "replayed": true,
                    })),
                    timestamp: None,
                });
            }
            ResumeItem::Goal { .. } => {}
            ResumeItem::ProposedPlan { plan } => {
                messages.push(ca::AgentMessage::Custom {
                    kind: "proposed_plan".into(),
                    payload: serde_json::json!({
                        "id": plan.id,
                        "revision": plan.revision,
                        "status": plan.status,
                        "global_reminders": plan.global_reminders,
                        "execution_contract": plan.execution_contract,
                        "context_revisions": plan.context_revisions,
                        "markdown": plan.markdown,
                    }),
                    timestamp: None,
                });
            }
        }
    }
    messages
}

/// Persist the local loop's canonical model transcript in the provider-
/// agnostic replay shape. Compaction checkpoints use this instead of the UI
/// timeline, which deliberately retains the full visible conversation.
pub(crate) fn from_agent_messages(messages: &[ca::AgentMessage]) -> ResumeTranscript {
    let mut items = Vec::new();
    for message in messages {
        match message {
            ca::AgentMessage::System { .. } | ca::AgentMessage::Custom { .. } => {}
            ca::AgentMessage::User { content, .. } => {
                let blocks = match content {
                    ca::UserContent::Text(text) => vec![ContentBlock::text(text.clone())],
                    ca::UserContent::Blocks(blocks) => blocks.iter().map(user_block).collect(),
                };
                if !blocks.is_empty() {
                    items.push(ResumeItem::Message {
                        role: Role::User,
                        blocks,
                    });
                }
            }
            ca::AgentMessage::Assistant { content, .. } => {
                let blocks = assistant_blocks(content);
                if !blocks.is_empty() {
                    items.push(ResumeItem::Message {
                        role: Role::Agent,
                        blocks,
                    });
                }
                for call in content.tool_calls() {
                    items.push(ResumeItem::ToolCall {
                        id: call.id.clone(),
                        tool_name: Some(call.name.clone()),
                        title: call.name.clone(),
                        kind: ToolKind::Other,
                        status: ToolStatus::Pending,
                        locations: Vec::new(),
                        arguments: Some(call.arguments.clone()),
                        content: Vec::new(),
                    });
                }
            }
            ca::AgentMessage::ToolResult {
                tool_call_id,
                tool_name,
                content,
                is_error,
                narration,
                ..
            } => {
                if let Some(ResumeItem::ToolCall {
                    title,
                    status,
                    content: stored_content,
                    ..
                }) = items.iter_mut().rev().find(
                    |item| matches!(item, ResumeItem::ToolCall { id, .. } if id == tool_call_id),
                ) {
                    *title = narration.clone().unwrap_or_else(|| tool_name.clone());
                    *status = if *is_error {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Completed
                    };
                    *stored_content = content.blocks.iter().map(tool_result_block).collect();
                }
            }
        }
    }
    ResumeTranscript {
        items,
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::domain::{ProposedPlan, ProposedPlanStatus};
    use agent_core::{ResumeItem, ResumeTranscript, ToolKind};

    #[test]
    fn replay_preserves_tool_pair_and_reasoning_for_future_compaction() {
        let transcript = ResumeTranscript {
            truncated: false,
            items: vec![
                ResumeItem::Message {
                    role: Role::User,
                    blocks: vec![ContentBlock::text("do it")],
                },
                ResumeItem::Message {
                    role: Role::Agent,
                    blocks: vec![
                        ContentBlock::thinking("private"),
                        ContentBlock::text("working"),
                    ],
                },
                ResumeItem::ToolCall {
                    id: "call-1".into(),
                    tool_name: Some("bash".into()),
                    title: "Run tests".into(),
                    kind: ToolKind::Execute,
                    status: ToolStatus::Completed,
                    locations: Vec::new(),
                    arguments: Some(serde_json::json!({"command": "true"})),
                    content: vec![ContentBlock::text("exit_code: 0")],
                },
            ],
        };
        let messages = to_agent_messages(Some(&transcript), false);
        assert_eq!(messages.len(), 4);
        assert!(format!("{messages:?}").contains("private"));
        assert!(matches!(messages[2], ca::AgentMessage::Assistant { .. }));
        assert!(matches!(messages[3], ca::AgentMessage::ToolResult { .. }));
    }

    #[test]
    fn readable_reasoning_details_survive_checkpoint_resume_round_trip() {
        let original = vec![ca::AgentMessage::Assistant {
            content: ca::AssistantContent {
                blocks: vec![
                    ca::AssistantBlock::Reasoning(ca::TextContent {
                        text: "complete native reasoning".into(),
                    }),
                    ca::AssistantBlock::ReasoningDetails(ca::ReasoningDetailsContent::from_items(
                        &[
                            ca::ReasoningItem::Summary {
                                id: Some("summary-1".into()),
                                format: ca::ReasoningFormat::AnthropicClaudeV1,
                                index: Some(0),
                                summary: "durable structured finding".into(),
                            },
                            ca::ReasoningItem::Encrypted {
                                id: Some("encrypted-1".into()),
                                format: ca::ReasoningFormat::GoogleGeminiV1,
                                index: Some(1),
                                data: "opaque-provider-payload".into(),
                            },
                        ],
                    )),
                ],
            },
            stop_reason: ca::StopReason::EndTurn,
            error_message: None,
            timestamp: None,
            usage: None,
        }];

        let transcript = from_agent_messages(&original);
        let serialized = serde_json::to_string(&transcript).unwrap();
        assert!(serialized.contains("complete native reasoning"));
        assert!(serialized.contains("durable structured finding"));
        assert!(!serialized.contains("opaque-provider-payload"));

        let replayed = to_agent_messages(Some(&transcript), false);
        let replayed = format!("{replayed:?}");
        assert!(replayed.contains("complete native reasoning"));
        assert!(replayed.contains("durable structured finding"));
    }

    #[test]
    fn compacted_agent_messages_round_trip_as_typed_resume_context() {
        let original = vec![
            ca::AgentMessage::User {
                content: ca::UserContent::Text("summary".into()),
                timestamp: None,
            },
            ca::AgentMessage::Assistant {
                content: ca::AssistantContent::with_tool_calls(
                    Some("checking".into()),
                    vec![ca::ToolCall {
                        id: "call-1".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({"command": "true"}),
                    }],
                ),
                stop_reason: ca::StopReason::ToolUse,
                error_message: None,
                timestamp: None,
                usage: None,
            },
            ca::AgentMessage::ToolResult {
                tool_call_id: "call-1".into(),
                tool_name: "bash".into(),
                content: ca::ToolResultContent::text("exit_code: 0"),
                is_error: false,
                narration: Some("Ran tests".into()),
                details: None,
                timestamp: None,
            },
        ];

        let transcript = from_agent_messages(&original);
        assert_eq!(transcript.items.len(), 3);
        assert!(matches!(
            &transcript.items[2],
            ResumeItem::ToolCall { status: ToolStatus::Completed, title, .. }
                if title == "Ran tests"
        ));
        let replayed = to_agent_messages(Some(&transcript), false);
        assert_eq!(replayed.len(), 4);
        assert!(matches!(replayed[2], ca::AgentMessage::Assistant { .. }));
        assert!(matches!(replayed[3], ca::AgentMessage::ToolResult { .. }));
    }

    #[test]
    fn tool_result_images_survive_resume_replay_byte_for_byte() {
        let transcript = ResumeTranscript {
            truncated: false,
            items: vec![ResumeItem::ToolCall {
                id: "image-call".into(),
                tool_name: Some("view_image".into()),
                title: "Viewed image".into(),
                kind: ToolKind::ViewImage,
                status: ToolStatus::Completed,
                locations: Vec::new(),
                arguments: Some(serde_json::json!({"path":"diagram.png"})),
                content: vec![
                    ContentBlock::text("complete image"),
                    ContentBlock::Image {
                        mime_type: "image/png".into(),
                        data: "QUJDREVGRw==".into(),
                        uri: None,
                    },
                ],
            }],
        };

        let messages = to_agent_messages(Some(&transcript), true);
        let ca::AgentMessage::ToolResult { content, .. } = &messages[1] else {
            panic!("expected tool result");
        };
        assert!(matches!(
            &content.blocks[1],
            ca::ToolResultBlock::Image(image)
                if image.source == "data:image/png;base64,QUJDREVGRw=="
                    && image.media_type.as_deref() == Some("image/png")
        ));

        let round_trip = from_agent_messages(&messages);
        let serialized = serde_json::to_string(&round_trip).unwrap();
        assert!(serialized.contains("QUJDREVGRw=="));
    }

    #[test]
    fn text_only_resume_keeps_tool_receipt_but_drops_historical_image_input() {
        let transcript = ResumeTranscript {
            truncated: false,
            items: vec![ResumeItem::ToolCall {
                id: "image-call".into(),
                tool_name: Some("view_image".into()),
                title: "Viewed image".into(),
                kind: ToolKind::ViewImage,
                status: ToolStatus::Completed,
                locations: Vec::new(),
                arguments: Some(serde_json::json!({"path":"screenshot.png"})),
                content: vec![
                    ContentBlock::text("vision-derived description of the screenshot"),
                    ContentBlock::Image {
                        mime_type: "image/png".into(),
                        data: "QUJDREVGRw==".into(),
                        uri: None,
                    },
                ],
            }],
        };

        let messages = to_agent_messages(Some(&transcript), false);
        let ca::AgentMessage::ToolResult { content, .. } = &messages[1] else {
            panic!("expected tool result");
        };
        assert_eq!(content.blocks.len(), 1);
        assert!(matches!(
            &content.blocks[0],
            ca::ToolResultBlock::Text(text)
                if text.text == "vision-derived description of the screenshot"
        ));
    }

    #[test]
    fn replay_marks_cancelled_tools_as_incomplete_errors() {
        let transcript = ResumeTranscript {
            truncated: false,
            items: vec![ResumeItem::ToolCall {
                id: "cancelled-call".into(),
                tool_name: Some("web_fetch".into()),
                title: "Fetch page".into(),
                kind: ToolKind::Fetch,
                status: ToolStatus::Cancelled,
                locations: Vec::new(),
                arguments: Some(serde_json::json!({"url": "https://example.com"})),
                content: Vec::new(),
            }],
        };

        let messages = to_agent_messages(Some(&transcript), false);
        let ca::AgentMessage::ToolResult {
            is_error, content, ..
        } = &messages[1]
        else {
            panic!("expected a replayed tool result");
        };
        assert!(*is_error);
        assert!(format!("{content:?}").contains("Cancelled"));
    }

    #[test]
    fn truncated_resume_starts_with_a_typed_boundary_marker() {
        let transcript = ResumeTranscript {
            truncated: true,
            items: vec![ResumeItem::Message {
                role: Role::User,
                blocks: vec![ContentBlock::text("recent request")],
            }],
        };
        let messages = to_agent_messages(Some(&transcript), false);
        assert!(matches!(
            &messages[0],
            ca::AgentMessage::Custom { kind, .. } if kind == "resume_boundary"
        ));
        assert!(matches!(messages[1], ca::AgentMessage::User { .. }));
    }

    #[test]
    fn skill_reference_identity_and_revision_survive_resume_conversion() {
        let transcript = ResumeTranscript {
            truncated: false,
            items: vec![ResumeItem::Message {
                role: Role::User,
                blocks: vec![
                    ContentBlock::text("plan this"),
                    ContentBlock::skill_reference("skill_123", "rev_456", "brainstorming"),
                ],
            }],
        };
        let messages = to_agent_messages(Some(&transcript), false);
        let rendered = format!("{messages:?}");
        assert!(rendered.contains("skill_123"));
        assert!(rendered.contains("rev_456"));
        assert!(rendered.contains("brainstorming"));
    }

    #[test]
    fn resumed_proposal_context_preserves_complete_typed_state() {
        let transcript = ResumeTranscript {
            truncated: false,
            items: vec![ResumeItem::ProposedPlan {
                plan: ProposedPlan {
                    id: "plan-large".into(),
                    revision: 3,
                    markdown: "x".repeat(8_000),
                    status: ProposedPlanStatus::AwaitingDecision,
                    global_reminders: Vec::new(),
                    execution_contract: Vec::new(),
                    context_revisions: Vec::new(),
                },
            }],
        };
        let messages = to_agent_messages(Some(&transcript), false);
        let ca::AgentMessage::Custom { payload, .. } = &messages[0] else {
            panic!("expected proposed-plan context");
        };
        let markdown = payload["markdown"].as_str().unwrap();
        assert_eq!(markdown.chars().count(), 8_000);
        assert!(!markdown.contains("proposal middle omitted"));
        assert_eq!(payload["id"], "plan-large");
        assert_eq!(payload["revision"], 3);
    }
}
