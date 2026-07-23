//! Event projection: pure, idempotent reduction of an [`AgentEvent`] stream into
//! a [`Snapshot`] the UI renders. This is the single source of truth that the
//! old web/mobile apps re-implemented separately in TypeScript — here it lives
//! once and ships to native and WASM.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::provider::ResumeTranscript;

use crate::domain::*;
use crate::ids::{RunId, SessionId, ToolCallId};

/// A run as the UI sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunView {
    pub id: RunId,
    pub status: RunStatus,
    /// Latest cumulative usage, available while the run is still active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunUsage>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<MessagePhase>,
    },
    /// Reference into [`Snapshot::tool_calls`] (kept by id so updates are O(1)).
    ToolCall {
        id: ToolCallId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<RunId>,
    },
    /// Reference into [`Snapshot::artifacts`] — rendered inline where produced.
    Artifact {
        id: String,
    },
    ExecutionChecklist {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<RunId>,
        #[serde(default)]
        checklist: ExecutionChecklist,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        explanation: Option<String>,
    },
    ProposedPlan {
        run: RunId,
        plan: ProposedPlan,
    },
    ProviderIncident {
        run: RunId,
        id: String,
    },
}

/// Everything the UI renders for a session. Pushed to the frontend (whole or
/// diffed) after each applied event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Device-local SQLite outbox cursor covered by this projection. The Tauri
    /// bridge strips it before cloud persistence; it exists only to checkpoint
    /// the exact local event prefix represented by a frontend snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_checkpoint: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    pub runs: IndexMap<RunId, RunView>,
    pub timeline: Vec<TimelineItem>,
    /// Provider history replacement captured at `timeline_index`. Reopening
    /// replays it followed by visible timeline items appended after that
    /// boundary, preserving compaction without hiding the original chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_context_checkpoint: Option<ModelContextCheckpoint>,
    pub tool_calls: IndexMap<ToolCallId, ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_checklist: Option<ExecutionChecklist>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_plan: Option<ProposedPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<GoalState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_permission: Option<PermissionRequest>,
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<WorkspaceFocus>,
    /// Live parallel fan-out (a `subagent_map` spread across child agents), or
    /// `None` when nothing is fanning out. Rendered by the fan-out surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_out: Option<FanOut>,
    /// Durable provider-incident diagnostics, referenced by timeline identity
    /// so later status updates replace the same card in place.
    #[serde(default)]
    pub provider_incidents: IndexMap<String, crate::recovery::ProviderIncident>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelContextCheckpoint {
    pub transcript: ResumeTranscript,
    pub timeline_index: usize,
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
            // Retire the previous turn's parallel-work receipt only when new
            // work begins. A completed card remains visible long enough for
            // the user to understand what Clark finished.
            snapshot.fan_out = None;
            snapshot.runs.entry(run.clone()).or_insert_with(|| RunView {
                id: run.clone(),
                status: RunStatus::Running,
                usage: None,
                outcome: None,
                checkpoint: None,
            });
        }

        AgentEvent::Checkpoint { run, id } => {
            let view = snapshot.runs.entry(run.clone()).or_insert_with(|| RunView {
                id: run.clone(),
                status: RunStatus::Running,
                usage: None,
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
                ..
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
                phase: None,
            });
        }

        AgentEvent::MessagePhase { run, phase } => {
            set_latest_unphased_agent_message(&mut snapshot.timeline, run, *phase);
        }

        AgentEvent::ToolCall { run, call } => {
            // Compatibility fallback for providers that stream assistant text
            // but do not expose an explicit message phase. Once a tool follows,
            // the preceding assistant text is necessarily mid-turn commentary.
            set_latest_unphased_agent_message(
                &mut snapshot.timeline,
                run,
                MessagePhase::Commentary,
            );
            let already = snapshot.tool_calls.contains_key(&call.id);
            snapshot.tool_calls.insert(call.id.clone(), call.clone());
            if !already {
                snapshot.timeline.push(TimelineItem::ToolCall {
                    id: call.id.clone(),
                    run: Some(run.clone()),
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
                if let Some(progress) = &patch.progress {
                    tc.progress = Some(progress.clone());
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

        AgentEvent::ExecutionChecklistUpdated {
            run,
            checklist,
            explanation,
        } => {
            if let Some(TimelineItem::ExecutionChecklist {
                checklist: existing_checklist,
                explanation: existing_explanation,
                ..
            }) = snapshot.timeline.iter_mut().rev().find(|item| {
                matches!(item, TimelineItem::ExecutionChecklist { run: Some(checklist_run), .. } if checklist_run == run)
            }) {
                *existing_checklist = checklist.clone();
                *existing_explanation = explanation.clone();
            } else {
                snapshot.timeline.push(TimelineItem::ExecutionChecklist {
                    run: Some(run.clone()),
                    checklist: checklist.clone(),
                    explanation: explanation.clone(),
                });
            }
            snapshot.execution_checklist = Some(checklist.clone());
        }

        AgentEvent::ProposedPlanUpdated { run, plan } => {
            if let Some(TimelineItem::ProposedPlan {
                plan: existing_plan,
                ..
            }) = snapshot.timeline.iter_mut().rev().find(|item| {
                matches!(item, TimelineItem::ProposedPlan { plan: existing, .. } if existing.id == plan.id)
            }) {
                *existing_plan = plan.clone();
            } else {
                snapshot.timeline.push(TimelineItem::ProposedPlan {
                    run: run.clone(),
                    plan: plan.clone(),
                });
            }
            snapshot.proposed_plan = Some(plan.clone());
        }

        AgentEvent::GoalUpdated { run, goal } => {
            let mut next = goal.clone();
            next.run = Some(run.clone());
            snapshot.goal = Some(next);
        }

        AgentEvent::RunUsageUpdated { run, usage } => {
            let view = snapshot.runs.entry(run.clone()).or_insert_with(|| RunView {
                id: run.clone(),
                status: RunStatus::Running,
                usage: None,
                outcome: None,
                checkpoint: None,
            });
            view.usage = Some(*usage);
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

        AgentEvent::ProviderIncidentUpdated { run, incident } => {
            let is_new = !snapshot.provider_incidents.contains_key(&incident.id);
            snapshot
                .provider_incidents
                .insert(incident.id.clone(), incident.clone());
            if is_new {
                snapshot.timeline.push(TimelineItem::ProviderIncident {
                    run: run.clone(),
                    id: incident.id.clone(),
                });
            }
        }

        AgentEvent::ContextCompacted { transcript, .. } => {
            snapshot.model_context_checkpoint = Some(ModelContextCheckpoint {
                transcript: transcript.clone(),
                timeline_index: snapshot.timeline.len(),
            });
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
                    if !agent.label.is_empty()
                        && (existing.label.is_empty() || existing.label.starts_with("Task "))
                    {
                        existing.label = agent.label.clone();
                    }
                    if agent.objective.is_some() {
                        existing.objective = agent.objective.clone();
                    }
                    if agent.activity.is_some() {
                        existing.activity = agent.activity.clone();
                    }
                    if agent.result.is_some() {
                        existing.result = agent.result.clone();
                    }
                    if agent.attempt.is_some() {
                        existing.attempt = agent.attempt;
                    }
                    if existing.started_at_ms.is_none() && agent.started_at_ms.is_some() {
                        existing.started_at_ms = agent.started_at_ms;
                    }
                    if agent.updated_at_ms.is_some() {
                        existing.updated_at_ms = agent.updated_at_ms;
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
            // A provider without native phase metadata leaves its terminal
            // assistant message unresolved until the run boundary. Explicit
            // commentary is never overwritten here.
            set_latest_unphased_agent_message(
                &mut snapshot.timeline,
                run,
                MessagePhase::FinalAnswer,
            );
            let view = snapshot.runs.entry(run.clone()).or_insert_with(|| RunView {
                id: run.clone(),
                status: outcome.status,
                usage: None,
                outcome: None,
                checkpoint: None,
            });
            view.status = outcome.status;
            if outcome.usage.is_some() {
                view.usage = outcome.usage;
            }
            view.outcome = Some(outcome.clone());
            settle_run_tool_calls(snapshot, run, outcome.status);
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

/// A terminal run must never leave tool rows looking live. Providers normally
/// emit a terminal `ToolCallUpdate`; this closes the gap when a batch is
/// cancelled or a provider terminates without one.
fn settle_run_tool_calls(snapshot: &mut Snapshot, run: &RunId, status: RunStatus) {
    let settled = match status {
        RunStatus::Done => ToolStatus::Completed,
        RunStatus::Cancelled => ToolStatus::Cancelled,
        RunStatus::Failed => ToolStatus::Failed,
        RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingInput => return,
    };
    let ids = snapshot
        .timeline
        .iter()
        .filter_map(|item| match item {
            TimelineItem::ToolCall {
                id,
                run: Some(tool_run),
            } if tool_run == run => Some(id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for id in ids {
        if let Some(call) = snapshot.tool_calls.get_mut(&id) {
            if matches!(call.status, ToolStatus::Pending | ToolStatus::InProgress) {
                call.status = settled;
            }
        }
    }
}

fn set_latest_unphased_agent_message(
    timeline: &mut [TimelineItem],
    run: &RunId,
    phase: MessagePhase,
) {
    if let Some(TimelineItem::Message {
        role: Role::Agent,
        phase: message_phase,
        ..
    }) = timeline.iter_mut().rev().find(|item| {
        matches!(
            item,
            TimelineItem::Message {
                run: message_run,
                role: Role::Agent,
                ..
            } if message_run == run
        )
    }) {
        if message_phase.is_none() {
            *message_phase = Some(phase);
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
#[path = "projection_tests.rs"]
mod tests;
