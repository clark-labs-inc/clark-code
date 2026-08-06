//! Translate ACP wire JSON into the normalized `agent-core` domain.
//!
//! Parsing is deliberately defensive (works off `serde_json::Value`, tolerates
//! unknown enum values and missing fields) so the adapter survives variation
//! across agents and minor schema drift. Unknown content/updates are dropped
//! rather than failing the run.

use agent_core::domain::*;
use agent_core::ids::{PermissionRequestId, RunId, SessionId, ToolCallId};
use serde_json::Value;

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// ACP `ContentBlock` → domain. Handles `mimeType` casing and the nested
/// `resource` object.
pub fn content_block(v: &Value) -> Option<ContentBlock> {
    match v.get("type").and_then(Value::as_str)? {
        "text" => Some(ContentBlock::Text {
            text: s(v, "text").unwrap_or_default().to_string(),
        }),
        "image" => Some(ContentBlock::Image {
            mime_type: s(v, "mimeType").unwrap_or_default().to_string(),
            data: s(v, "data").unwrap_or_default().to_string(),
            uri: s(v, "uri").map(String::from),
        }),
        "audio" => Some(ContentBlock::Audio {
            mime_type: s(v, "mimeType").unwrap_or_default().to_string(),
            data: s(v, "data").unwrap_or_default().to_string(),
        }),
        "resource" => {
            let r = v.get("resource");
            Some(ContentBlock::Resource {
                uri: r.and_then(|r| s(r, "uri")).unwrap_or_default().to_string(),
                mime_type: r.and_then(|r| s(r, "mimeType")).map(String::from),
                text: r.and_then(|r| s(r, "text")).map(String::from),
                data: r.and_then(|r| s(r, "blob")).map(String::from),
            })
        }
        "resource_link" => Some(ContentBlock::ResourceLink {
            uri: s(v, "uri").unwrap_or_default().to_string(),
            name: s(v, "name").map(String::from),
        }),
        _ => None,
    }
}

/// Domain `ContentBlock` → ACP wire JSON (for `session/prompt` input).
pub fn content_block_to_acp(b: &ContentBlock) -> Value {
    match b {
        ContentBlock::Text { text } => serde_json::json!({ "type": "text", "text": text }),
        // Reasoning is model-produced, never user input, so it never legitimately
        // reaches this user-prompt path — map it to text if it ever does.
        ContentBlock::Thinking { text } => serde_json::json!({ "type": "text", "text": text }),
        ContentBlock::Image {
            mime_type,
            data,
            uri,
        } => {
            serde_json::json!({ "type": "image", "mimeType": mime_type, "data": data, "uri": uri })
        }
        ContentBlock::Audio { mime_type, data } => {
            serde_json::json!({ "type": "audio", "mimeType": mime_type, "data": data })
        }
        ContentBlock::Resource {
            uri,
            mime_type,
            text,
            data,
        } => {
            let resource = match data {
                Some(blob) => serde_json::json!({
                    "uri": uri, "mimeType": mime_type, "blob": blob
                }),
                None => serde_json::json!({
                    "uri": uri, "mimeType": mime_type, "text": text
                }),
            };
            serde_json::json!({ "type": "resource", "resource": resource })
        }
        ContentBlock::ResourceLink { uri, name } => {
            serde_json::json!({ "type": "resource_link", "uri": uri, "name": name })
        }
        ContentBlock::SkillReference { name, .. } => {
            serde_json::json!({ "type": "text", "text": format!("[Selected skill: {name}]") })
        }
    }
}

/// An attached file → a capability-gated inline ACP content block.
pub fn attachment_to_acp(att: &PendingUpload) -> Value {
    if att.is_image() {
        serde_json::json!({
            "type": "image",
            "mimeType": att.content_type,
            "data": att.data_base64,
        })
    } else if att.content_type.starts_with("audio/") {
        serde_json::json!({
            "type": "audio",
            "mimeType": att.content_type,
            "data": att.data_base64,
        })
    } else {
        serde_json::json!({
            "type": "resource",
            "resource": {
                "uri": format!("attachment://{}", att.filename),
                "mimeType": att.content_type,
                "blob": att.data_base64,
            },
        })
    }
}

pub fn tool_kind(k: Option<&str>) -> ToolKind {
    match k.unwrap_or("") {
        "read" | "read_file" => ToolKind::Read,
        "edit" | "modify_file" | "create_file" => ToolKind::Edit,
        "delete" | "delete_file" => ToolKind::Delete,
        "move" | "rename" => ToolKind::Move,
        "search" | "grep" => ToolKind::Search,
        "execute" | "execute_command" => ToolKind::Execute,
        "think" => ToolKind::Think,
        "fetch" => ToolKind::Fetch,
        "view_image" => ToolKind::ViewImage,
        "generate_image" | "image_generation" => ToolKind::GenerateImage,
        _ => ToolKind::Other,
    }
}

pub fn tool_status(s: Option<&str>) -> ToolStatus {
    match s.unwrap_or("") {
        "pending" => ToolStatus::Pending,
        "in_progress" | "running" | "approved" => ToolStatus::InProgress,
        "completed" | "success" => ToolStatus::Completed,
        "cancelled" => ToolStatus::Cancelled,
        "failed" | "error" => ToolStatus::Failed,
        _ => ToolStatus::Pending,
    }
}

fn plan_status(s: Option<&str>) -> ChecklistStatus {
    match s.unwrap_or("") {
        "in_progress" => ChecklistStatus::InProgress,
        "completed" => ChecklistStatus::Completed,
        _ => ChecklistStatus::Pending,
    }
}

fn locations(v: Option<&Value>) -> Vec<FsLocation> {
    v.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|l| {
                    s(l, "path").map(|path| FsLocation {
                        path: path.to_string(),
                        line: l.get("line").and_then(Value::as_u64).map(|n| n as u32),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// ACP tool-call `content` entries → content blocks. Handles `{type:"content"}`
/// wrappers and renders `{type:"diff"}` entries as readable text.
fn tool_content(v: Option<&Value>) -> Vec<ContentBlock> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|entry| match entry.get("type").and_then(Value::as_str) {
            Some("content") => entry.get("content").and_then(content_block),
            Some("diff") => {
                let path = s(entry, "path").unwrap_or("file");
                let old = s(entry, "oldText").unwrap_or("");
                let new = s(entry, "newText").unwrap_or("");
                let mut out = format!("diff {path}\n");
                for line in old.lines() {
                    out.push('-');
                    out.push_str(line);
                    out.push('\n');
                }
                for line in new.lines() {
                    out.push('+');
                    out.push_str(line);
                    out.push('\n');
                }
                Some(ContentBlock::text(out))
            }
            // Some agents inline a bare content block without a wrapper.
            _ => content_block(entry),
        })
        .collect()
}

fn public_tool_title(v: &Value) -> Option<String> {
    let title = s(v, "title")?.trim();
    if title.is_empty() {
        return None;
    }
    let matches_tool_name =
        s(v, "toolName").is_some_and(|name| title.eq_ignore_ascii_case(name.trim()));
    let looks_like_identifier = (title.contains('_') || title.contains('-'))
        && title
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'));
    (!matches_tool_name && !looks_like_identifier).then(|| title.to_string())
}

fn input_argument<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get("rawInput")
        .and_then(|input| s(input, key))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn fallback_tool_title(v: &Value, kind: ToolKind) -> String {
    let path = locations(v.get("locations"))
        .first()
        .map(|location| location.path.clone())
        .or_else(|| input_argument(v, "path").map(String::from));
    match kind {
        ToolKind::Read => path
            .map(|path| format!("Read {path}"))
            .unwrap_or_else(|| "Reading a file".to_string()),
        ToolKind::Edit => path
            .map(|path| format!("Edit {path}"))
            .unwrap_or_else(|| "Editing a file".to_string()),
        ToolKind::Delete => path
            .map(|path| format!("Delete {path}"))
            .unwrap_or_else(|| "Deleting a file".to_string()),
        ToolKind::Move => path
            .map(|path| format!("Move {path}"))
            .unwrap_or_else(|| "Moving a file".to_string()),
        ToolKind::Search => input_argument(v, "query")
            .or_else(|| input_argument(v, "pattern"))
            .map(|query| format!("Search for {query}"))
            .unwrap_or_else(|| "Searching".to_string()),
        ToolKind::Execute => input_argument(v, "command")
            .map(String::from)
            .unwrap_or_else(|| "a command".to_string()),
        ToolKind::Think => "Thinking".to_string(),
        ToolKind::Fetch => input_argument(v, "url")
            .map(|url| format!("Read {url}"))
            .unwrap_or_else(|| "Reading a web page".to_string()),
        ToolKind::ViewImage => "Viewing an image".to_string(),
        ToolKind::GenerateImage => "Generating an image".to_string(),
        ToolKind::Research => "Researching".to_string(),
        _ => "Working".to_string(),
    }
}

fn tool_call(v: &Value) -> ToolCall {
    let kind = tool_kind(s(v, "kind"));
    ToolCall {
        id: ToolCallId::new(s(v, "toolCallId").unwrap_or_default()),
        tool_name: s(v, "toolName").map(String::from),
        title: public_tool_title(v).unwrap_or_else(|| fallback_tool_title(v, kind)),
        kind,
        status: tool_status(s(v, "status")),
        locations: locations(v.get("locations")),
        content: tool_content(v.get("content")),
        raw_input: v.get("rawInput").cloned(),
        progress: None,
    }
}

fn tool_call_patch(v: &Value) -> ToolCallPatch {
    ToolCallPatch {
        title: public_tool_title(v),
        kind: v.get("kind").map(|k| tool_kind(k.as_str())),
        status: v.get("status").map(|st| tool_status(st.as_str())),
        locations: v.get("locations").map(|l| locations(Some(l))),
        append_content: tool_content(v.get("content")),
        replace_content: None,
        progress: None,
    }
}

fn execution_checklist(v: &Value) -> ExecutionChecklist {
    let steps = v
        .get("entries")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|e| ChecklistStep {
                    plan_step_id: None,
                    title: s(e, "content").unwrap_or_default().to_string(),
                    status: plan_status(s(e, "status")),
                    priority: s(e, "priority").map(String::from),
                })
                .collect()
        })
        .unwrap_or_default();
    ExecutionChecklist { steps, revision: 0 }
}

/// Translate one `session/update` `update` object into an [`AgentEvent`].
/// Returns `None` for updates we don't surface (e.g. available-commands).
pub fn update_to_event(update: &Value, run: &RunId) -> Option<AgentEvent> {
    // Accept either discriminator field name for resilience.
    let ty = update
        .get("type")
        .or_else(|| update.get("sessionUpdate"))
        .and_then(Value::as_str)?;
    match ty {
        "agent_message_chunk" => Some(AgentEvent::MessageChunk {
            run: run.clone(),
            role: Role::Agent,
            delta: content_block(update.get("content")?)?,
        }),
        "agent_thought_chunk" => Some(AgentEvent::MessageChunk {
            run: run.clone(),
            role: Role::Agent,
            delta: match content_block(update.get("content")?)? {
                ContentBlock::Text { text } => ContentBlock::thinking(text),
                ContentBlock::Thinking { text } => ContentBlock::thinking(text),
                _ => return None,
            },
        }),
        "user_message_chunk" => Some(AgentEvent::MessageChunk {
            run: run.clone(),
            role: Role::User,
            delta: content_block(update.get("content")?)?,
        }),
        "tool_call" => Some(AgentEvent::ToolCall {
            run: run.clone(),
            call: tool_call(update),
        }),
        "tool_call_update" => Some(AgentEvent::ToolCallUpdate {
            run: run.clone(),
            id: ToolCallId::new(s(update, "toolCallId").unwrap_or_default()),
            patch: tool_call_patch(update),
        }),
        "plan" => Some(AgentEvent::ExecutionChecklistUpdated {
            run: run.clone(),
            checklist: execution_checklist(update),
            explanation: None,
        }),
        _ => None,
    }
}

/// Build a [`PermissionRequest`] from a `session/request_permission` params plus
/// the JSON-RPC id (reused as the UI request id).
pub fn permission_request(params: &Value, rpc_id: &str) -> PermissionRequest {
    let tc = params.get("toolCall");
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    Some(PermissionOption {
                        id: s(o, "optionId")?.to_string(),
                        label: s(o, "name").unwrap_or("Option").to_string(),
                        kind: match s(o, "kind").unwrap_or("") {
                            "allow_once" => PermissionOptionKind::AllowOnce,
                            "allow_always" => PermissionOptionKind::AllowAlways,
                            "reject_always" => PermissionOptionKind::RejectAlways,
                            _ => PermissionOptionKind::RejectOnce,
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    PermissionRequest {
        id: PermissionRequestId::new(rpc_id),
        session: SessionId::new(s(params, "sessionId").unwrap_or_default()),
        tool_call: tc.and_then(|t| s(t, "toolCallId")).map(ToolCallId::new),
        title: tc
            .and_then(public_tool_title)
            .unwrap_or_else(|| "Permission required".to_string()),
        options,
        detail: None,
        risk: None,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run() -> RunId {
        RunId::new("r1")
    }

    #[test]
    fn agent_message_chunk_maps_to_message() {
        let u = json!({"type":"agent_message_chunk","content":{"type":"text","text":"hi"}});
        match update_to_event(&u, &run()).unwrap() {
            AgentEvent::MessageChunk { role, delta, .. } => {
                assert_eq!(role, Role::Agent);
                assert_eq!(delta, ContentBlock::text("hi"));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn agent_thought_chunk_maps_to_private_agent_reasoning() {
        let update = json!({
            "type": "agent_thought_chunk",
            "content": { "type": "text", "text": "inspect the full history" }
        });
        match update_to_event(&update, &run()).unwrap() {
            AgentEvent::MessageChunk { role, delta, .. } => {
                assert_eq!(role, Role::Agent);
                assert_eq!(delta, ContentBlock::thinking("inspect the full history"));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn tool_call_with_read_file_kind_and_locations() {
        let u = json!({
            "type":"tool_call","toolCallId":"t1","title":"Read main.rs",
            "kind":"read_file","status":"in_progress",
            "locations":[{"path":"/abs/main.rs","line":3}],
            "content":[{"type":"content","content":{"type":"text","text":"body"}}]
        });
        match update_to_event(&u, &run()).unwrap() {
            AgentEvent::ToolCall { call, .. } => {
                assert_eq!(call.kind, ToolKind::Read);
                assert_eq!(call.status, ToolStatus::InProgress);
                assert_eq!(call.locations[0].line, Some(3));
                assert_eq!(call.content, vec![ContentBlock::text("body")]);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn cancelled_tool_status_stays_distinct_from_failure() {
        assert_eq!(tool_status(Some("cancelled")), ToolStatus::Cancelled);
        assert_eq!(tool_status(Some("failed")), ToolStatus::Failed);
    }

    #[test]
    fn raw_or_blank_tool_titles_get_semantic_fallbacks() {
        let raw = json!({
            "type":"tool_call", "toolCallId":"t1", "toolName":"web_fetch",
            "title":"web_fetch", "kind":"fetch",
            "rawInput":{"url":"https://example.com/docs"}
        });
        let blank = json!({
            "type":"tool_call", "toolCallId":"t2", "title":" ", "kind":"fetch"
        });

        let AgentEvent::ToolCall { call: raw_call, .. } = update_to_event(&raw, &run()).unwrap()
        else {
            panic!("expected tool call");
        };
        let AgentEvent::ToolCall {
            call: blank_call, ..
        } = update_to_event(&blank, &run()).unwrap()
        else {
            panic!("expected tool call");
        };

        assert_eq!(raw_call.title, "Read https://example.com/docs");
        assert_eq!(blank_call.title, "Reading a web page");
    }

    #[test]
    fn raw_tool_title_updates_are_ignored() {
        let update = json!({
            "type":"tool_call_update", "toolCallId":"t1",
            "toolName":"web_fetch", "title":"web_fetch"
        });

        let AgentEvent::ToolCallUpdate { patch, .. } = update_to_event(&update, &run()).unwrap()
        else {
            panic!("expected tool call update");
        };

        assert_eq!(patch.title, None);
    }

    #[test]
    fn image_tool_kinds_are_preserved() {
        assert_eq!(tool_kind(Some("view_image")), ToolKind::ViewImage);
        assert_eq!(tool_kind(Some("image_generation")), ToolKind::GenerateImage);
    }

    #[test]
    fn plan_entries_map_to_phases() {
        let u = json!({"type":"plan","entries":[
            {"content":"step a","status":"completed","priority":"high"},
            {"content":"step b","status":"in_progress"}
        ]});
        match update_to_event(&u, &run()).unwrap() {
            AgentEvent::ExecutionChecklistUpdated { checklist, .. } => {
                assert_eq!(checklist.steps.len(), 2);
                assert_eq!(checklist.steps[0].status, ChecklistStatus::Completed);
                assert_eq!(checklist.steps[1].status, ChecklistStatus::InProgress);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn unknown_update_is_ignored() {
        let u = json!({"type":"available_commands_update","availableCommands":[]});
        assert!(update_to_event(&u, &run()).is_none());
    }

    #[test]
    fn permission_request_parses_options() {
        let p = json!({
            "sessionId":"s1",
            "toolCall":{"toolCallId":"t1","title":"Run cargo build"},
            "options":[
                {"optionId":"a","name":"Allow","kind":"allow_once"},
                {"optionId":"r","name":"Reject","kind":"reject_once"}
            ]
        });
        let req = permission_request(&p, "42");
        assert_eq!(req.id.as_str(), "42");
        assert_eq!(req.tool_call.as_ref().unwrap().as_str(), "t1");
        assert_eq!(req.options.len(), 2);
        assert_eq!(req.options[0].kind, PermissionOptionKind::AllowOnce);
    }

    #[test]
    fn image_attachment_becomes_acp_image_block() {
        let att = PendingUpload {
            filename: "shot.png".into(),
            content_type: "image/png".into(),
            data_base64: "QUJD".into(),
        };
        let v = attachment_to_acp(&att);
        assert_eq!(v["type"], "image");
        assert_eq!(v["mimeType"], "image/png");
        assert_eq!(v["data"], "QUJD");
    }

    #[test]
    fn non_image_attachment_becomes_complete_embedded_resource() {
        let att = PendingUpload {
            filename: "report.pdf".into(),
            content_type: "application/pdf".into(),
            data_base64: "x".into(),
        };
        let v = attachment_to_acp(&att);
        assert_eq!(v["type"], "resource");
        assert_eq!(v["resource"]["uri"], "attachment://report.pdf");
        assert_eq!(v["resource"]["mimeType"], "application/pdf");
        assert_eq!(v["resource"]["blob"], "x");
    }

    #[test]
    fn embedded_blob_round_trips_without_losing_bytes() {
        let wire = json!({
            "type": "resource",
            "resource": {
                "uri": "attachment://archive.bin",
                "mimeType": "application/octet-stream",
                "blob": "QUJDREVGRw=="
            }
        });
        let block = content_block(&wire).unwrap();
        assert_eq!(content_block_to_acp(&block), wire);
    }
}
