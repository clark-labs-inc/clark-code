//! Event projection: pure, idempotent reduction of an [`AgentEvent`] stream into
//! a [`Snapshot`] the UI renders. This is the single source of truth that the
//! old web/mobile apps re-implemented separately in TypeScript — here it lives
//! once and ships to native and WASM.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::domain::*;
use crate::ids::{RunId, SessionId, ToolCallId};

/// A run as the UI sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunView {
    pub id: RunId,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RunOutcome>,
}

/// One ordered entry in the conversation timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum TimelineItem {
    Message {
        run: RunId,
        role: Role,
        blocks: Vec<ContentBlock>,
    },
    /// Reference into [`Snapshot::tool_calls`] (kept by id so updates are O(1)).
    ToolCall {
        id: ToolCallId,
    },
    /// Reference into [`Snapshot::artifacts`] — rendered inline where produced.
    Artifact {
        id: String,
    },
    Plan,
}

/// Everything the UI renders for a session. Pushed to the frontend (whole or
/// diffed) after each applied event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    pub runs: IndexMap<RunId, RunView>,
    pub timeline: Vec<TimelineItem>,
    pub tool_calls: IndexMap<ToolCallId, ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_permission: Option<PermissionRequest>,
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<WorkspaceFocus>,
}

impl Snapshot {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Apply a single event to a snapshot. Pure and idempotent w.r.t. identity
/// (re-applying a `ToolCallUpdate` yields the same state).
pub fn apply(snapshot: &mut Snapshot, event: &AgentEvent) {
    match event {
        AgentEvent::RunStarted { run } => {
            snapshot.runs.entry(run.clone()).or_insert_with(|| RunView {
                id: run.clone(),
                status: RunStatus::Running,
                outcome: None,
            });
        }

        AgentEvent::MessageChunk { run, role, delta } => {
            // Merge into the trailing message of the same run+role; else push.
            if let Some(TimelineItem::Message {
                run: last_run,
                role: last_role,
                blocks,
            }) = snapshot.timeline.last_mut()
            {
                if last_run == run && last_role == role {
                    merge_block(blocks, delta);
                    return;
                }
            }
            snapshot.timeline.push(TimelineItem::Message {
                run: run.clone(),
                role: *role,
                blocks: vec![delta.clone()],
            });
        }

        AgentEvent::ToolCall { call, .. } => {
            let already = snapshot.tool_calls.contains_key(&call.id);
            snapshot.tool_calls.insert(call.id.clone(), call.clone());
            if !already {
                snapshot.timeline.push(TimelineItem::ToolCall {
                    id: call.id.clone(),
                });
            }
        }

        AgentEvent::ToolCallUpdate { id, patch, .. } => {
            if let Some(tc) = snapshot.tool_calls.get_mut(id) {
                if let Some(t) = &patch.title {
                    tc.title = t.clone();
                }
                if let Some(k) = patch.kind {
                    tc.kind = k;
                }
                if let Some(s) = patch.status {
                    tc.status = s;
                }
                if let Some(loc) = &patch.locations {
                    tc.locations = loc.clone();
                }
                tc.content.extend(patch.append_content.iter().cloned());
            }
        }

        AgentEvent::Plan { plan, .. } => {
            if snapshot.plan.is_none() {
                snapshot.timeline.push(TimelineItem::Plan);
            }
            snapshot.plan = Some(plan.clone());
        }

        AgentEvent::PermissionRequest { request } => {
            snapshot.pending_permission = Some(request.clone());
        }

        AgentEvent::Artifact { artifact, .. } => {
            if let Some(existing) = snapshot.artifacts.iter_mut().find(|a| a.id == artifact.id) {
                // Update in place (e.g. URL filled in after publish).
                *existing = artifact.clone();
            } else {
                snapshot.artifacts.push(artifact.clone());
                snapshot.timeline.push(TimelineItem::Artifact {
                    id: artifact.id.clone(),
                });
            }
        }

        AgentEvent::Surface { focus } => {
            snapshot.focus = Some(focus.clone());
        }

        AgentEvent::ModeChanged { .. } => {}

        AgentEvent::RunFinished { run, outcome } => {
            let view = snapshot.runs.entry(run.clone()).or_insert_with(|| RunView {
                id: run.clone(),
                status: outcome.status,
                outcome: None,
            });
            view.status = outcome.status;
            view.outcome = Some(outcome.clone());
            // A finished run clears any permission gate tied to it.
            snapshot.pending_permission = None;
        }

        AgentEvent::Error { run, .. } => {
            if let Some(run) = run {
                if let Some(view) = snapshot.runs.get_mut(run) {
                    view.status = RunStatus::Failed;
                }
            }
        }
    }
}

/// Fold a whole event sequence into a fresh snapshot. Handy for hydration/replay
/// and for tests.
pub fn reduce_all<'a, I>(events: I) -> Snapshot
where
    I: IntoIterator<Item = &'a AgentEvent>,
{
    let mut snap = Snapshot::new();
    for ev in events {
        apply(&mut snap, ev);
    }
    snap
}

/// Append `delta` to `blocks`, concatenating adjacent text for smooth streaming.
fn merge_block(blocks: &mut Vec<ContentBlock>, delta: &ContentBlock) {
    if let (Some(ContentBlock::Text { text: last }), ContentBlock::Text { text: add }) =
        (blocks.last_mut(), delta)
    {
        last.push_str(add);
    } else {
        blocks.push(delta.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> RunId {
        RunId::new("run-1")
    }

    #[test]
    fn streaming_text_chunks_merge_into_one_message() {
        let events = vec![
            AgentEvent::RunStarted { run: run() },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::Agent,
                delta: ContentBlock::text("Hel"),
            },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::Agent,
                delta: ContentBlock::text("lo"),
            },
        ];
        let snap = reduce_all(&events);
        assert_eq!(snap.timeline.len(), 1);
        match &snap.timeline[0] {
            TimelineItem::Message { blocks, role, .. } => {
                assert_eq!(*role, Role::Agent);
                assert_eq!(blocks, &vec![ContentBlock::text("Hello")]);
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[test]
    fn different_role_starts_a_new_message() {
        let events = vec![
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::User,
                delta: ContentBlock::text("hi"),
            },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::Agent,
                delta: ContentBlock::text("hey"),
            },
        ];
        let snap = reduce_all(&events);
        assert_eq!(snap.timeline.len(), 2);
    }

    #[test]
    fn tool_call_then_update_patches_in_place_without_duplicate_timeline_entry() {
        let id = ToolCallId::new("tc-1");
        let events = vec![
            AgentEvent::ToolCall {
                run: run(),
                call: ToolCall {
                    id: id.clone(),
                    title: "Reading file".into(),
                    kind: ToolKind::Read,
                    status: ToolStatus::Pending,
                    locations: vec![],
                    content: vec![],
                    raw_input: None,
                },
            },
            AgentEvent::ToolCallUpdate {
                run: run(),
                id: id.clone(),
                patch: ToolCallPatch {
                    status: Some(ToolStatus::Completed),
                    append_content: vec![ContentBlock::text("file contents")],
                    ..Default::default()
                },
            },
        ];
        let snap = reduce_all(&events);
        assert_eq!(snap.tool_calls.len(), 1);
        assert_eq!(
            snap.timeline
                .iter()
                .filter(|i| matches!(i, TimelineItem::ToolCall { .. }))
                .count(),
            1
        );
        let tc = &snap.tool_calls[&id];
        assert_eq!(tc.status, ToolStatus::Completed);
        assert_eq!(tc.content, vec![ContentBlock::text("file contents")]);
    }

    #[test]
    fn run_finished_sets_outcome_and_clears_permission() {
        let mut snap = Snapshot::new();
        apply(
            &mut snap,
            &AgentEvent::PermissionRequest {
                request: PermissionRequest {
                    id: crate::ids::PermissionRequestId::new("p1"),
                    session: SessionId::new("s1"),
                    tool_call: None,
                    title: "Run command?".into(),
                    options: vec![],
                },
            },
        );
        assert!(snap.pending_permission.is_some());
        apply(
            &mut snap,
            &AgentEvent::RunFinished {
                run: run(),
                outcome: RunOutcome {
                    status: RunStatus::Done,
                    stop_reason: Some("end_turn".into()),
                    error: None,
                },
            },
        );
        assert!(snap.pending_permission.is_none());
        assert_eq!(snap.runs[&run()].status, RunStatus::Done);
    }

    #[test]
    fn plan_pushes_one_timeline_marker_and_updates_in_place() {
        let mk = |s: PlanPhaseStatus| AgentEvent::Plan {
            run: run(),
            plan: Plan {
                phases: vec![PlanPhase {
                    title: "step".into(),
                    status: s,
                    priority: None,
                }],
            },
        };
        let snap = reduce_all(&[mk(PlanPhaseStatus::Pending), mk(PlanPhaseStatus::Completed)]);
        assert_eq!(
            snap.timeline
                .iter()
                .filter(|i| matches!(i, TimelineItem::Plan))
                .count(),
            1
        );
        assert_eq!(
            snap.plan.unwrap().phases[0].status,
            PlanPhaseStatus::Completed
        );
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let snap = reduce_all(&[
            AgentEvent::RunStarted { run: run() },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::Agent,
                delta: ContentBlock::text("hi"),
            },
        ]);
        let json = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
    }

    /// Conformance: applying every event variant must never panic and must
    /// settle into a sensible snapshot. This locks the reducer contract every
    /// provider relies on.
    #[test]
    fn every_event_variant_reduces_without_panic() {
        let tc = ToolCallId::new("tc");
        let all = vec![
            AgentEvent::RunStarted { run: run() },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::User,
                delta: ContentBlock::text("hello"),
            },
            AgentEvent::ToolCall {
                run: run(),
                call: ToolCall {
                    id: tc.clone(),
                    title: "t".into(),
                    kind: ToolKind::Execute,
                    status: ToolStatus::Pending,
                    locations: vec![FsLocation {
                        path: "a.rs".into(),
                        line: Some(2),
                    }],
                    content: vec![],
                    raw_input: Some(serde_json::json!({"cmd": "ls"})),
                },
            },
            AgentEvent::ToolCallUpdate {
                run: run(),
                id: tc.clone(),
                patch: ToolCallPatch {
                    status: Some(ToolStatus::Completed),
                    ..Default::default()
                },
            },
            AgentEvent::Plan {
                run: run(),
                plan: Plan::default(),
            },
            AgentEvent::PermissionRequest {
                request: PermissionRequest {
                    id: crate::ids::PermissionRequestId::new("p"),
                    session: SessionId::new("s"),
                    tool_call: Some(tc.clone()),
                    title: "ok?".into(),
                    options: vec![PermissionOption {
                        id: "a".into(),
                        label: "Allow".into(),
                        kind: PermissionOptionKind::AllowOnce,
                    }],
                },
            },
            AgentEvent::Artifact {
                run: run(),
                artifact: Artifact {
                    id: "art".into(),
                    title: "report.pdf".into(),
                    kind: ArtifactKind::Pdf,
                    mime_type: Some("application/pdf".into()),
                    uri: None,
                    tool_call: Some(tc.clone()),
                },
            },
            AgentEvent::Surface {
                focus: WorkspaceFocus {
                    surface: WorkspaceSurfaceKind::Browser,
                    path: None,
                    url: Some("https://x".into()),
                    is_dir: None,
                    tool_call: None,
                },
            },
            AgentEvent::ModeChanged {
                session: SessionId::new("s"),
                mode: "plan".into(),
            },
            AgentEvent::Error {
                code: "boom".into(),
                message: "failed".into(),
                run: Some(run()),
            },
            AgentEvent::RunFinished {
                run: run(),
                outcome: RunOutcome {
                    status: RunStatus::Done,
                    stop_reason: Some("end_turn".into()),
                    error: None,
                },
            },
        ];

        // Idempotency: folding the sequence twice yields the same snapshot.
        let once = reduce_all(&all);
        let twice = reduce_all(all.iter().chain(all.iter()));
        assert_eq!(once.tool_calls.len(), twice.tool_calls.len());
        assert_eq!(once.artifacts.len(), 1, "artifacts dedupe by id");
        assert_eq!(twice.artifacts.len(), 1, "re-applying keeps one artifact");
        assert_eq!(once.focus.unwrap().surface, WorkspaceSurfaceKind::Browser);
        assert_eq!(once.runs[&run()].status, RunStatus::Done);
        assert!(once.pending_permission.is_none());
    }
}
