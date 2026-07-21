//! Conversion from the provider-agnostic persisted transcript into the local
//! loop's canonical typed history.

use agent_core::{ContentBlock, ResumeItem, ResumeTranscript, Role, ToolStatus};
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
            ResumeItem::Message { role, blocks } => {
                let text = visible_text(blocks);
                if text.trim().is_empty() {
                    continue;
                }
                match role {
                    Role::User => messages.push(ca::AgentMessage::User {
                        content: ca::UserContent::Text(text),
                        timestamp: None,
                    }),
                    Role::Agent => messages.push(ca::AgentMessage::Assistant {
                        content: ca::AssistantContent::text(text),
                        stop_reason: ca::StopReason::EndTurn,
                        error_message: None,
                        timestamp: None,
                        usage: None,
                    }),
                    Role::System => {}
                }
            }
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
