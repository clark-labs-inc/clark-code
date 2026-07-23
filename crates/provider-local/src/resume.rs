//! Conversion from the provider-agnostic persisted transcript into the local
//! loop's canonical typed history.

use agent_core::{ContentBlock, ResumeItem, ResumeTranscript, Role, ToolKind, ToolStatus};
use clark_agent as ca;

pub(crate) fn to_agent_messages(resume: Option<&ResumeTranscript>) -> Vec<ca::AgentMessage> {
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
                    if let Some(content) = user_content(blocks) {
                        messages.push(ca::AgentMessage::User {
                            content,
                            timestamp: None,
                        });
                    }
                }
                Role::Agent => {
                    let text = visible_text(blocks);
                    if !text.trim().is_empty() {
                        messages.push(ca::AgentMessage::Assistant {
                            content: ca::AssistantContent::text(text),
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
                let result = visible_text(content);
                let result = if result.trim().is_empty() {
                    format!("{title} ({status:?})")
                } else {
                    result
                };
                messages.push(ca::AgentMessage::ToolResult {
                    tool_call_id: id.clone(),
                    tool_name: name,
                    content: ca::ToolResultContent::text(result),
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
                let markdown = crate::planning::bounded_plan_markdown(&plan.markdown);
                messages.push(ca::AgentMessage::Custom {
                    kind: "proposed_plan".into(),
                    payload: serde_json::json!({
                        "id": plan.id,
                        "revision": plan.revision,
                        "markdown": markdown,
                        "status": plan.status,
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
                let text = content.plain_text();
                if !text.trim().is_empty() {
                    items.push(ResumeItem::Message {
                        role: Role::Agent,
                        blocks: vec![ContentBlock::text(text)],
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

fn user_content(blocks: &[ContentBlock]) -> Option<ca::UserContent> {
    let mut rich = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text }
            | ContentBlock::Resource {
                text: Some(text), ..
            } => {
                rich.push(ca::UserBlock::Text(ca::TextContent { text: text.clone() }));
            }
            ContentBlock::Image {
                mime_type,
                data,
                uri,
            } => rich.push(ca::UserBlock::Image(ca::ImageContent {
                source: uri
                    .clone()
                    .unwrap_or_else(|| format!("data:{mime_type};base64,{data}")),
                media_type: Some(mime_type.clone()),
                alt: None,
            })),
            ContentBlock::ResourceLink { uri, name } => {
                rich.push(ca::UserBlock::Text(ca::TextContent {
                    text: name.as_deref().unwrap_or(uri).to_string(),
                }))
            }
            ContentBlock::SkillReference { id, revision, name } => {
                rich.push(ca::UserBlock::Text(ca::TextContent {
                    text: format!("[Selected Clark skill: {name} ({id}@{revision})]"),
                }))
            }
            ContentBlock::Audio { .. }
            | ContentBlock::Thinking { .. }
            | ContentBlock::Resource { text: None, .. } => {}
        }
    }
    match rich.as_slice() {
        [] => None,
        [ca::UserBlock::Text(text)] => Some(ca::UserContent::Text(text.text.clone())),
        _ => Some(ca::UserContent::Blocks(rich)),
    }
}

fn user_block(block: &ca::UserBlock) -> ContentBlock {
    match block {
        ca::UserBlock::Text(text) => ContentBlock::text(text.text.clone()),
        ca::UserBlock::Image(image) => image_block(image),
    }
}

fn tool_result_block(block: &ca::ToolResultBlock) -> ContentBlock {
    match block {
        ca::ToolResultBlock::Text(text) => ContentBlock::text(text.text.clone()),
        ca::ToolResultBlock::Image(image) => image_block(image),
    }
}

fn image_block(image: &ca::ImageContent) -> ContentBlock {
    let mime_type = image
        .media_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    let data_prefix = format!("data:{mime_type};base64,");
    if let Some(data) = image.source.strip_prefix(&data_prefix) {
        ContentBlock::Image {
            mime_type,
            data: data.to_string(),
            uri: None,
        }
    } else {
        ContentBlock::Image {
            mime_type,
            data: String::new(),
            uri: Some(image.source.clone()),
        }
    }
}

fn visible_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::Resource {
                text: Some(text), ..
            } => Some(text.clone()),
            ContentBlock::ResourceLink { uri, name } => {
                Some(name.as_deref().unwrap_or(uri).to_string())
            }
            ContentBlock::Image { uri, .. } => Some(
                uri.as_deref()
                    .map(|uri| format!("[image: {uri}]"))
                    .unwrap_or_else(|| "[image attachment]".into()),
            ),
            ContentBlock::Audio { .. } => Some("[audio attachment]".into()),
            ContentBlock::SkillReference { name, .. } => Some(format!("[Selected skill: {name}]")),
            ContentBlock::Thinking { .. } | ContentBlock::Resource { text: None, .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::domain::{ProposedPlan, ProposedPlanStatus};
    use agent_core::{ResumeItem, ResumeTranscript, ToolKind};

    #[test]
    fn replay_preserves_tool_pair_and_omits_thinking() {
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
        let messages = to_agent_messages(Some(&transcript));
        assert_eq!(messages.len(), 4);
        assert!(!format!("{messages:?}").contains("private"));
        assert!(matches!(messages[2], ca::AgentMessage::Assistant { .. }));
        assert!(matches!(messages[3], ca::AgentMessage::ToolResult { .. }));
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
        let replayed = to_agent_messages(Some(&transcript));
        assert_eq!(replayed.len(), 4);
        assert!(matches!(replayed[2], ca::AgentMessage::Assistant { .. }));
        assert!(matches!(replayed[3], ca::AgentMessage::ToolResult { .. }));
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

        let messages = to_agent_messages(Some(&transcript));
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
        let messages = to_agent_messages(Some(&transcript));
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
        let messages = to_agent_messages(Some(&transcript));
        let rendered = format!("{messages:?}");
        assert!(rendered.contains("skill_123"));
        assert!(rendered.contains("rev_456"));
        assert!(rendered.contains("brainstorming"));
    }

    #[test]
    fn resumed_proposal_context_is_bounded_without_changing_typed_state() {
        let transcript = ResumeTranscript {
            truncated: false,
            items: vec![ResumeItem::ProposedPlan {
                plan: ProposedPlan {
                    id: "plan-large".into(),
                    revision: 3,
                    markdown: "x".repeat(8_000),
                    status: ProposedPlanStatus::AwaitingDecision,
                },
            }],
        };
        let messages = to_agent_messages(Some(&transcript));
        let ca::AgentMessage::Custom { payload, .. } = &messages[0] else {
            panic!("expected proposed-plan context");
        };
        let markdown = payload["markdown"].as_str().unwrap();
        assert_eq!(markdown.chars().count(), 6_000);
        assert!(markdown.contains("proposal middle omitted"));
        assert_eq!(payload["id"], "plan-large");
        assert_eq!(payload["revision"], 3);
    }
}
