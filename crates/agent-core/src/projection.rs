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
    /// Pre-run working-tree checkpoint used as a change-tracking baseline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
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
    ToolCall { id: ToolCallId },
    /// Reference into [`Snapshot::artifacts`] — rendered inline where produced.
    Artifact { id: String },
    Plan {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<RunId>,
        #[serde(default)]
        plan: Plan,
    },
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
    /// Live parallel fan-out (a `subagent_map` spread across child agents), or
    /// `None` when nothing is fanning out. Rendered by the fan-out surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_out: Option<FanOut>,
}

impl Snapshot {
    pub fn new() -> Self {
        Self::default()
    }
}

fn same_artifact_identity(left: &Artifact, right: &Artifact) -> bool {
    if left.id == right.id {
        return true;
    }

    let left_uri = left.uri.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let right_uri = right
        .uri
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    matches!((left_uri, right_uri), (Some(left_uri), Some(right_uri)) if left_uri == right_uri)
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
                checkpoint: None,
            });
        }

        AgentEvent::Checkpoint { run, id } => {
            let view = snapshot.runs.entry(run.clone()).or_insert_with(|| RunView {
                id: run.clone(),
                status: RunStatus::Running,
                outcome: None,
                checkpoint: None,
            });
            view.checkpoint = Some(id.clone());
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
                if let Some(content) = &patch.replace_content {
                    tc.content = content.clone();
                }
                tc.content.extend(patch.append_content.iter().cloned());
            }
            // A status change on a gated tool means its permission prompt was
            // resolved (approved → it runs, denied → it fails), so clear the gate.
            if snapshot
                .pending_permission
                .as_ref()
                .and_then(|p| p.tool_call.as_ref())
                == Some(id)
            {
                snapshot.pending_permission = None;
            }
        }

        AgentEvent::Plan { run, plan } => {
            if let Some(TimelineItem::Plan {
                plan: existing_plan,
                ..
            }) = snapshot.timeline.iter_mut().rev().find(|item| {
                matches!(item, TimelineItem::Plan { run: Some(plan_run), .. } if plan_run == run)
            }) {
                *existing_plan = plan.clone();
            } else {
                snapshot.timeline.push(TimelineItem::Plan {
                    run: Some(run.clone()),
                    plan: plan.clone(),
                });
            }
            snapshot.plan = Some(plan.clone());
        }

        AgentEvent::PermissionRequest { request } => {
            snapshot.pending_permission = Some(request.clone());
        }

        AgentEvent::Artifact { artifact, .. } => {
            if let Some(existing) = snapshot
                .artifacts
                .iter_mut()
                .find(|a| same_artifact_identity(a, artifact))
            {
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

        AgentEvent::ModeChanged { .. } | AgentEvent::Trace { .. } => {}

        AgentEvent::FanOut { parent, agent, .. } => {
            // Prefer the map tool call's title as the subtitle once it's known.
            let title = snapshot
                .tool_calls
                .get(parent)
                .map(|c| c.title.clone())
                .unwrap_or_default();
            let fo = snapshot.fan_out.get_or_insert_with(FanOut::default);
            if title.len() > fo.title.len() {
                fo.title = title;
            }
            match fo.agents.iter_mut().find(|a| a.id == agent.id) {
                Some(existing) => {
                    existing.status = agent.status;
                    if !agent.label.is_empty() {
                        existing.label = agent.label.clone();
                    }
                }
                None => fo.agents.push(agent.clone()),
            }
            fo.total = fo.agents.len();
            fo.done = fo
                .agents
                .iter()
                .filter(|a| a.status == FanOutStatus::Done)
                .count();
            fo.running = fo
                .agents
                .iter()
                .filter(|a| a.status == FanOutStatus::Running)
                .count();
        }

        AgentEvent::RunFinished { run, outcome } => {
            let view = snapshot.runs.entry(run.clone()).or_insert_with(|| RunView {
                id: run.clone(),
                status: outcome.status,
                outcome: None,
                checkpoint: None,
            });
            view.status = outcome.status;
            view.outcome = Some(outcome.clone());
            // A finished run clears any permission gate tied to it.
            snapshot.pending_permission = None;
            // The fan-out surface is a live-run affordance; retire it when the
            // run ends so it fades out rather than lingering.
            snapshot.fan_out = None;
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

/// Append `delta` to `blocks`, concatenating adjacent same-kind text/thinking
/// blocks for smooth streaming.
fn merge_block(blocks: &mut Vec<ContentBlock>, delta: &ContentBlock) {
    match (blocks.last_mut(), delta) {
        (Some(ContentBlock::Text { text: last }), ContentBlock::Text { text: add }) => {
            last.push_str(add);
        }
        (Some(ContentBlock::Thinking { text: last }), ContentBlock::Thinking { text: add }) => {
            last.push_str(add);
        }
        _ => blocks.push(delta.clone()),
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
    fn thinking_chunks_coalesce_and_keep_order_with_text() {
        let events = vec![
            AgentEvent::RunStarted { run: run() },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::Agent,
                delta: ContentBlock::text("Answer: "),
            },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::Agent,
                delta: ContentBlock::thinking("Think"),
            },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::Agent,
                delta: ContentBlock::thinking("ing…"),
            },
            AgentEvent::MessageChunk {
                run: run(),
                role: Role::Agent,
                delta: ContentBlock::text("done"),
            },
        ];
        let snap = reduce_all(&events);
        assert_eq!(snap.timeline.len(), 1);
        match &snap.timeline[0] {
            TimelineItem::Message { blocks, .. } => {
                assert_eq!(
                    blocks,
                    &vec![
                        ContentBlock::text("Answer: "),
                        ContentBlock::thinking("Thinking…"),
                        ContentBlock::text("done"),
                    ]
                );
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
                    tool_name: None,
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
    fn replace_content_supersedes_streamed_partials() {
        let id = ToolCallId::new("t1");
        let events = vec![
            AgentEvent::ToolCall {
                run: run(),
                call: ToolCall {
                    tool_name: None,
                    id: id.clone(),
                    title: "bash: make build".into(),
                    kind: ToolKind::Execute,
                    status: ToolStatus::Pending,
                    locations: vec![],
                    content: vec![],
                    raw_input: None,
                },
            },
            // Live output streamed while the command runs…
            AgentEvent::ToolCallUpdate {
                run: run(),
                id: id.clone(),
                patch: ToolCallPatch {
                    status: Some(ToolStatus::InProgress),
                    append_content: vec![ContentBlock::text("compiling…\n")],
                    ..Default::default()
                },
            },
            // …then the final result replaces the partials wholesale.
            AgentEvent::ToolCallUpdate {
                run: run(),
                id: id.clone(),
                patch: ToolCallPatch {
                    status: Some(ToolStatus::Completed),
                    replace_content: Some(vec![ContentBlock::text("exit_code: 0")]),
                    ..Default::default()
                },
            },
        ];
        let snap = reduce_all(&events);
        let tc = &snap.tool_calls[&id];
        assert_eq!(tc.status, ToolStatus::Completed);
        assert_eq!(tc.content, vec![ContentBlock::text("exit_code: 0")]);
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
                    detail: None,
                    risk: None,
                    reason: None,
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
                    failure_kind: None,
                    usage: None,
                    execution: None,
                },
            },
        );
        assert!(snap.pending_permission.is_none());
        assert_eq!(snap.runs[&run()].status, RunStatus::Done);
    }

    #[test]
    fn permission_clears_when_its_gated_tool_proceeds() {
        let tc = ToolCallId::new("tc-1");
        let mut snap = reduce_all(&[
            AgentEvent::ToolCall {
                run: run(),
                call: ToolCall {
                    tool_name: None,
                    id: tc.clone(),
                    title: "bash".into(),
                    kind: ToolKind::Execute,
                    status: ToolStatus::Pending,
                    locations: vec![],
                    content: vec![],
                    raw_input: None,
                },
            },
            AgentEvent::PermissionRequest {
                request: PermissionRequest {
                    id: crate::ids::PermissionRequestId::new("perm-tc-1"),
                    session: SessionId::new("s"),
                    tool_call: Some(tc.clone()),
                    title: "Allow?".into(),
                    options: vec![],
                    detail: None,
                    risk: None,
                    reason: None,
                },
            },
        ]);
        assert!(snap.pending_permission.is_some());
        // Approving the tool makes it proceed (InProgress) → the gate clears.
        apply(
            &mut snap,
            &AgentEvent::ToolCallUpdate {
                run: run(),
                id: tc,
                patch: ToolCallPatch {
                    status: Some(ToolStatus::InProgress),
                    ..Default::default()
                },
            },
        );
        assert!(snap.pending_permission.is_none());
    }

    #[test]
    fn plan_pushes_one_timeline_marker_per_run_and_updates_in_place() {
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
                .filter(|i| matches!(i, TimelineItem::Plan { .. }))
                .count(),
            1
        );
        match &snap.timeline[0] {
            TimelineItem::Plan {
                run: plan_run,
                plan,
            } => {
                assert_eq!(plan_run.as_ref(), Some(&run()));
                assert_eq!(plan.phases[0].status, PlanPhaseStatus::Completed);
            }
            other => panic!("expected plan, got {other:?}"),
        }
        assert_eq!(
            snap.plan.unwrap().phases[0].status,
            PlanPhaseStatus::Completed
        );

        let run_two = RunId::new("run-2");
        let snap = reduce_all(&[
            AgentEvent::Plan {
                run: run(),
                plan: Plan {
                    phases: vec![PlanPhase {
                        title: "first".into(),
                        status: PlanPhaseStatus::Completed,
                        priority: None,
                    }],
                },
            },
            AgentEvent::Plan {
                run: run_two,
                plan: Plan {
                    phases: vec![PlanPhase {
                        title: "second".into(),
                        status: PlanPhaseStatus::InProgress,
                        priority: None,
                    }],
                },
            },
        ]);
        assert_eq!(
            snap.timeline
                .iter()
                .filter(|i| matches!(i, TimelineItem::Plan { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn artifact_with_same_uri_updates_in_place_without_duplicate_timeline_entry() {
        let first = AgentEvent::Artifact {
            run: run(),
            artifact: Artifact {
                id: "artifact-path".into(),
                title: "Draft report".into(),
                kind: ArtifactKind::File,
                mime_type: None,
                uri: Some("http://localhost:8787/api/artifacts/conv-1/report.pdf".into()),
                tool_call: None,
            },
        };
        let second = AgentEvent::Artifact {
            run: run(),
            artifact: Artifact {
                id: "artifact-url".into(),
                title: "Final report".into(),
                kind: ArtifactKind::Pdf,
                mime_type: Some("application/pdf".into()),
                uri: Some("http://localhost:8787/api/artifacts/conv-1/report.pdf".into()),
                tool_call: None,
            },
        };

        let snap = reduce_all(&[first, second]);
        assert_eq!(snap.artifacts.len(), 1);
        assert_eq!(snap.artifacts[0].id, "artifact-url");
        assert_eq!(snap.artifacts[0].title, "Final report");
        assert_eq!(
            snap.timeline
                .iter()
                .filter(|i| matches!(i, TimelineItem::Artifact { .. }))
                .count(),
            1
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

    #[test]
    fn legacy_plan_timeline_item_deserializes() {
        let item: TimelineItem = serde_json::from_value(serde_json::json!({
            "item": "plan"
        }))
        .unwrap();
        match item {
            TimelineItem::Plan { run, plan } => {
                assert!(run.is_none());
                assert!(plan.phases.is_empty());
            }
            other => panic!("expected legacy plan item, got {other:?}"),
        }
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
                    tool_name: None,
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
                    detail: None,
                    risk: None,
                    reason: None,
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
                    failure_kind: None,
                    usage: None,
                    execution: None,
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
