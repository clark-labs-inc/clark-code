use super::*;
use crate::domain::{
    ContentBlock, GoalState, GoalStatus, ProposedPlan, ProposedPlanStatus, Role, RunOutcome,
    RunStatus, RunUsage, ToolCallPatch, ToolKind, ToolStatus,
};
use crate::projection::apply;

fn run(index: usize) -> RunId {
    RunId::new(format!("run-{index}"))
}

fn tool(id: &str, status: ToolStatus) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id),
        tool_name: Some("read_file".into()),
        title: format!("Read {id}"),
        kind: ToolKind::Read,
        status,
        locations: vec![],
        content: vec![],
        raw_input: None,
        streamed_input: String::new(),
        progress: None,
    }
}

/// One realistic turn: start, stream some prose, run a tool, settle it, answer.
fn turn(index: usize) -> Vec<AgentEvent> {
    let id = format!("call-{index}");
    vec![
        AgentEvent::RunStarted { run: run(index) },
        AgentEvent::MessageChunk {
            run: run(index),
            role: Role::Agent,
            delta: ContentBlock::text(format!("working on step {index}: ")),
        },
        AgentEvent::MessageChunk {
            run: run(index),
            role: Role::Agent,
            delta: ContentBlock::text("reading the file"),
        },
        AgentEvent::ToolCall {
            run: run(index),
            call: tool(&id, ToolStatus::Pending),
        },
        AgentEvent::ToolCallUpdate {
            run: run(index),
            id: ToolCallId::new(&id),
            patch: ToolCallPatch {
                status: Some(ToolStatus::Completed),
                append_content: vec![ContentBlock::text("file contents here")],
                ..Default::default()
            },
        },
        AgentEvent::MessageChunk {
            run: run(index),
            role: Role::Agent,
            delta: ContentBlock::text(format!("done with {index}")),
        },
        AgentEvent::RunUsageUpdated {
            run: run(index),
            usage: RunUsage {
                input_tokens: 100,
                output_tokens: 20,
                context_tokens: 120,
                cost_usd: None,
                context_limit: None,
            },
        },
        AgentEvent::RunFinished {
            run: run(index),
            outcome: RunOutcome {
                status: RunStatus::Done,
                stop_reason: None,
                error: None,
                failure_kind: None,
                usage: None,
                execution: None,
            },
        },
    ]
}

/// Apply a batch to a clone, diff it, replay the delta onto the original, and
/// assert the two projections are identical. This is the property the whole
/// protocol rests on: a consumer that folds deltas must never diverge from one
/// that folds events.
fn assert_delta_reproduces(base: &Snapshot, batch: &[AgentEvent]) -> Snapshot {
    let mut projected = base.clone();
    for event in batch {
        apply(&mut projected, event);
    }
    let delta = diff_after_apply(base, &projected, batch, 1, 0)
        .expect("an apply-driven transition is expressible");

    let mut replayed = base.clone();
    apply_delta(&mut replayed, &delta, 0).expect("delta applies to its own base");
    assert_eq!(
        replayed, projected,
        "delta replay diverged from event replay"
    );
    projected
}

#[test]
fn a_delta_reproduces_the_projection_across_many_turns() {
    let mut snapshot = Snapshot::new();
    // Deltas must stay exact as the transcript grows, which is the case the
    // whole-snapshot publication path handles badly.
    for index in 0..25 {
        snapshot = assert_delta_reproduces(&snapshot, &turn(index));
    }
    assert_eq!(snapshot.tool_calls.len(), 25);
    assert_eq!(snapshot.runs.len(), 25);
}

#[test]
fn a_delta_stays_bounded_while_the_transcript_grows() {
    // The point of the protocol: publication size tracks new content, not
    // conversation length. Compare a delta for the same batch early and late.
    let mut snapshot = Snapshot::new();
    let early_base = snapshot.clone();
    let early = {
        let mut projected = early_base.clone();
        let batch = turn(0);
        for event in &batch {
            apply(&mut projected, event);
        }
        diff_after_apply(&early_base, &projected, &batch, 1, 0).expect("early delta")
    };

    for index in 0..40 {
        snapshot = assert_delta_reproduces(&snapshot, &turn(index));
    }

    let late_base = snapshot.clone();
    let late = {
        let mut projected = late_base.clone();
        let batch = turn(99);
        for event in &batch {
            apply(&mut projected, event);
        }
        diff_after_apply(&late_base, &projected, &batch, 2, 1).expect("late delta")
    };

    // Same work, same-size delta — while the full snapshot has grown far past
    // it. Sizes are compared as serialized bytes, which is what the wire pays.
    let bytes = |value: &SnapshotDelta| serde_json::to_vec(value).unwrap().len();
    let full_bytes = serde_json::to_vec(&snapshot).unwrap().len();
    let late_bytes = bytes(&late);
    assert!(
        late_bytes < early_bytes_ceiling(bytes(&early)),
        "late delta {late_bytes} grew against early {}",
        bytes(&early)
    );
    assert!(
        late_bytes * 4 < full_bytes,
        "a delta ({late_bytes}) should be far smaller than the snapshot ({full_bytes})"
    );
    if std::env::var("CLARK_DELTA_SIZES").is_ok() {
        println!(
            "delta bytes: early={} late={}  full snapshot bytes={}  ratio={:.1}x",
            bytes(&early),
            late_bytes,
            full_bytes,
            full_bytes as f64 / late_bytes as f64
        );
    }
}

/// Allow a little slack for run/tool ids getting longer, without allowing the
/// delta to scale with the transcript.
fn early_bytes_ceiling(early: usize) -> usize {
    early + 512
}

#[test]
fn keys_are_complete_against_the_reducer() {
    // `touched_keys` is derived from events, never from scanning the maps. If
    // the reducer ever changes a map entry this function does not name, a
    // consumer would silently miss it — so verify the claim by brute force
    // against a full before/after comparison.
    let mut snapshot = Snapshot::new();
    for index in 0..3 {
        for event in turn(index) {
            let before = snapshot.clone();
            apply(&mut snapshot, &event);
            let claimed = touched_keys(std::slice::from_ref(&event), &snapshot);

            for (id, call) in &snapshot.tool_calls {
                let changed = before.tool_calls.get(id) != Some(call);
                if changed {
                    assert!(
                        claimed.tool_calls.contains(id),
                        "{event:?} changed tool call {id:?} without naming it"
                    );
                }
            }
            for (id, view) in &snapshot.runs {
                if before.runs.get(id) != Some(view) {
                    assert!(
                        claimed.runs.contains(id),
                        "{event:?} changed run {id:?} without naming it"
                    );
                }
            }
            for (id, incident) in &snapshot.provider_incidents {
                if before.provider_incidents.get(id) != Some(incident) {
                    assert!(
                        claimed.provider_incidents.contains(id),
                        "{event:?} changed incident {id} without naming it"
                    );
                }
            }
        }
    }
}

#[test]
fn a_rewrite_of_an_earlier_item_is_still_reproduced() {
    // Plans and presentations are rewritten in place by id, which can reach
    // back past the trailing item — the case a naive "append only" delta would
    // get wrong.
    let mut snapshot = Snapshot::new();
    let plan = |markdown: &str| ProposedPlan {
        id: "plan-1".into(),
        revision: 1,
        markdown: markdown.into(),
        status: ProposedPlanStatus::AwaitingDecision,
        global_reminders: Vec::new(),
        execution_contract: Vec::new(),
        context_revisions: Vec::new(),
    };
    snapshot = assert_delta_reproduces(
        &snapshot,
        &[AgentEvent::ProposedPlanUpdated {
            run: run(0),
            plan: plan("first draft"),
        }],
    );
    // Push several items after the plan, then rewrite it.
    snapshot = assert_delta_reproduces(&snapshot, &turn(1));
    let snapshot = assert_delta_reproduces(
        &snapshot,
        &[AgentEvent::ProposedPlanUpdated {
            run: run(1),
            plan: plan("revised after review"),
        }],
    );
    assert_eq!(
        snapshot.proposed_plan.as_ref().unwrap().markdown,
        "revised after review"
    );
}

#[test]
fn a_structural_rewrite_is_not_expressible_as_a_delta() {
    // Sealing pages moves `timeline_offset`; a delta cannot describe that, and
    // saying so is what keeps the caller republishing in full instead of
    // silently desynchronising the consumer.
    let mut snapshot = Snapshot::new();
    for index in 0..3 {
        for event in turn(index) {
            apply(&mut snapshot, &event);
        }
    }
    let previous = snapshot.clone();
    let mut sealed = snapshot.clone();
    sealed.timeline_offset += 1;
    sealed.timeline.remove(0);

    assert!(
        diff_after_apply(&previous, &sealed, &[], 2, 1).is_none(),
        "an offset change must decline rather than emit a delta"
    );
}

#[test]
fn a_change_far_behind_unchanged_trailing_items_is_still_found() {
    // The regression this module was first written with: walking back from the
    // end stops at the first agreement, so an in-place rewrite sitting behind
    // identical trailing items was silently dropped from the delta. The scan
    // must look forward from the start of the shared region.
    let mut snapshot = Snapshot::new();
    for index in 0..20 {
        for event in turn(index) {
            apply(&mut snapshot, &event);
        }
    }
    let previous = snapshot.clone();
    let mut rewritten = snapshot.clone();
    let deep = 2;
    rewritten.timeline[deep] = TimelineItem::Message {
        run: run(0),
        role: Role::Agent,
        blocks: vec![ContentBlock::text("history rewritten")],
        phase: None,
    };

    let delta = diff_after_apply(&previous, &rewritten, &[], 2, 1)
        .expect("a deep in-place rewrite is expressible");
    assert_eq!(
        delta.timeline_from, deep,
        "the delta must start at the change"
    );

    let mut replayed = previous;
    apply_delta(&mut replayed, &delta, 1).expect("applies");
    assert_eq!(replayed, rewritten, "deep rewrite must replicate exactly");
}

#[test]
fn a_skipped_publication_is_refused_rather_than_guessed() {
    let mut snapshot = Snapshot::new();
    let batch = turn(0);
    let mut projected = snapshot.clone();
    for event in &batch {
        apply(&mut projected, event);
    }
    // seq 5 built on 4, but the consumer only holds 3: one publication was
    // lost, so the tail would land on the wrong base.
    let delta = diff_after_apply(&snapshot, &projected, &batch, 5, 4).expect("delta");
    assert_eq!(
        apply_delta(&mut snapshot, &delta, 3),
        Err(DeltaError::OutOfOrder { held: 3, base: 4 })
    );
    assert_eq!(snapshot, Snapshot::new(), "a refused delta changes nothing");
}

#[test]
fn a_tail_that_would_leave_a_hole_is_refused() {
    let mut snapshot = Snapshot::new();
    let delta = SnapshotDelta {
        seq: 1,
        base_seq: 0,
        timeline_from: 9,
        timeline_tail: vec![],
        tool_calls: IndexMap::new(),
        runs: IndexMap::new(),
        provider_incidents: IndexMap::new(),
        scalars: SnapshotScalars::default(),
    };
    assert_eq!(
        apply_delta(&mut snapshot, &delta, 0),
        Err(DeltaError::AfterEnd { from: 9, end: 0 })
    );

    let mut windowed = Snapshot::new();
    windowed.timeline_offset = 10;
    let behind = SnapshotDelta {
        timeline_from: 4,
        ..delta
    };
    assert_eq!(
        apply_delta(&mut windowed, &behind, 0),
        Err(DeltaError::BeforeWindow {
            from: 4,
            offset: 10
        })
    );
}

#[test]
fn scalars_replicate_a_transition_back_to_none() {
    // Clearing is why the bounded fields are carried whole: a sparse encoding
    // cannot distinguish "unchanged" from "now empty".
    let mut snapshot = Snapshot::new();
    snapshot = assert_delta_reproduces(
        &snapshot,
        &[AgentEvent::GoalUpdated {
            run: run(0),
            goal: GoalState {
                id: "goal-1".into(),
                objective: "ship it".into(),
                status: GoalStatus::Active,
                run: None,
                tokens_used: 0,
                time_used_seconds: 0,
                continuations: 0,
                updated_at_ms: 1,
                blocker_reason: None,
            },
        }],
    );
    assert!(snapshot.goal.is_some());

    let cleared = assert_delta_reproduces(&snapshot, &[AgentEvent::GoalCleared {}]);
    assert!(cleared.goal.is_none(), "clearing must replicate");
}
