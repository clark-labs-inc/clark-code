use agent_core::{ChecklistStep, RunId};

use super::*;

fn goal(status: GoalStatus, continuations: u32) -> ProviderGoalState {
    ProviderGoalState {
        id: "goal-7".into(),
        objective: "ship the Clark terminal".into(),
        status,
        run: Some(RunId::new("run-7")),
        token_budget: Some(50_000),
        tokens_used: 12_000,
        time_used_seconds: 90,
        continuations,
        updated_at_ms: 42,
        blocker_reason: (status == GoalStatus::Blocked).then(|| "needs user input".into()),
    }
}

fn plan() -> ProposedPlan {
    ProposedPlan {
        id: "plan-7".into(),
        revision: 2,
        markdown: "# Clark plan\nImplement the terminal state.".into(),
        status: ProposedPlanStatus::AwaitingDecision,
        global_reminders: Vec::new(),
        execution_contract: Vec::new(),
    }
}

#[test]
fn typed_plan_goal_and_checklist_render_as_dedicated_state() {
    let run = RunId::new("run-7");
    let mut state = PlanGoalState::new(CollaborationMode::Plan);
    state.observe_event(&AgentEvent::ProposedPlanUpdated {
        run: run.clone(),
        plan: plan(),
    });
    state.observe_event(&AgentEvent::ExecutionChecklistUpdated {
        run: run.clone(),
        checklist: ExecutionChecklist {
            revision: 3,
            steps: vec![ChecklistStep {
                plan_step_id: Some("step-1".into()),
                title: "Build it".into(),
                status: ChecklistStatus::InProgress,
                priority: None,
            }],
        },
        explanation: None,
    });
    state.observe_event(&AgentEvent::GoalUpdated {
        run,
        goal: goal(GoalStatus::Active, 3),
    });

    let lines = state.panel_lines().join("\n");
    assert!(lines.contains("Plan r2 · awaiting decision"));
    assert!(lines.contains("Checklist · 0/1 complete · now: Build it"));
    assert!(lines.contains("Goal · active · ship the Clark terminal"));
    assert!(state.desired_height() > 0);
}

#[test]
fn goal_status_and_continuations_round_trip_through_typed_resume_items() {
    for status in [
        GoalStatus::Active,
        GoalStatus::Blocked,
        GoalStatus::BudgetLimited,
        GoalStatus::Complete,
    ] {
        let mut original = PlanGoalState::new(CollaborationMode::Plan);
        original.proposed_plan = Some(plan());
        original.goal = Some(goal(status, 4));
        let mut transcript = ResumeTranscript::default();
        original.append_resume_items(&mut transcript);
        let encoded = serde_json::to_string(&transcript).unwrap();
        let decoded: ResumeTranscript = serde_json::from_str(&encoded).unwrap();
        let restored = PlanGoalState::restore(CollaborationMode::Plan, &decoded);

        assert_eq!(restored.collaboration_mode, CollaborationMode::Plan);
        assert_eq!(restored.proposed_plan, original.proposed_plan);
        assert_eq!(restored.goal, original.goal);
        assert_eq!(restored.goal.as_ref().unwrap().continuations, 4);
    }
}

#[test]
fn command_parser_keeps_goal_objectives_for_the_provider_boundary() {
    assert_eq!(parse("/goal"), Some(Ok(PlanGoalCommand::Inspect)));
    assert_eq!(parse("/goal status"), Some(Ok(PlanGoalCommand::Inspect)));
    assert_eq!(
        parse("/goal resume --tokens 24000"),
        Some(Ok(PlanGoalCommand::Resume(Some(24_000))))
    );
    assert!(matches!(parse("/goal resume --tokens nope"), Some(Err(_))));
    assert_eq!(parse("/goal ship every gap"), None);
    assert_eq!(parse("/plan implement fresh"), None);
}
