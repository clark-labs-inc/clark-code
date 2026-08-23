//! Bounded description of how one [`Snapshot`] became the next.
//!
//! The host projects a session by folding [`AgentEvent`]s into a `Snapshot`,
//! then publishing the result. Publishing the *whole* snapshot costs
//! O(conversation) per publication — and the timeline and tool-call maps grow
//! monotonically for a session's life, so a long conversation pays more per
//! event than a short one for identical work.
//!
//! A delta carries only what an event batch could have changed:
//!
//! - **timeline**: a tail replacing everything from one absolute index onward,
//!   found by scanning for the first item that differs. The scan is O(items)
//!   but each comparison is trivial (string equality rejects on length), which
//!   is nothing beside the O(bytes) serialization and JavaScript-source parse a
//!   whole-snapshot publication pays. Note the changed region is *not* always a
//!   suffix: presentations, checklists and plans are rewritten in place by id
//!   and can sit arbitrarily far back, so a bounded-window scan would silently
//!   miss them.
//! - **maps** (`tool_calls`, `runs`, `provider_incidents`): only the entries
//!   whose keys the batch names. Those keys come from the events themselves
//!   ([`TouchedKeys`]), never from a scan, which is what keeps a delta cheap
//!   for a conversation with thousands of settled tool calls.
//! - **everything else**: carried whole. Every remaining field is bounded and
//!   small, and several must express a transition back to `None`, which a
//!   sparse encoding would have to special-case anyway.
//!
//! Two invariants make this sound, and both are pinned by tests below:
//!
//! 1. A delta describes an `apply`-driven transition only. Structural rewrites
//!    that shift `timeline_offset` — [`Snapshot::seal_transcript_pages`] — are
//!    not expressible, and [`diff_after_apply`] returns `None` for them so the
//!    caller republishes in full.
//! 2. `apply_delta(base, diff(base, next)) == next` for every `apply`-driven
//!    transition. The property test drives realistic turns through both paths
//!    and compares the projections; it is what caught the first version of
//!    this module assuming a suffix.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{RunView, Snapshot, TimelineItem};
use crate::domain::{
    Artifact, ExecutionChecklist, GoalState, PermissionRequest, ProposedPlan, ToolCall,
    WorkspaceFocus,
};
use crate::ids::{RunId, SessionId, ToolCallId};
use crate::recovery::ProviderIncident;
use crate::AgentEvent;

/// The bounded fields of a snapshot, carried whole by every delta.
///
/// Cheap to send and each one needs to express clearing, so a sparse encoding
/// would buy nothing. Keeping them in one struct means adding a snapshot field
/// is a compile error here rather than a silently un-replicated field.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotScalars {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_checkpoint: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    #[serde(default)]
    pub starting: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_context_checkpoint: Option<super::ModelContextCheckpoint>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_out: Option<crate::domain::FanOut>,
}

impl SnapshotScalars {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            history_checkpoint: snapshot.history_checkpoint,
            session: snapshot.session.clone(),
            starting: snapshot.starting,
            model_context_checkpoint: snapshot.model_context_checkpoint.clone(),
            execution_checklist: snapshot.execution_checklist.clone(),
            proposed_plan: snapshot.proposed_plan.clone(),
            goal: snapshot.goal.clone(),
            pending_permission: snapshot.pending_permission.clone(),
            artifacts: snapshot.artifacts.clone(),
            focus: snapshot.focus.clone(),
            fan_out: snapshot.fan_out.clone(),
        }
    }

    fn apply_to(&self, snapshot: &mut Snapshot) {
        snapshot.history_checkpoint = self.history_checkpoint;
        snapshot.session = self.session.clone();
        snapshot.starting = self.starting;
        snapshot.model_context_checkpoint = self.model_context_checkpoint.clone();
        snapshot.execution_checklist = self.execution_checklist.clone();
        snapshot.proposed_plan = self.proposed_plan.clone();
        snapshot.goal = self.goal.clone();
        snapshot.pending_permission = self.pending_permission.clone();
        snapshot.artifacts = self.artifacts.clone();
        snapshot.focus = self.focus.clone();
        snapshot.fan_out = self.fan_out.clone();
    }
}

/// One publication's worth of change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDelta {
    /// Monotonic publication counter. A consumer that receives a delta whose
    /// `base_seq` is not the sequence it currently holds has missed a
    /// publication and must ask for a full snapshot instead of guessing.
    pub seq: u64,
    pub base_seq: u64,
    /// Absolute transcript index (`timeline_offset` + local index) from which
    /// `timeline_tail` replaces the consumer's items.
    pub timeline_from: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline_tail: Vec<TimelineItem>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub tool_calls: IndexMap<ToolCallId, ToolCall>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub runs: IndexMap<RunId, RunView>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub provider_incidents: IndexMap<String, ProviderIncident>,
    pub scalars: SnapshotScalars,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeltaError {
    /// The consumer's sequence does not match the delta's base.
    OutOfOrder { held: u64, base: u64 },
    /// `timeline_from` precedes the consumer's transcript window, so applying
    /// the tail would leave a hole.
    BeforeWindow { from: usize, offset: usize },
    /// `timeline_from` is past the consumer's end, which would also leave a
    /// hole rather than a contiguous transcript.
    AfterEnd { from: usize, end: usize },
}

impl std::fmt::Display for DeltaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfOrder { held, base } => {
                write!(
                    formatter,
                    "delta expects base {base}, consumer holds {held}"
                )
            }
            Self::BeforeWindow { from, offset } => {
                write!(formatter, "delta starts at {from}, before window {offset}")
            }
            Self::AfterEnd { from, end } => {
                write!(formatter, "delta starts at {from}, past end {end}")
            }
        }
    }
}

/// Map keys an event batch could have changed.
///
/// Derived from the events, never by scanning the maps — scanning is exactly
/// the O(conversation) cost a delta exists to avoid. `keys_are_complete` in the
/// tests below is what keeps this honest against the reducer.
#[derive(Debug, Default, PartialEq)]
pub struct TouchedKeys {
    pub tool_calls: Vec<ToolCallId>,
    pub runs: Vec<RunId>,
    pub provider_incidents: Vec<String>,
}

/// Which map keys `events` can touch, given the projection they produced.
///
/// `projected` is only read to expand `RunFinished`, which settles every tool
/// call belonging to the finished run; those ids live in the timeline.
pub fn touched_keys(events: &[AgentEvent], projected: &Snapshot) -> TouchedKeys {
    let mut touched = TouchedKeys::default();
    for event in events {
        match event {
            AgentEvent::ToolCall { run, call } => {
                touched.tool_calls.push(call.id.clone());
                touched.runs.push(run.clone());
            }
            AgentEvent::ToolCallUpdate { id, .. } => touched.tool_calls.push(id.clone()),
            AgentEvent::RunFinished { run, .. } => {
                touched.runs.push(run.clone());
                // Settling a run rewrites the status of every tool call it
                // owns; the timeline is where that ownership is recorded.
                touched
                    .tool_calls
                    .extend(projected.timeline.iter().filter_map(|item| match item {
                        TimelineItem::ToolCall {
                            id,
                            run: Some(owner),
                        } if owner == run => Some(id.clone()),
                        _ => None,
                    }));
            }
            AgentEvent::RunStarted { run } => touched.runs.push(run.clone()),
            AgentEvent::Checkpoint { run, .. } => touched.runs.push(run.clone()),
            AgentEvent::RunUsageUpdated { run, .. } => touched.runs.push(run.clone()),
            // Error carries an optional run: a transport failure before the
            // provider allocated one has nothing to attribute.
            AgentEvent::Error { run, .. } => touched.runs.extend(run.clone()),
            AgentEvent::ProviderIncidentUpdated { incident, .. } => {
                touched.provider_incidents.push(incident.id.clone());
            }
            _ => {}
        }
    }
    touched.tool_calls.sort();
    touched.tool_calls.dedup();
    touched.runs.sort();
    touched.runs.dedup();
    touched.provider_incidents.sort();
    touched.provider_incidents.dedup();
    touched
}

/// Describe the transition from `previous` to `projected` under `events`.
///
/// Returns `None` when the transition is not expressible as a delta — a
/// structural rewrite that moved `timeline_offset`, a shrinking timeline, or a
/// change deeper than [`TIMELINE_DIFF_WINDOW`]. The caller republishes in full
/// in that case, which is always correct and merely more expensive.
///
/// `previous` must be the complete projection the consumer holds: the scan for
/// the first changed item has to be able to see every item, because an in-place
/// rewrite by id can land anywhere in the transcript.
pub fn diff_after_apply(
    previous: &Snapshot,
    projected: &Snapshot,
    events: &[AgentEvent],
    seq: u64,
    base_seq: u64,
) -> Option<SnapshotDelta> {
    if previous.timeline_offset != projected.timeline_offset {
        return None;
    }
    if projected.timeline.len() < previous.timeline.len() {
        return None;
    }

    // First index whose item changed, or the end of the shared region when the
    // batch only appended. Scanning forward (rather than walking back from the
    // end) is what makes an in-place rewrite behind unchanged trailing items
    // visible — the case that breaks any suffix-shaped assumption.
    let shared = previous.timeline.len();
    let local_from = (0..shared)
        .find(|index| previous.timeline[*index] != projected.timeline[*index])
        .unwrap_or(shared);

    let touched = touched_keys(events, projected);
    let pick = |keys: &[ToolCallId]| {
        keys.iter()
            .filter_map(|key| {
                projected
                    .tool_calls
                    .get(key)
                    .map(|value| (key.clone(), value.clone()))
            })
            .collect::<IndexMap<_, _>>()
    };

    Some(SnapshotDelta {
        seq,
        base_seq,
        timeline_from: projected.timeline_offset + local_from,
        timeline_tail: projected.timeline[local_from..].to_vec(),
        tool_calls: pick(&touched.tool_calls),
        runs: touched
            .runs
            .iter()
            .filter_map(|key| {
                projected
                    .runs
                    .get(key)
                    .map(|value| (key.clone(), value.clone()))
            })
            .collect(),
        provider_incidents: touched
            .provider_incidents
            .iter()
            .filter_map(|key| {
                projected
                    .provider_incidents
                    .get(key)
                    .map(|value| (key.clone(), value.clone()))
            })
            .collect(),
        scalars: SnapshotScalars::from_snapshot(projected),
    })
}

/// Fold a delta into a consumer's snapshot.
///
/// `held_seq` is the sequence the consumer currently holds; a mismatch is
/// refused rather than guessed, because a silently-skipped publication is
/// exactly the divergence this protocol has to make impossible.
pub fn apply_delta(
    snapshot: &mut Snapshot,
    delta: &SnapshotDelta,
    held_seq: u64,
) -> Result<(), DeltaError> {
    if held_seq != delta.base_seq {
        return Err(DeltaError::OutOfOrder {
            held: held_seq,
            base: delta.base_seq,
        });
    }
    let offset = snapshot.timeline_offset;
    if delta.timeline_from < offset {
        return Err(DeltaError::BeforeWindow {
            from: delta.timeline_from,
            offset,
        });
    }
    let local_from = delta.timeline_from - offset;
    if local_from > snapshot.timeline.len() {
        return Err(DeltaError::AfterEnd {
            from: delta.timeline_from,
            end: offset + snapshot.timeline.len(),
        });
    }

    snapshot.timeline.truncate(local_from);
    snapshot
        .timeline
        .extend(delta.timeline_tail.iter().cloned());
    for (id, call) in &delta.tool_calls {
        snapshot.tool_calls.insert(id.clone(), call.clone());
    }
    for (id, run) in &delta.runs {
        snapshot.runs.insert(id.clone(), run.clone());
    }
    for (id, incident) in &delta.provider_incidents {
        snapshot
            .provider_incidents
            .insert(id.clone(), incident.clone());
    }
    delta.scalars.apply_to(snapshot);
    Ok(())
}

#[cfg(test)]
mod tests;
