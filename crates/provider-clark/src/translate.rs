//! Translate Clark gateway events into the normalized `agent-core` domain.
//!
//! Clean-room mapping derived from observed `{type:"event", event:{type, data}}`
//! frames. Defensive: unknown event types (checkpoints, gate/policy, token
//! usage, thinking) are ignored so the run keeps streaming.

use agent_core::domain::*;
use agent_core::ids::{PermissionRequestId, RunId, SessionId, ToolCallId};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

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
    if n == "view_image" || n.contains("image_view") {
        return ToolKind::ViewImage;
    }
    if n == "generate_image" || n.contains("image_generation") {
        return ToolKind::GenerateImage;
    }
    if n.contains("browser") || n.contains("fetch") {
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
    let target = non_empty(s(args, "path"))
        .map(basename)
        .or_else(|| non_empty(s(args, "name")))
        .or_else(|| non_empty(s(args, "title")))
        .or_else(|| non_empty(s(args, "slug")));
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
        ToolKind::Search => non_empty(s(args, "query"))
            .or_else(|| non_empty(s(args, "q")))
            .map(|q| format!("Search \u{201c}{q}\u{201d}"))
            .unwrap_or_else(|| "Searching the web".into()),
        ToolKind::Fetch => non_empty(s(args, "url"))
            .or_else(|| non_empty(s(args, "href")))
            .map(|url| format!("Read {url}"))
            .unwrap_or_else(|| "Reading a web page".into()),
        ToolKind::Execute => non_empty(s(args, "command"))
            .or_else(|| non_empty(s(args, "cmd")))
            .or_else(|| non_empty(s(args, "script")))
            .map(str::to_string)
            .unwrap_or_else(|| "Running a command".into()),
        ToolKind::Think => "Thinking".into(),
        ToolKind::Research => non_empty(s(args, "query"))
            .map(|q| format!("Researching \u{201c}{q}\u{201d}"))
            .unwrap_or_else(|| "Researching".into()),
        ToolKind::ViewImage => with("Viewed", "Viewing an image"),
        ToolKind::GenerateImage => with("Generated", "Generating an image"),
        ToolKind::Other => target
            .map(str::to_string)
            .unwrap_or_else(|| "Working".into()),
    }
}

/// A backend-provided, user-facing label if present. Explicitly excludes the
/// internal `tool_name`; only fields meant for display are consulted, and a
/// malformed display field that merely repeats that identifier is rejected.
fn public_label(data: &Value, tool_name: &str) -> Option<String> {
    let normalized_tool_name = tool_name
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-'], " ");
    [
        "public_activity_label",
        "activity_label",
        "display_title",
        "label",
    ]
    .iter()
    .filter_map(|k| s(data, k))
    .map(str::trim)
    .find(|v| {
        !v.is_empty() && v.to_ascii_lowercase().replace(['_', '-'], " ") != normalized_tool_name
    })
    .map(str::to_string)
}

fn plan_status(status: Option<&str>) -> ChecklistStatus {
    match status.unwrap_or("") {
        "running" | "in_progress" | "active" => ChecklistStatus::InProgress,
        "completed" | "done" | "complete" => ChecklistStatus::Completed,
        _ => ChecklistStatus::Pending,
    }
}

/// Map one child's `subagent_event` phase to a fan-out tile status.
fn fan_out_status(phase: &str) -> FanOutStatus {
    match phase.trim() {
        "run_completed" | "completed" | "done" | "finished" => FanOutStatus::Done,
        "run_failed" | "failed" | "error" | "cancelled" => FanOutStatus::Failed,
        "queued" | "pending" => FanOutStatus::Queued,
        // "started", "running", "in_progress", per-step tool_call/tool_result, …
        _ => FanOutStatus::Running,
    }
}

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn first_u64(v: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| v.get(key).and_then(Value::as_u64))
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
pub fn events_to_agent(event: &Value, run: &RunId, session: &SessionId) -> Vec<AgentEvent> {
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
    // The backend paused for a clarification answer (`message_ask`). Surface
    // the question as agent text and END the desktop turn: the composer
    // unblocks, and the user's next message resumes the same backend job (a
    // `user_message` clears the server-side pending gate).
    if ty == Some("message_ask") {
        if let Some(question) = event
            .get("data")
            .and_then(|d| d.get("question"))
            .and_then(Value::as_str)
            .filter(|q| !q.trim().is_empty())
        {
            events.push(AgentEvent::MessageChunk {
                run: run.clone(),
                role: Role::Agent,
                delta: ContentBlock::text(question),
            });
        }
        events.push(AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status: RunStatus::Done,
                stop_reason: Some("message_ask".to_string()),
                error: None,
                failure_kind: None,
                usage: None,
                execution: None,
            },
        });
        return events;
    }
    if let Some(event) = event_to_agent(event, run, session) {
        events.push(event);
    }
    events
}

/// Map one inner `event` object to an [`AgentEvent`]. `run` is the
/// client-synthesized run id for the active turn; `session` is the desktop
/// session (conversation) id, stamped on permission requests.
pub fn event_to_agent(event: &Value, run: &RunId, session: &SessionId) -> Option<AgentEvent> {
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
            let title = public_label(d, name).unwrap_or_else(|| tool_title(kind, action, &args));
            Some(AgentEvent::ToolCall {
                run: run.clone(),
                call: ToolCall {
                    id: ToolCallId::new(id),
                    tool_name: Some(name.to_string()),
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
                    progress: None,
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

        // The backend paused before an irreversible action and wants an
        // approve/reject decision. The `action_id` becomes the permission
        // request id — `respond` sends it back verbatim in the `confirm`
        // command. The wire resumption is binary (approved: bool), so backend
        // `choices` reduce to one approve label (the non-"Cancel" one, same
        // heuristic as the web UI) and one reject label.
        "confirmation_requested" => {
            let d = data?;
            let action_id = s(d, "action_id").filter(|id| !id.trim().is_empty())?;
            let choices = d.get("choices").and_then(Value::as_array);
            let choice_label = |want_cancel: bool| -> Option<String> {
                choices?.iter().find_map(|c| {
                    let label = s(c, "label")?.trim();
                    (!label.is_empty() && label.eq_ignore_ascii_case("cancel") == want_cancel)
                        .then(|| label.to_string())
                })
            };
            let title = s(d, "description")
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or("Clark needs your confirmation to continue")
                .to_string();
            Some(AgentEvent::PermissionRequest {
                request: PermissionRequest {
                    id: PermissionRequestId::new(action_id),
                    session: session.clone(),
                    tool_call: s(d, "tool_call_id")
                        .filter(|t| !t.trim().is_empty())
                        .map(ToolCallId::new),
                    title,
                    options: vec![
                        PermissionOption {
                            id: "approve".into(),
                            label: choice_label(false).unwrap_or_else(|| "Confirm".to_string()),
                            kind: PermissionOptionKind::AllowOnce,
                        },
                        PermissionOption {
                            id: "reject".into(),
                            label: choice_label(true).unwrap_or_else(|| "Cancel".to_string()),
                            kind: PermissionOptionKind::RejectOnce,
                        },
                    ],
                    detail: s(d, "draft_preview")
                        .filter(|p| !p.trim().is_empty())
                        .map(str::to_string),
                    risk: Some("confirm".to_string()),
                    reason: None,
                },
            })
        }

        "execution_plan_committed" | "execution_plan_provisional" => {
            let steps = data?
                .get("phases")?
                .as_array()?
                .iter()
                .map(|p| ChecklistStep {
                    title: s(p, "title").unwrap_or_default().to_string(),
                    status: plan_status(s(p, "status")),
                    priority: None,
                })
                .collect();
            Some(AgentEvent::ExecutionChecklistUpdated {
                run: run.clone(),
                checklist: ExecutionChecklist { steps, revision: 0 },
                explanation: None,
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
            let scope = d.get("scope");

            // Parallel fan-out (`subagent_map`): aggregate per-child telemetry
            // into the fan-out surface rather than appending to one tool call.
            if scope.and_then(|sc| s(sc, "spawning_tool")) == Some("subagent_map") {
                let parent = scope
                    .and_then(|sc| s(sc, "parent_tool_call_id"))
                    .filter(|p| !p.trim().is_empty())?;
                let row = scope
                    .and_then(|sc| sc.get("row_index"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let phase = first_s(d, &["phase", "event_type"]).unwrap_or("");
                let status = fan_out_status(phase);
                let summary = first_s(d, &["public_activity_label", "summary", "tool"]);
                let objective = scope.and_then(|sc| non_empty(s(sc, "input_label")));
                let label = objective
                    .or_else(|| first_s(d, &["label", "title"]))
                    .or(summary)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Task {}", row + 1));
                let now = wall_clock_ms();
                let started_at_ms = first_u64(d, &["started_at_ms", "started_ms"])
                    .or_else(|| {
                        scope.and_then(|sc| first_u64(sc, &["started_at_ms", "started_ms"]))
                    })
                    .or_else(|| (status == FanOutStatus::Running).then_some(now));
                let updated_at_ms =
                    first_u64(d, &["updated_at_ms", "timestamp_ms", "finished_ms"]).unwrap_or(now);
                return Some(AgentEvent::FanOut {
                    run: run.clone(),
                    parent: ToolCallId::new(parent),
                    agent: FanOutAgent {
                        id: row.to_string(),
                        label,
                        status,
                        objective: objective.map(str::to_string),
                        activity: match status {
                            FanOutStatus::Queued => Some("Waiting to start".into()),
                            FanOutStatus::Running => summary.map(str::to_string),
                            FanOutStatus::Done => Some("Complete".into()),
                            FanOutStatus::Failed => Some("Needs attention".into()),
                        },
                        result: matches!(status, FanOutStatus::Done | FanOutStatus::Failed)
                            .then(|| summary.map(str::to_string))
                            .flatten(),
                        attempt: first_u64(d, &["attempt"])
                            .or_else(|| scope.and_then(|sc| first_u64(sc, &["attempt"])))
                            .and_then(|value| value.try_into().ok()),
                        started_at_ms,
                        updated_at_ms: Some(updated_at_ms),
                    },
                });
            }

            // A single spawned subagent (e.g. the website builder) reporting a
            // step: attach its natural-language summary to the parent tool call
            // so a long create_artifact isn't a silent spinner. `summary` is
            // display text; the internal tool name is never shown.
            let summary = s(d, "summary").filter(|t| !t.trim().is_empty())?;
            let parent = scope
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
                    failure_kind: None,
                    usage: None,
                    execution: None,
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
                    failure_kind: None,
                    usage: None,
                    execution: None,
                },
            })
        }

        _ => None,
    }
}

#[cfg(test)]
#[path = "translate_tests.rs"]
mod tests;
