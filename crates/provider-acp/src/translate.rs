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
        } => serde_json::json!({
            "type": "resource",
            "resource": { "uri": uri, "mimeType": mime_type, "text": text }
        }),
        ContentBlock::ResourceLink { uri, name } => {
            serde_json::json!({ "type": "resource_link", "uri": uri, "name": name })
        }
    }
}

/// An attached file → an ACP content block. Images go inline (base64); other
/// files are surfaced as a resource link the agent can reference.
pub fn attachment_to_acp(att: &PendingUpload) -> Value {
    if att.is_image() {
        serde_json::json!({
            "type": "image",
            "mimeType": att.content_type,
            "data": att.data_base64,
        })
    } else {
        serde_json::json!({
            "type": "resource_link",
            "uri": format!("attachment://{}", att.filename),
            "name": att.filename,
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
        _ => ToolKind::Other,
    }
}

pub fn tool_status(s: Option<&str>) -> ToolStatus {
    match s.unwrap_or("") {
        "pending" => ToolStatus::Pending,
        "in_progress" | "running" | "approved" => ToolStatus::InProgress,
        "completed" | "success" => ToolStatus::Completed,
        "failed" | "error" | "cancelled" => ToolStatus::Failed,
        _ => ToolStatus::Pending,
    }
}

fn plan_status(s: Option<&str>) -> PlanPhaseStatus {
    match s.unwrap_or("") {
        "in_progress" => PlanPhaseStatus::InProgress,
        "completed" => PlanPhaseStatus::Completed,
        _ => PlanPhaseStatus::Pending,
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

fn tool_call(v: &Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(s(v, "toolCallId").unwrap_or_default()),
        title: s(v, "title").unwrap_or("Tool call").to_string(),
        kind: tool_kind(s(v, "kind")),
        status: tool_status(s(v, "status")),
        locations: locations(v.get("locations")),
        content: tool_content(v.get("content")),
        raw_input: v.get("rawInput").cloned(),
    }
}

fn tool_call_patch(v: &Value) -> ToolCallPatch {
    ToolCallPatch {
        title: s(v, "title").map(String::from),
        kind: v.get("kind").map(|k| tool_kind(k.as_str())),
        status: v.get("status").map(|st| tool_status(st.as_str())),
        locations: v.get("locations").map(|l| locations(Some(l))),
        append_content: tool_content(v.get("content")),
    }
}

fn plan(v: &Value) -> Plan {
    let phases = v
        .get("entries")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|e| PlanPhase {
                    title: s(e, "content").unwrap_or_default().to_string(),
                    status: plan_status(s(e, "status")),
                    priority: s(e, "priority").map(String::from),
                })
                .collect()
        })
        .unwrap_or_default();
    Plan { phases }
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
            role: Role::System,
            delta: content_block(update.get("content")?)?,
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
        "plan" => Some(AgentEvent::Plan {
            run: run.clone(),
            plan: plan(update),
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
            .and_then(|t| s(t, "title"))
            .unwrap_or("Permission required")
            .to_string(),
        options,
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
    fn plan_entries_map_to_phases() {
        let u = json!({"type":"plan","entries":[
            {"content":"step a","status":"completed","priority":"high"},
            {"content":"step b","status":"in_progress"}
        ]});
        match update_to_event(&u, &run()).unwrap() {
            AgentEvent::Plan { plan, .. } => {
                assert_eq!(plan.phases.len(), 2);
                assert_eq!(plan.phases[0].status, PlanPhaseStatus::Completed);
                assert_eq!(plan.phases[1].status, PlanPhaseStatus::InProgress);
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
    fn non_image_attachment_becomes_resource_link() {
        let att = PendingUpload {
            filename: "report.pdf".into(),
            content_type: "application/pdf".into(),
            data_base64: "x".into(),
        };
        let v = attachment_to_acp(&att);
        assert_eq!(v["type"], "resource_link");
        assert_eq!(v["name"], "report.pdf");
    }
}
