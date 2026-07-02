//! Translate Clark gateway events into the normalized `agent-core` domain.
//!
//! Clean-room mapping derived from observed `{type:"event", event:{type, data}}`
//! frames. Defensive: unknown event types (checkpoints, gate/policy, token
//! usage, thinking) are ignored so the run keeps streaming.

use agent_core::domain::*;
use agent_core::ids::{RunId, ToolCallId};
use serde_json::Value;

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_str()
}

fn basename(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

/// Classify a Clark work tool into a presentational kind.
fn tool_kind(tool_name: &str, action: Option<&str>) -> ToolKind {
    let n = tool_name.to_ascii_lowercase();
    if n.contains("browser") {
        return ToolKind::Fetch;
    }
    if n.contains("search") {
        return ToolKind::Search;
    }
    if n.contains("shell") || n.contains("bash") || n.contains("exec") || n.contains("terminal") {
        return ToolKind::Execute;
    }
    if n.contains("publish") || n.contains("deploy") {
        return ToolKind::Fetch;
    }
    if n.contains("artifact")
        || n.contains("website")
        || n.contains("slide")
        || n.contains("deck")
        || n.contains("present")
        || n.contains("create")
        || n.contains("write")
    {
        return ToolKind::Edit;
    }
    if n.contains("file") {
        return match action.unwrap_or("") {
            "read" | "view" | "cat" | "open" => ToolKind::Read,
            "delete" | "rm" | "remove" => ToolKind::Delete,
            "move" | "rename" => ToolKind::Move,
            _ => ToolKind::Edit,
        };
    }
    ToolKind::Other
}

/// A user-facing label derived ONLY from the work kind and the call's arguments
/// (paths / queries / urls — the user's own content). It never references the
/// backend's internal tool name; those identifiers must not leak into the UI.
fn tool_title(kind: ToolKind, action: Option<&str>, args: &Value) -> String {
    let target = s(args, "path")
        .map(basename)
        .or_else(|| s(args, "name"))
        .or_else(|| s(args, "title"))
        .or_else(|| s(args, "slug"));
    let with = |verb: &str, fallback: &str| match target {
        Some(t) => format!("{verb} {t}"),
        None => fallback.to_string(),
    };
    match kind {
        ToolKind::Read => with("Read", "Reading a file"),
        ToolKind::Edit => {
            let verb = match action.unwrap_or("") {
                "write" | "create" => "Write",
                "append" => "Append to",
                _ => "Edit",
            };
            with(verb, "Editing a file")
        }
        ToolKind::Delete => with("Delete", "Deleting a file"),
        ToolKind::Move => with("Move", "Moving a file"),
        ToolKind::Search => s(args, "query")
            .or_else(|| s(args, "q"))
            .map(|q| format!("Search \u{201c}{q}\u{201d}"))
            .unwrap_or_else(|| "Searching the web".into()),
        ToolKind::Fetch => s(args, "url")
            .or_else(|| s(args, "href"))
            .map(str::to_string)
            .unwrap_or_else(|| "Working on the web".into()),
        ToolKind::Execute => s(args, "command")
            .or_else(|| s(args, "cmd"))
            .or_else(|| s(args, "script"))
            .map(str::to_string)
            .unwrap_or_else(|| "Running a command".into()),
        ToolKind::Think => "Thinking".into(),
        ToolKind::Research => s(args, "query")
            .map(|q| format!("Researching \u{201c}{q}\u{201d}"))
            .unwrap_or_else(|| "Researching".into()),
        ToolKind::Other => target
            .map(str::to_string)
            .unwrap_or_else(|| "Working".into()),
    }
}

/// A backend-provided, user-facing label if present. Explicitly excludes the
/// internal `tool_name`; only fields meant for display are consulted.
fn public_label(data: &Value) -> Option<String> {
    [
        "public_activity_label",
        "activity_label",
        "display_title",
        "label",
    ]
    .iter()
    .filter_map(|k| s(data, k))
    .find(|v| !v.trim().is_empty())
    .map(str::to_string)
}

fn plan_status(status: Option<&str>) -> PlanPhaseStatus {
    match status.unwrap_or("") {
        "running" | "in_progress" | "active" => PlanPhaseStatus::InProgress,
        "completed" | "done" | "complete" => PlanPhaseStatus::Completed,
        _ => PlanPhaseStatus::Pending,
    }
}

fn workspace_surface(name: Option<&str>) -> WorkspaceSurfaceKind {
    match name.unwrap_or("") {
        "browser" => WorkspaceSurfaceKind::Browser,
        "terminal" => WorkspaceSurfaceKind::Terminal,
        "website" => WorkspaceSurfaceKind::Website,
        _ => WorkspaceSurfaceKind::Files,
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn first_s<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| non_empty(s(v, key)))
}

fn preview_url(v: &Value) -> Option<&str> {
    v.get("preview")
        .and_then(|p| non_empty(s(p, "url")))
        .or_else(|| str_at(v, &["preview", "url"]).and_then(|u| non_empty(Some(u))))
}

fn artifact_uri(v: &Value) -> Option<&str> {
    first_s(
        v,
        &[
            "uri",
            "url",
            "preview_url",
            "artifact_url",
            "download_url",
            "pptx_url",
            "pdf_artifact_url",
        ],
    )
    .or_else(|| preview_url(v))
}

fn artifact_path(v: &Value) -> Option<&str> {
    first_s(v, &["path", "source_path", "restore_target_path"])
}

fn openable_uri<'a>(artifact: &'a Value, path: Option<&'a str>) -> Option<&'a str> {
    artifact_uri(artifact).or_else(|| {
        path.filter(|p| {
            p.starts_with("http://")
                || p.starts_with("https://")
                || p.starts_with("/api/")
                || p.starts_with("/artifacts/")
        })
    })
}

fn artifact_kind(
    kind: Option<&str>,
    mime_type: Option<&str>,
    uri: Option<&str>,
    path: Option<&str>,
    title: Option<&str>,
) -> ArtifactKind {
    let haystack = [kind, mime_type, uri, path, title]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    if haystack.contains("website")
        || haystack.contains("webpage")
        || haystack.contains("site")
        || haystack.contains("text/html")
        || haystack.contains(".html")
    {
        return ArtifactKind::Website;
    }
    if haystack.contains("presentation")
        || haystack.contains("slide")
        || haystack.contains("powerpoint")
        || haystack.contains("pptx")
        || haystack.contains(".ppt")
    {
        return ArtifactKind::Slides;
    }
    if haystack.contains("pdf") {
        return ArtifactKind::Pdf;
    }
    if haystack.contains("image/")
        || haystack.contains(".png")
        || haystack.contains(".jpg")
        || haystack.contains(".jpeg")
        || haystack.contains(".gif")
        || haystack.contains(".webp")
        || haystack.contains(".svg")
    {
        return ArtifactKind::Image;
    }
    if haystack.contains("video/")
        || haystack.contains(".mp4")
        || haystack.contains(".webm")
        || haystack.contains(".mov")
    {
        return ArtifactKind::Video;
    }
    if haystack.contains("audio/") {
        return ArtifactKind::Media;
    }
    if haystack.contains("office")
        || haystack.contains("document")
        || haystack.contains("spreadsheet")
        || haystack.contains(".docx")
        || haystack.contains(".xlsx")
        || haystack.contains(".csv")
    {
        return ArtifactKind::Office;
    }
    ArtifactKind::File
}

fn default_mime(kind: ArtifactKind) -> Option<String> {
    match kind {
        ArtifactKind::Website => Some("text/html".into()),
        ArtifactKind::Pdf => Some("application/pdf".into()),
        _ => None,
    }
}

fn terminal_artifact_values(event: &Value) -> Vec<&Value> {
    let Some(data) = event.get("data") else {
        return vec![];
    };
    let sources = [
        data.get("result_envelope")
            .and_then(|r| r.get("payload"))
            .and_then(|p| p.get("artifacts")),
        data.get("canonical_terminal_record")
            .and_then(|r| r.get("result_envelope"))
            .and_then(|e| e.get("payload"))
            .and_then(|p| p.get("artifacts")),
        data.get("raw_terminal_record")
            .and_then(|r| r.get("result_envelope"))
            .and_then(|e| e.get("payload"))
            .and_then(|p| p.get("artifacts")),
        data.get("artifacts"),
    ];

    sources
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .find(|items| !items.is_empty())
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn terminal_artifact(artifact: &Value, index: usize) -> Option<Artifact> {
    let kind_hint = first_s(artifact, &["kind", "type", "kind_label"]);
    let mime_type = first_s(artifact, &["mime_type", "content_type"]);
    let path = artifact_path(artifact);
    let uri = openable_uri(artifact, path);
    let title = first_s(artifact, &["title", "name", "filename", "label"])
        .or_else(|| uri.map(basename))
        .or_else(|| path.map(basename));
    let kind = artifact_kind(kind_hint, mime_type, uri, path, title);
    let id = if kind == ArtifactKind::Website {
        "site".to_string()
    } else {
        first_s(artifact, &["id", "artifact_id"])
            .or(uri)
            .or(path)
            .map(str::to_string)
            .unwrap_or_else(|| format!("terminal-artifact-{}", index + 1))
    };
    let title = title.map(str::to_string).unwrap_or_else(|| match kind {
        ArtifactKind::Website => "Website".into(),
        _ => format!("Output {}", index + 1),
    });

    Some(Artifact {
        id,
        title,
        kind,
        mime_type: mime_type.map(str::to_string).or_else(|| default_mime(kind)),
        uri: uri.map(str::to_string),
        tool_call: None,
    })
}

/// Map one gateway event to zero or more normalized events. Terminal artifacts
/// in completion envelopes are emitted before the final run state so the UI can
/// render finished files/decks/sites even when no separate artifact event fired.
pub fn events_to_agent(event: &Value, run: &RunId) -> Vec<AgentEvent> {
    let ty = event.get("type").and_then(Value::as_str);
    let mut events = Vec::new();
    if matches!(ty, Some("run_completed") | Some("turn_completed")) {
        for (index, artifact) in terminal_artifact_values(event).into_iter().enumerate() {
            if let Some(artifact) = terminal_artifact(artifact, index) {
                events.push(AgentEvent::Artifact {
                    run: run.clone(),
                    artifact,
                });
            }
        }
    }
    if let Some(event) = event_to_agent(event, run) {
        events.push(event);
    }
    events
}

/// Map one inner `event` object to an [`AgentEvent`]. `run` is the
/// client-synthesized run id for the active turn.
pub fn event_to_agent(event: &Value, run: &RunId) -> Option<AgentEvent> {
    let ty = event.get("type").and_then(Value::as_str)?;
    let data = event.get("data");
    match ty {
        "message_stream_delta" => {
            let delta = data.and_then(|d| d.get("delta")).and_then(Value::as_str)?;
            Some(AgentEvent::MessageChunk {
                run: run.clone(),
                role: Role::Agent,
                delta: ContentBlock::text(delta),
            })
        }

        "tool_call" => {
            let d = data?;
            let id = s(d, "tool_call_id")?;
            let name = s(d, "tool_name").unwrap_or("tool");
            let args = d.get("arguments").cloned().unwrap_or(Value::Null);
            let action = s(&args, "action");
            let path = s(&args, "path");
            let kind = tool_kind(name, action);
            // Prefer a backend public label; otherwise derive from kind + args.
            // The raw tool name is used only to pick an icon kind, never shown.
            let title = public_label(d).unwrap_or_else(|| tool_title(kind, action, &args));
            Some(AgentEvent::ToolCall {
                run: run.clone(),
                call: ToolCall {
                    id: ToolCallId::new(id),
                    title,
                    kind,
                    status: ToolStatus::InProgress,
                    locations: path
                        .map(|p| {
                            vec![FsLocation {
                                path: p.to_string(),
                                line: None,
                            }]
                        })
                        .unwrap_or_default(),
                    content: vec![],
                    raw_input: Some(args),
                },
            })
        }

        "tool_result" => {
            let d = data?;
            let id = s(d, "tool_call_id")?;
            let result = d.get("result");
            let is_error = d.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                || result
                    .and_then(|r| r.get("success"))
                    .and_then(Value::as_bool)
                    .map(|ok| !ok)
                    .unwrap_or(false);
            let body = result
                .and_then(|r| r.get("excerpt").and_then(Value::as_str))
                .or_else(|| result.and_then(|r| r.get("content").and_then(Value::as_str)))
                .unwrap_or("");
            let mut patch = ToolCallPatch {
                status: Some(if is_error {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Completed
                }),
                ..Default::default()
            };
            if !body.is_empty() {
                patch.append_content = vec![ContentBlock::text(body)];
            }
            Some(AgentEvent::ToolCallUpdate {
                run: run.clone(),
                id: ToolCallId::new(id),
                patch,
            })
        }

        "execution_plan_committed" | "execution_plan_provisional" => {
            let phases = data?
                .get("phases")?
                .as_array()?
                .iter()
                .map(|p| PlanPhase {
                    title: s(p, "title").unwrap_or_default().to_string(),
                    status: plan_status(s(p, "status")),
                    priority: None,
                })
                .collect();
            Some(AgentEvent::Plan {
                run: run.clone(),
                plan: Plan { phases },
            })
        }

        "workspace_focus" => {
            let d = data?;
            Some(AgentEvent::Surface {
                focus: WorkspaceFocus {
                    surface: workspace_surface(s(d, "surface")),
                    path: s(d, "path").map(String::from),
                    url: s(d, "url").map(String::from),
                    is_dir: d.get("is_dir").and_then(Value::as_bool),
                    tool_call: None,
                },
            })
        }

        // A spawned subagent (e.g. the website builder) reporting a step. We
        // attach its natural-language summary to the parent tool call so a long
        // create_artifact isn't a silent spinner. `summary`/`tool` are display
        // text; the internal tool name is never shown.
        "subagent_event" => {
            let d = data?;
            let summary = s(d, "summary").filter(|t| !t.trim().is_empty())?;
            let parent = d
                .get("scope")
                .and_then(|sc| s(sc, "child_storage_id"))
                .and_then(|c| c.split_once(':').map(|x| x.1))
                .filter(|p| !p.is_empty())?;
            Some(AgentEvent::ToolCallUpdate {
                run: run.clone(),
                id: ToolCallId::new(parent),
                patch: ToolCallPatch {
                    append_content: vec![ContentBlock::text(format!("• {summary}"))],
                    ..Default::default()
                },
            })
        }

        // A published website → an inline artifact with its URL. On the local
        // stack this is a preview path (relative); the engine absolutizes it.
        "website.published" => {
            let d = data?;
            let run_id = s(d, "run_id").unwrap_or_default();
            let url = s(d, "site_url")
                .or_else(|| s(d, "preview_url"))
                .or_else(|| s(d, "url"));
            Some(AgentEvent::Artifact {
                run: run.clone(),
                artifact: Artifact {
                    // Stable id so re-publishes update a single Website card in
                    // place rather than stacking a new card per run.
                    id: "site".into(),
                    title: "Website".into(),
                    kind: ArtifactKind::Website,
                    mime_type: Some("text/html".into()),
                    uri: url.map(str::to_string),
                    tool_call: (!run_id.is_empty()).then(|| ToolCallId::new(run_id)),
                },
            })
        }

        "run_completed" | "turn_completed" => {
            let outcome = str_at(event, &["data", "loop_outcome"]).unwrap_or("done");
            Some(AgentEvent::RunFinished {
                run: run.clone(),
                outcome: RunOutcome {
                    status: RunStatus::Done,
                    stop_reason: Some(outcome.to_string()),
                    error: None,
                    usage: None,
                },
            })
        }

        "run_failed" | "run_error" | "error" => {
            let message = data
                .and_then(|d| d.get("message").or_else(|| d.get("error")))
                .and_then(Value::as_str)
                .unwrap_or("run failed");
            Some(AgentEvent::RunFinished {
                run: run.clone(),
                outcome: RunOutcome {
                    status: RunStatus::Failed,
                    stop_reason: None,
                    error: Some(message.to_string()),
                    usage: None,
                },
            })
        }

        _ => None,
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
    fn message_stream_delta_becomes_agent_text() {
        let ev = json!({"type":"message_stream_delta","data":{"delta":"pong"}});
        match event_to_agent(&ev, &run()).unwrap() {
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
        match event_to_agent(&ev, &run()).unwrap() {
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
            event_to_agent(&read, &run()),
            Some(AgentEvent::ToolCall { call, .. }) if call.kind == ToolKind::Read && call.title == "Read y.md"
        ));
        let search = json!({"type":"tool_call","data":{"tool_call_id":"b","tool_name":"web_search","arguments":{"query":"joke of the day"}}});
        assert!(matches!(
            event_to_agent(&search, &run()),
            Some(AgentEvent::ToolCall { call, .. }) if call.kind == ToolKind::Search
        ));
    }

    #[test]
    fn tool_result_completes_and_appends_excerpt() {
        let ev = json!({
            "type":"tool_result",
            "data":{"tool_call_id":"t1","is_error":false,
                    "result":{"success":true,"content":"wrote 6 bytes","excerpt":"a\nb\nc\n"}}
        });
        match event_to_agent(&ev, &run()).unwrap() {
            AgentEvent::ToolCallUpdate { id, patch, .. } => {
                assert_eq!(id.as_str(), "t1");
                assert_eq!(patch.status, Some(ToolStatus::Completed));
                assert_eq!(patch.append_content, vec![ContentBlock::text("a\nb\nc\n")]);
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
        match event_to_agent(&ev, &run()).unwrap() {
            AgentEvent::Plan { plan, .. } => {
                assert_eq!(plan.phases.len(), 2);
                assert_eq!(plan.phases[0].status, PlanPhaseStatus::InProgress);
                assert_eq!(plan.phases[1].status, PlanPhaseStatus::Pending);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn workspace_focus_maps_surface() {
        let ev = json!({"type":"workspace_focus","data":{"surface":"files","path":"/x/y.txt"}});
        match event_to_agent(&ev, &run()).unwrap() {
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
            event_to_agent(&ev, &run()),
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

        let events = events_to_agent(&ev, &run());
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
    fn internal_events_ignored() {
        for t in [
            "runtime_checkpoint",
            "tool_gate_applied",
            "llm_response",
            "assistant_thinking",
        ] {
            assert!(event_to_agent(&json!({"type": t, "data": {}}), &run()).is_none());
        }
    }

    #[test]
    fn tool_titles_never_leak_internal_tool_names() {
        // create_artifact → no "create_artifact"/underscore in the label.
        let art = json!({"type":"tool_call","data":{"tool_call_id":"x","tool_name":"create_artifact","arguments":{"name":"index.html"}}});
        match event_to_agent(&art, &run()).unwrap() {
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
        match event_to_agent(&publish, &run()).unwrap() {
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
    }

    #[test]
    fn backend_public_label_is_used_when_present() {
        let ev = json!({
            "type":"tool_call",
            "data":{"tool_call_id":"z","tool_name":"create_artifact",
                    "public_activity_label":"Building the homepage","arguments":{}}
        });
        match event_to_agent(&ev, &run()).unwrap() {
            AgentEvent::ToolCall { call, .. } => assert_eq!(call.title, "Building the homepage"),
            other => panic!("got {other:?}"),
        }
    }
}
