use super::*;

fn run() -> RunId {
    RunId::new("run-1")
}

#[test]
fn usage_updates_are_visible_before_the_run_finishes() {
    let mut snapshot = Snapshot::new();
    apply(&mut snapshot, &AgentEvent::RunStarted { run: run() });
    apply(
        &mut snapshot,
        &AgentEvent::RunUsageUpdated {
            run: run(),
            usage: RunUsage {
                input_tokens: 1_000,
                output_tokens: 100,
                context_tokens: 1_000,
                cost_usd: Some(0.01),
                context_limit: Some(10_000),
            },
        },
    );

    let view = &snapshot.runs[&run()];
    assert_eq!(view.status, RunStatus::Running);
    assert_eq!(
        view.usage,
        Some(RunUsage {
            input_tokens: 1_000,
            output_tokens: 100,
            context_tokens: 1_000,
            cost_usd: Some(0.01),
            context_limit: Some(10_000),
        })
    );
    assert!(view.outcome.is_none());
}

#[test]
fn run_started_clears_the_transient_prompt_starting_state() {
    // The host sets `starting` directly on the snapshot before it awaits the
    // provider (attachment upload / connect handshake in flight). The reducer
    // owns the other half of the contract: once a real run is allocated,
    // `RunStarted` retires the transient flag so the working row transitions
    // into normal run-driven activity.
    let mut snapshot = Snapshot::new();
    snapshot.starting = true;
    apply(&mut snapshot, &AgentEvent::RunStarted { run: run() });
    assert!(
        !snapshot.starting,
        "RunStarted retires the host-set starting state"
    );
}

fn incident(status: crate::recovery::ProviderIncidentStatus) -> crate::recovery::ProviderIncident {
    crate::recovery::ProviderIncident {
        id: "run-1:provider-incident:1".into(),
        status,
        scope: crate::recovery::ProviderIncidentScope::ModelRequest,
        failure_class: crate::recovery::ProviderFailureClass::TransientTransport,
        category: crate::recovery::ProviderIncidentCategory::Timeout,
        message: "Model connection timed out while the agent was working.".into(),
        detail: "gateway timeout".into(),
        model: "test-model".into(),
        provider_route: "gateway.test".into(),
        provider_status: Some(524),
        provider_error_type: Some("upstream_timeout".into()),
        request: crate::recovery::ProviderRequestDiagnostics {
            idempotency_key: "request-1".into(),
            provider_request_id: Some("upstream-1".into()),
            attempts: 4,
            max_attempts: 17,
            retries: crate::recovery::ProviderRetryCounts {
                transient: 3,
                ..Default::default()
            },
            output_started: false,
            started_at_ms: 10,
        },
        execution_recovery: None,
        observed_at_ms: 20,
        updated_at_ms: 21,
        completed_at_ms: None,
    }
}

#[test]
fn provider_incident_updates_one_durable_timeline_card() {
    let mut snapshot = Snapshot::new();
    apply(
        &mut snapshot,
        &AgentEvent::ProviderIncidentUpdated {
            run: run(),
            incident: incident(crate::recovery::ProviderIncidentStatus::Retrying),
        },
    );
    let mut settled = incident(crate::recovery::ProviderIncidentStatus::Recovered);
    settled.completed_at_ms = Some(42);
    apply(
        &mut snapshot,
        &AgentEvent::ProviderIncidentUpdated {
            run: run(),
            incident: settled.clone(),
        },
    );

    assert_eq!(snapshot.timeline.len(), 1);
    assert!(matches!(
        &snapshot.timeline[0],
        TimelineItem::ProviderIncident { id, .. } if id == &settled.id
    ));
    assert_eq!(snapshot.provider_incidents[&settled.id], settled);
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
fn goal_update_projects_authoritative_state_and_run() {
    let goal = GoalState {
        id: "goal-1".into(),
        objective: "ship the feature".into(),
        status: GoalStatus::Blocked,
        run: None,
        token_budget: Some(50_000),
        tokens_used: 12_345,
        time_used_seconds: 43,
        continuations: 2,
        updated_at_ms: 99,
        blocker_reason: Some("needs user input".into()),
    };
    let snap = reduce_all(&[AgentEvent::GoalUpdated { run: run(), goal }]);

    let projected = snap.goal.expect("goal projected");
    assert_eq!(projected.run, Some(run()));
    assert_eq!(projected.status, GoalStatus::Blocked);
    assert_eq!(
        projected.blocker_reason.as_deref(),
        Some("needs user input")
    );
}

#[test]
fn explicit_phase_patches_latest_agent_message_idempotently() {
    let phase = AgentEvent::MessagePhase {
        run: run(),
        phase: MessagePhase::Commentary,
    };
    let mut snap = reduce_all(&[
        AgentEvent::MessageChunk {
            run: run(),
            role: Role::Agent,
            delta: ContentBlock::text("First update"),
        },
        phase.clone(),
        AgentEvent::MessageChunk {
            run: run(),
            role: Role::User,
            delta: ContentBlock::text("User message"),
        },
    ]);

    apply(&mut snap, &phase);

    assert!(matches!(
        &snap.timeline[0],
        TimelineItem::Message {
            role: Role::Agent,
            phase: Some(MessagePhase::Commentary),
            ..
        }
    ));
    assert!(matches!(
        &snap.timeline[1],
        TimelineItem::Message {
            role: Role::User,
            phase: None,
            ..
        }
    ));
}

#[test]
fn tool_after_unphased_text_classifies_it_as_commentary() {
    let events = vec![
        AgentEvent::MessageChunk {
            run: run(),
            role: Role::Agent,
            delta: ContentBlock::text("I found the config; next I’ll inspect its callers."),
        },
        AgentEvent::ToolCall {
            run: run(),
            call: ToolCall {
                id: ToolCallId::new("tc-phase"),
                tool_name: Some("read_file".into()),
                title: "Read callers".into(),
                kind: ToolKind::Read,
                status: ToolStatus::Pending,
                locations: vec![],
                content: vec![],
                raw_input: None,
                progress: None,
            },
        },
    ];

    let snap = reduce_all(&events);
    assert!(matches!(
        &snap.timeline[0],
        TimelineItem::Message {
            phase: Some(MessagePhase::Commentary),
            ..
        }
    ));
}

#[test]
fn run_finish_classifies_latest_unphased_agent_message_as_final() {
    let events = vec![
        AgentEvent::MessageChunk {
            run: run(),
            role: Role::Agent,
            delta: ContentBlock::text("The requested change is complete."),
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

    let snap = reduce_all(&events);
    assert!(matches!(
        &snap.timeline[0],
        TimelineItem::Message {
            phase: Some(MessagePhase::FinalAnswer),
            ..
        }
    ));
}

#[test]
fn run_finish_preserves_explicit_commentary_phase() {
    let events = vec![
        AgentEvent::MessageChunk {
            run: run(),
            role: Role::Agent,
            delta: ContentBlock::text("I’m running the focused tests next."),
        },
        AgentEvent::MessagePhase {
            run: run(),
            phase: MessagePhase::Commentary,
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

    let snap = reduce_all(&events);
    assert!(matches!(
        &snap.timeline[0],
        TimelineItem::Message {
            phase: Some(MessagePhase::Commentary),
            ..
        }
    ));
}

#[test]
fn legacy_message_without_phase_deserializes_unphased() {
    let item: TimelineItem = serde_json::from_str(
            r#"{"item":"message","run":"run-1","role":"agent","blocks":[{"type":"text","text":"Legacy"}]}"#,
        )
        .expect("legacy timeline message should deserialize");

    assert!(matches!(item, TimelineItem::Message { phase: None, .. }));
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
                progress: None,
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
                progress: None,
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
fn structured_tool_progress_survives_final_content_replacement() {
    let id = ToolCallId::new("research-1");
    let progress = ToolCallProgress {
        revision: 3,
        status: ToolStatus::InProgress,
        latest_activity: Some("Reading official documentation".into()),
        phases: vec![ToolProgressPhase {
            id: "research".into(),
            title: "Search and verify sources".into(),
            status: ToolStatus::InProgress,
            summary: None,
            steps: vec![ToolProgressStep {
                id: "read".into(),
                title: "Read documentation".into(),
                status: ToolStatus::InProgress,
                summary: None,
            }],
        }],
        agents: vec![],
    };
    let events = vec![
        AgentEvent::ToolCall {
            run: run(),
            call: ToolCall {
                id: id.clone(),
                tool_name: Some("research_extension".into()),
                title: "Researching".into(),
                kind: ToolKind::Research,
                status: ToolStatus::Pending,
                locations: vec![],
                content: vec![],
                raw_input: None,
                progress: None,
            },
        },
        AgentEvent::ToolCallUpdate {
            run: run(),
            id: id.clone(),
            patch: ToolCallPatch {
                status: Some(ToolStatus::InProgress),
                progress: Some(progress.clone()),
                ..Default::default()
            },
        },
        AgentEvent::ToolCallUpdate {
            run: run(),
            id: id.clone(),
            patch: ToolCallPatch {
                status: Some(ToolStatus::Completed),
                replace_content: Some(vec![ContentBlock::text("Cited findings")]),
                ..Default::default()
            },
        },
    ];
    let snap = reduce_all(&events);
    let call = &snap.tool_calls[&id];
    assert_eq!(call.status, ToolStatus::Completed);
    assert_eq!(call.content, vec![ContentBlock::text("Cited findings")]);
    assert_eq!(call.progress.as_ref(), Some(&progress));
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
fn terminal_run_settles_only_its_unfinished_tool_calls() {
    for (run_status, expected_tool_status) in [
        (RunStatus::Done, ToolStatus::Completed),
        (RunStatus::Cancelled, ToolStatus::Cancelled),
        (RunStatus::Failed, ToolStatus::Failed),
    ] {
        let active_run = run();
        let other_run = RunId::new("run-other");
        let pending = ToolCallId::new("pending");
        let completed = ToolCallId::new("completed");
        let unrelated = ToolCallId::new("unrelated");
        let mut snap = reduce_all(&[
            AgentEvent::ToolCall {
                run: active_run.clone(),
                call: ToolCall {
                    id: pending.clone(),
                    tool_name: Some("web_fetch".into()),
                    title: "Fetch page".into(),
                    kind: ToolKind::Fetch,
                    status: ToolStatus::InProgress,
                    locations: vec![],
                    content: vec![],
                    raw_input: None,
                    progress: None,
                },
            },
            AgentEvent::ToolCall {
                run: active_run.clone(),
                call: ToolCall {
                    id: completed.clone(),
                    tool_name: Some("read_file".into()),
                    title: "Read file".into(),
                    kind: ToolKind::Read,
                    status: ToolStatus::Completed,
                    locations: vec![],
                    content: vec![],
                    raw_input: None,
                    progress: None,
                },
            },
            AgentEvent::ToolCall {
                run: other_run,
                call: ToolCall {
                    id: unrelated.clone(),
                    tool_name: Some("grep".into()),
                    title: "Search".into(),
                    kind: ToolKind::Search,
                    status: ToolStatus::Pending,
                    locations: vec![],
                    content: vec![],
                    raw_input: None,
                    progress: None,
                },
            },
        ]);

        apply(
            &mut snap,
            &AgentEvent::RunFinished {
                run: active_run,
                outcome: RunOutcome {
                    status: run_status,
                    stop_reason: None,
                    error: None,
                    failure_kind: None,
                    usage: None,
                    execution: None,
                },
            },
        );

        assert_eq!(snap.tool_calls[&pending].status, expected_tool_status);
        assert_eq!(snap.tool_calls[&completed].status, ToolStatus::Completed);
        assert_eq!(snap.tool_calls[&unrelated].status, ToolStatus::Pending);
    }
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
                progress: None,
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
    let mk = |s: ChecklistStatus| AgentEvent::ExecutionChecklistUpdated {
        run: run(),
        checklist: ExecutionChecklist {
            steps: vec![ChecklistStep {
                plan_step_id: None,
                title: "step".into(),
                status: s,
                priority: None,
            }],
            revision: 1,
        },
        explanation: None,
    };
    let snap = reduce_all(&[mk(ChecklistStatus::Pending), mk(ChecklistStatus::Completed)]);
    assert_eq!(
        snap.timeline
            .iter()
            .filter(|i| matches!(i, TimelineItem::ExecutionChecklist { .. }))
            .count(),
        1
    );
    match &snap.timeline[0] {
        TimelineItem::ExecutionChecklist {
            run: plan_run,
            checklist,
            ..
        } => {
            assert_eq!(plan_run.as_ref(), Some(&run()));
            assert_eq!(checklist.steps[0].status, ChecklistStatus::Completed);
        }
        other => panic!("expected plan, got {other:?}"),
    }
    assert_eq!(
        snap.execution_checklist.unwrap().steps[0].status,
        ChecklistStatus::Completed
    );

    let run_two = RunId::new("run-2");
    let snap = reduce_all(&[
        AgentEvent::ExecutionChecklistUpdated {
            run: run(),
            checklist: ExecutionChecklist {
                steps: vec![ChecklistStep {
                    plan_step_id: None,
                    title: "first".into(),
                    status: ChecklistStatus::Completed,
                    priority: None,
                }],
                revision: 1,
            },
            explanation: None,
        },
        AgentEvent::ExecutionChecklistUpdated {
            run: run_two,
            checklist: ExecutionChecklist {
                steps: vec![ChecklistStep {
                    plan_step_id: None,
                    title: "second".into(),
                    status: ChecklistStatus::InProgress,
                    priority: None,
                }],
                revision: 1,
            },
            explanation: None,
        },
    ]);
    assert_eq!(
        snap.timeline
            .iter()
            .filter(|i| matches!(i, TimelineItem::ExecutionChecklist { .. }))
            .count(),
        2
    );
}

#[test]
fn proposed_plan_revision_and_approval_update_one_durable_timeline_item() {
    let proposal = |revision, status, markdown: &str| AgentEvent::ProposedPlanUpdated {
        run: run(),
        plan: ProposedPlan {
            id: "proposal-1".into(),
            revision,
            markdown: markdown.into(),
            status,
            global_reminders: Vec::new(),
            execution_contract: Vec::new(),
            context_revisions: Vec::new(),
        },
    };
    let snap = reduce_all(&[
        proposal(1, ProposedPlanStatus::AwaitingDecision, "first"),
        proposal(2, ProposedPlanStatus::AwaitingDecision, "revised"),
        proposal(2, ProposedPlanStatus::Approved, "revised"),
    ]);
    assert_eq!(
        snap.timeline
            .iter()
            .filter(|item| matches!(item, TimelineItem::ProposedPlan { .. }))
            .count(),
        1
    );
    let plan = snap.proposed_plan.expect("latest proposal");
    assert_eq!(plan.revision, 2);
    assert_eq!(plan.status, ProposedPlanStatus::Approved);
    assert_eq!(plan.markdown, "revised");
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
fn specialist_presentation_updates_one_durable_timeline_item() {
    let presentation = SpecialistPresentation {
        id: "security-archive-boundary".into(),
        kind: "security".into(),
        prompt: "Review the archive path.".into(),
        title: "Archive extraction can cross the workspace boundary".into(),
        summary: "The path reaches the write sink before containment.".into(),
        takeaway: "Move containment ahead of the write.".into(),
        diagram: "flowchart LR".into(),
        diagram_title: "Validated attack path".into(),
        metrics: vec![],
        evidence: vec![],
        stages: vec![],
        limitation: "Requires accepted receipts.".into(),
    };
    let updated = SpecialistPresentation {
        summary: "The guarded path now has a reproducible control.".into(),
        ..presentation.clone()
    };

    let events = [
        AgentEvent::SpecialistPresentation {
            run: run(),
            presentation,
        },
        AgentEvent::SpecialistPresentation {
            run: run(),
            presentation: updated.clone(),
        },
    ];
    let snapshot = reduce_all(&events);

    assert_eq!(
        snapshot
            .timeline
            .iter()
            .filter(|item| matches!(item, TimelineItem::SpecialistPresentation { .. }))
            .count(),
        1
    );
    assert!(matches!(
        &snapshot.timeline[0],
        TimelineItem::SpecialistPresentation { presentation, .. }
            if presentation == &updated
    ));
}

#[test]
fn cloud_snapshot_restores_typed_provider_history_and_title() {
    let events = [
        AgentEvent::MessageChunk {
            run: run(),
            role: Role::User,
            delta: ContentBlock::text(
                "Replicate the long-running experiment with a controlled baseline and receipts",
            ),
        },
        AgentEvent::MessageChunk {
            run: run(),
            role: Role::Agent,
            delta: ContentBlock::text("I will inspect the preregistration first."),
        },
    ];
    let snapshot = reduce_all(&events);

    let resume = snapshot.resume_transcript().expect("typed history");
    assert_eq!(resume.items.len(), 2);
    assert!(matches!(
        &resume.items[0],
        crate::provider::ResumeItem::Message { role: Role::User, blocks }
            if blocks == &vec![ContentBlock::text(
                "Replicate the long-running experiment with a controlled baseline and receipts"
            )]
    ));
    assert_eq!(
        snapshot.derived_title(),
        "Replicate the long-running experiment with a controlled b…"
    );
    assert!(snapshot.has_conversation_content());
}

#[test]
fn legacy_cloud_plan_normalizes_in_the_shared_projection_core() {
    let normalized = normalize_snapshot_value(serde_json::json!({
        "runs": {},
        "timeline": [{
            "item": "plan",
            "run": "run-1",
            "plan": {"phases": [
                {"title": "Inspect", "status": "done"},
                {"title": "Replicate", "status": "in-progress"}
            ]}
        }],
        "plan": {"phases": [{"title": "Inspect", "status": "completed"}]},
        "tool_calls": {},
        "artifacts": []
    }));

    assert_eq!(normalized["timeline"][0]["item"], "execution_checklist");
    assert_eq!(
        normalized["timeline"][0]["checklist"]["steps"][1]["status"],
        "in_progress"
    );
    assert!(normalized.get("plan").is_none());
    assert!(serde_json::from_value::<Snapshot>(normalized).is_ok());
}

#[path = "projection_tests/projection_extended.rs"]
mod projection_extended;
