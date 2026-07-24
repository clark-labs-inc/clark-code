//! Session goal tools. `create_goal` starts a
//! standing objective the engine then pursues autonomously (continuation
//! turns after each clean completion); `update_goal` is restricted to
//! `complete`/`blocked` so the model can never grant itself more runway
//! (budgets and stopping are user/engine-owned); `get_goal` reports status
//! and usage.

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::loop_state::{GoalStatus, SessionGoal};
use crate::tools::{arg_str, ToolCtx, ToolExecutor, ToolOutcome, ToolSignal};

fn goal_summary(goal: &SessionGoal) -> String {
    let budget = match goal.token_budget {
        Some(budget) => format!(
            "{} of {} tokens used ({} remaining)",
            goal.tokens_used,
            budget,
            budget.saturating_sub(goal.tokens_used)
        ),
        None => format!("{} tokens used, no budget", goal.tokens_used),
    };
    format!(
        "Goal is {}. Objective: {} — {budget}; {}s elapsed; {} continuation turn(s).",
        goal.status_label(),
        goal.objective,
        goal.time_used_seconds,
        goal.continuations
    )
}

/// Start a goal from either the model tool or the deterministic `/goal`
/// command path. Keeping the validation here prevents the two entry points
/// from drifting into subtly different lifecycle semantics.
pub(crate) fn start_goal(
    session: &mut crate::loop_state::SessionState,
    objective: String,
    token_budget: Option<u64>,
) -> Result<(), String> {
    let objective = objective.trim().to_string();
    if objective.is_empty() {
        return Err("`objective` must not be empty".into());
    }
    if objective.chars().count() > 4_000 {
        return Err("`objective` must be at most 4000 characters".into());
    }
    if session.planning.plan_mode() {
        return Err(
            "Plan mode is active — agree on a plan first; a goal can be created after \
             the plan is approved."
                .into(),
        );
    }
    if let Some(existing) = &session.goal {
        if existing.status != GoalStatus::Complete {
            return Err(format!(
                "an unfinished goal already exists ({}): finish it with update_goal or \
                 ask the user to clear it. Objective: {}",
                existing.status_label(),
                existing.objective
            ));
        }
    }

    let mut goal = SessionGoal {
        id: uuid::Uuid::new_v4().to_string(),
        objective,
        status: GoalStatus::Active,
        token_budget,
        tokens_used: 0,
        time_used_seconds: 0,
        continuations: 0,
        updated_at_ms: 0,
        blocker_reason: None,
        blocker_observations: 0,
        last_blocker_continuation: None,
    };
    goal.touch();
    session.goal = Some(goal);
    Ok(())
}

pub struct CreateGoal;

#[async_trait]
impl ToolExecutor for CreateGoal {
    fn name(&self) -> &str {
        "create_goal"
    }
    fn description(&self) -> &str {
        "Start a standing goal this session pursues autonomously: after each of your turns \
        completes, the runtime automatically gives you another continuation turn toward the \
        objective until you prove it complete with update_goal. Call this ONLY when the user \
        explicitly asks for autonomous or keep-going-until-done work — never infer a goal from \
        an ordinary task. Set token_budget only when the user gave one. Fails while an \
        unfinished goal exists."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "objective": {
                    "type": "string",
                    "description": "The concrete end state to pursue, in full."
                },
                "token_budget": {
                    "type": "integer",
                    "description": "Optional positive token cap for the goal. Omit unless the user asked for one."
                }
            },
            "required": ["objective"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let Ok(objective) = arg_str(&args, "objective") else {
            return ToolOutcome::error("missing required string argument `objective`");
        };
        let token_budget = match args.get("token_budget") {
            None | Some(Value::Null) => None,
            Some(value) => match value.as_u64().filter(|budget| *budget > 0) {
                Some(budget) => Some(budget),
                None => {
                    return ToolOutcome::error("`token_budget` must be a positive integer");
                }
            },
        };

        let mut session = ctx.session.lock().await;
        if let Err(error) = start_goal(&mut session, objective.to_string(), token_budget) {
            return ToolOutcome::error(error);
        }
        let budget_note = match token_budget {
            Some(budget) => format!(" Token budget: {budget}."),
            None => String::new(),
        };
        let state = session.goal.as_ref().expect("goal was created").state(None);
        ToolOutcome::ok(format!(
            "Goal created and active.{budget_note} The runtime will keep giving you \
             continuation turns toward it after each completed turn. Work toward the full \
             objective; call update_goal with status \"complete\" only when current evidence \
             proves every requirement is satisfied."
        ))
        .with_signal(ToolSignal::Goal(state))
    }
}

pub struct UpdateGoal;

#[async_trait]
impl ToolExecutor for UpdateGoal {
    fn name(&self) -> &str {
        "update_goal"
    }
    fn description(&self) -> &str {
        "Update the session goal's status — ONLY to `complete` or `blocked`. Set \
        \"complete\" only when the objective is actually achieved: audit every explicit \
        requirement against current files/command output first; the evidence must prove \
        completion, not merely fail to show remaining work. Set \"blocked\" only after the \
        same blocking condition repeated for at least three consecutive goal turns and no \
        progress is possible without the user. Never use \"blocked\" because the work is \
        hard, slow, or unclear, and never mark complete because the budget is nearly gone. \
        You cannot pause or re-budget a goal — that belongs to the user."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["complete", "blocked"],
                    "description": "The new goal status."
                },
                "reason": {
                    "type": "string",
                    "description": "Required for blocked. The concrete external condition preventing progress."
                }
            },
            "required": ["status"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let status = match args.get("status").and_then(Value::as_str) {
            Some("complete") => GoalStatus::Complete,
            Some("blocked") => GoalStatus::Blocked,
            _ => {
                return ToolOutcome::error(
                    "`status` must be \"complete\" or \"blocked\" — other transitions are \
                     user/engine-owned",
                );
            }
        };
        let completes_goal = status == GoalStatus::Complete;
        let mut session = ctx.session.lock().await;
        let Some(goal) = session.goal.as_mut() else {
            return ToolOutcome::error("no goal exists for this session");
        };
        if goal.status == GoalStatus::Complete {
            return ToolOutcome::error("the goal is already complete");
        }
        let outcome = match status {
            GoalStatus::Complete => {
                goal.status = GoalStatus::Complete;
                goal.blocker_reason = None;
                goal.touch();
                ToolOutcome::ok(
                    "Goal marked complete. The UI shows the elapsed time. In the final reply, \
                     summarize the outcome only; do not report token usage, budgets, \
                     continuation counts, or session counts.",
                )
            }
            GoalStatus::Blocked => {
                let Some(reason) = args.get("reason").and_then(Value::as_str) else {
                    return ToolOutcome::error("`reason` is required when marking a goal blocked");
                };
                let reason = reason.trim();
                if reason.is_empty() || reason.chars().count() > 1_000 {
                    return ToolOutcome::error("`reason` must be between 1 and 1000 characters");
                }
                let same_condition = goal
                    .blocker_reason
                    .as_deref()
                    .is_some_and(|previous| previous.eq_ignore_ascii_case(reason));
                if !same_condition {
                    goal.blocker_reason = Some(reason.to_string());
                    goal.blocker_observations = 1;
                    goal.last_blocker_continuation = Some(goal.continuations);
                } else if goal.last_blocker_continuation != Some(goal.continuations) {
                    goal.blocker_observations = goal.blocker_observations.saturating_add(1);
                    goal.last_blocker_continuation = Some(goal.continuations);
                }
                goal.touch();
                if goal.blocker_observations < 3 {
                    return ToolOutcome::error(format!(
                        "Blocked state rejected: this condition has repeated for {}/3 \
                         consecutive goal turns. The goal remains active; continue with any \
                         safe work and only report the same blocker again on a later goal turn.",
                        goal.blocker_observations
                    ));
                }
                goal.status = GoalStatus::Blocked;
                let summary = goal_summary(goal);
                ToolOutcome::ok(format!(
                    "Goal marked blocked — automatic continuation stops here. {summary} Tell \
                     the user exactly what is blocking and what you need from them."
                ))
            }
            _ => unreachable!("update_goal accepts only complete or blocked"),
        };
        let state = goal.state(None);
        let checklist = completes_goal
            .then(|| session.planning.complete_execution_checklist())
            .flatten();
        let outcome = if let Some(checklist) = checklist {
            outcome.with_signal(ToolSignal::ExecutionChecklist {
                checklist,
                explanation: Some(
                    "Goal completed; marked the remaining checklist steps complete.".into(),
                ),
            })
        } else {
            outcome
        };
        outcome.with_signal(ToolSignal::Goal(state))
    }
}

pub struct GetGoal;

#[async_trait]
impl ToolExecutor for GetGoal {
    fn name(&self) -> &str {
        "get_goal"
    }
    fn description(&self) -> &str {
        "Get the current session goal: objective, status, token budget and usage, elapsed \
        time, and continuation count."
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    async fn invoke(&self, _args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let session = ctx.session.lock().await;
        match &session.goal {
            Some(goal) => ToolOutcome::ok(goal_summary(goal)),
            None => ToolOutcome::ok("No goal is set for this session."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;
    use crate::tools::ReadTracker;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn test_ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(dir).unwrap()),
            executor: Arc::new(crate::exec::LocalExecutor),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(
                crate::loop_state::SessionState::default(),
            )),
            progress: None,
            agent_progress: None,
            call_progress: None,
        }
    }

    #[tokio::test]
    async fn create_then_update_goal_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());

        let created = CreateGoal
            .invoke(
                json!({"objective": "build the whole site", "token_budget": 50_000}),
                &ctx,
            )
            .await;
        assert!(!created.is_error);
        {
            let s = ctx.session.lock().await;
            let goal = s.goal.as_ref().unwrap();
            assert_eq!(goal.status, GoalStatus::Active);
            assert_eq!(goal.token_budget, Some(50_000));
        }

        // A second unfinished goal is refused.
        let duplicate = CreateGoal
            .invoke(json!({"objective": "another"}), &ctx)
            .await;
        assert!(duplicate.is_error);
        assert!(duplicate.content.contains("unfinished goal"));

        let checklist = crate::tools::plan::UpdatePlan
            .invoke(
                json!({"plan": [
                    {"step": "Implement the site", "status": "completed"},
                    {"step": "Verify the site", "status": "in_progress"}
                ]}),
                &ctx,
            )
            .await;
        assert!(!checklist.is_error);

        let done = UpdateGoal.invoke(json!({"status": "complete"}), &ctx).await;
        assert!(!done.is_error);
        assert!(done.content.contains("UI shows the elapsed time"));
        assert!(!done.content.contains("tokens used"));
        assert!(!done.content.contains("continuation turn"));
        assert!(!done.content.contains("Report the final usage"));
        assert_eq!(
            ctx.session.lock().await.goal.as_ref().unwrap().status,
            GoalStatus::Complete
        );
        assert!(matches!(
            done.signals.as_slice(),
            [
                ToolSignal::ExecutionChecklist { checklist, explanation: Some(_) },
                ToolSignal::Goal(goal),
            ] if checklist.revision == 2
                && checklist.steps.iter().all(|step| step.status == agent_core::domain::ChecklistStatus::Completed)
                && goal.status == GoalStatus::Complete
        ));
        let persisted_checklist = ctx
            .session
            .lock()
            .await
            .planning
            .execution_checklist
            .clone()
            .expect("completed checklist persisted");
        assert_eq!(persisted_checklist.revision, 2);
        assert!(persisted_checklist
            .steps
            .iter()
            .all(|step| step.status == agent_core::domain::ChecklistStatus::Completed));

        // Once complete, a new goal may replace it.
        let replacement = CreateGoal
            .invoke(json!({"objective": "next thing"}), &ctx)
            .await;
        assert!(!replacement.is_error);
    }

    #[tokio::test]
    async fn update_goal_rejects_other_statuses_and_missing_goal() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());

        let none = UpdateGoal.invoke(json!({"status": "complete"}), &ctx).await;
        assert!(none.is_error);

        CreateGoal.invoke(json!({"objective": "x"}), &ctx).await;
        let paused = UpdateGoal.invoke(json!({"status": "paused"}), &ctx).await;
        assert!(paused.is_error, "pause is user/engine-owned");
    }

    #[tokio::test]
    async fn create_goal_refused_in_plan_mode_and_reports_via_get() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        ctx.session
            .lock()
            .await
            .planning
            .set_mode(agent_core::provider::CollaborationMode::Plan);
        let refused = CreateGoal.invoke(json!({"objective": "x"}), &ctx).await;
        assert!(refused.is_error);
        assert!(refused.content.contains("Plan mode"));

        ctx.session
            .lock()
            .await
            .planning
            .set_mode(agent_core::provider::CollaborationMode::Default);
        let empty = GetGoal.invoke(json!({}), &ctx).await;
        assert!(empty.content.contains("No goal"));
        CreateGoal
            .invoke(json!({"objective": "ship it"}), &ctx)
            .await;
        let report = GetGoal.invoke(json!({}), &ctx).await;
        assert!(report.content.contains("active"));
        assert!(report.content.contains("ship it"));
    }

    #[tokio::test]
    async fn blocked_requires_same_reason_on_three_distinct_goal_turns() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        CreateGoal
            .invoke(json!({"objective": "ship it"}), &ctx)
            .await;

        for continuation in 0..2 {
            ctx.session
                .lock()
                .await
                .goal
                .as_mut()
                .unwrap()
                .continuations = continuation;
            let rejected = UpdateGoal
                .invoke(
                    json!({"status": "blocked", "reason": "Waiting for the user's API key"}),
                    &ctx,
                )
                .await;
            assert!(rejected.is_error);
            assert_eq!(
                ctx.session.lock().await.goal.as_ref().unwrap().status,
                GoalStatus::Active
            );
        }

        ctx.session
            .lock()
            .await
            .goal
            .as_mut()
            .unwrap()
            .continuations = 2;
        let accepted = UpdateGoal
            .invoke(
                json!({"status": "blocked", "reason": "Waiting for the user's API key"}),
                &ctx,
            )
            .await;
        assert!(!accepted.is_error);
        assert_eq!(
            ctx.session.lock().await.goal.as_ref().unwrap().status,
            GoalStatus::Blocked
        );
    }
}
