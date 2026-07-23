use super::*;

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
fn execution_checklist_timeline_item_deserializes() {
    let item: TimelineItem = serde_json::from_value(serde_json::json!({
        "item": "execution_checklist"
    }))
    .unwrap();
    match item {
        TimelineItem::ExecutionChecklist { run, checklist, .. } => {
            assert!(run.is_none());
            assert!(checklist.steps.is_empty());
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
        AgentEvent::MessagePhase {
            run: run(),
            phase: MessagePhase::Commentary,
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
                progress: None,
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
        AgentEvent::ExecutionChecklistUpdated {
            run: run(),
            checklist: ExecutionChecklist::default(),
            explanation: None,
        },
        AgentEvent::ProposedPlanUpdated {
            run: run(),
            plan: ProposedPlan {
                id: "plan-1".into(),
                revision: 1,
                markdown: "# Plan".into(),
                status: ProposedPlanStatus::AwaitingDecision,
            },
        },
        AgentEvent::RunUsageUpdated {
            run: run(),
            usage: RunUsage {
                input_tokens: 100,
                output_tokens: 20,
                context_tokens: 100,
                cost_usd: Some(0.01),
                context_limit: Some(1_000),
            },
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
        AgentEvent::ContextCompacted {
            run: run(),
            transcript: ResumeTranscript {
                items: vec![crate::provider::ResumeItem::Message {
                    role: Role::User,
                    blocks: vec![ContentBlock::text("summary")],
                }],
                truncated: false,
            },
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
    assert_eq!(once.model_context_checkpoint.unwrap().timeline_index, 5);
    assert_eq!(once.focus.unwrap().surface, WorkspaceSurfaceKind::Browser);
    assert_eq!(once.runs[&run()].status, RunStatus::Done);
    assert!(once.pending_permission.is_none());
}

#[test]
fn fan_out_updates_preserve_labels_and_recompute_aggregate_state() {
    let parent = ToolCallId::new("delegate");
    let mut snapshot = Snapshot::new();
    apply(
        &mut snapshot,
        &AgentEvent::FanOut {
            run: run(),
            parent: parent.clone(),
            agent: FanOutAgent {
                id: "implementation".into(),
                label: "Implement the change".into(),
                status: FanOutStatus::Queued,
                objective: Some("Implement the change without touching unrelated files".into()),
                activity: Some("Waiting to start".into()),
                result: None,
                attempt: None,
                started_at_ms: None,
                updated_at_ms: Some(10),
            },
        },
    );
    apply(
        &mut snapshot,
        &AgentEvent::FanOut {
            run: run(),
            parent,
            agent: FanOutAgent {
                id: "implementation".into(),
                label: String::new(),
                status: FanOutStatus::Running,
                objective: None,
                activity: Some("Reading the implementation".into()),
                result: None,
                attempt: Some(1),
                started_at_ms: Some(20),
                updated_at_ms: Some(30),
            },
        },
    );

    let fan_out = snapshot.fan_out.unwrap();
    assert_eq!(fan_out.total, 1);
    assert_eq!(fan_out.running, 1);
    assert_eq!(fan_out.done, 0);
    assert_eq!(fan_out.agents[0].label, "Implement the change");
    assert_eq!(
        fan_out.agents[0].objective.as_deref(),
        Some("Implement the change without touching unrelated files")
    );
    assert_eq!(
        fan_out.agents[0].activity.as_deref(),
        Some("Reading the implementation")
    );
    assert_eq!(fan_out.agents[0].started_at_ms, Some(20));
}

#[test]
fn completed_fan_out_stays_visible_until_the_next_run_starts() {
    let mut snapshot = Snapshot::new();
    apply(
        &mut snapshot,
        &AgentEvent::FanOut {
            run: run(),
            parent: ToolCallId::new("delegate"),
            agent: FanOutAgent {
                id: "review".into(),
                label: "Review the result".into(),
                status: FanOutStatus::Done,
                objective: Some("Review the result".into()),
                activity: Some("Review complete".into()),
                result: Some("No issues found".into()),
                attempt: Some(1),
                started_at_ms: Some(10),
                updated_at_ms: Some(20),
            },
        },
    );
    apply(
        &mut snapshot,
        &AgentEvent::RunFinished {
            run: run(),
            outcome: RunOutcome {
                status: RunStatus::Done,
                stop_reason: None,
                error: None,
                failure_kind: None,
                usage: None,
                execution: None,
            },
        },
    );
    assert!(snapshot.fan_out.is_some());

    apply(
        &mut snapshot,
        &AgentEvent::RunStarted {
            run: RunId::new("run-2"),
        },
    );
    assert!(snapshot.fan_out.is_none());
}
