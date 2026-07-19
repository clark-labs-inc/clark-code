//! Session goal tools — the Codex `/goal` analog. `create_goal` starts a
//! standing objective the engine then pursues autonomously (continuation
//! turns after each clean completion); `update_goal` is restricted to
//! `complete`/`blocked` so the model can never grant itself more runway
//! (budgets and stopping are user/engine-owned); `get_goal` reports status
//! and usage.

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::loop_state::{GoalStatus, SessionGoal};
use crate::tools::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};

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
        goal.status.label(),
        goal.objective,
        goal.time_used_seconds,
        goal.continuations
    )
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
        let objective = objective.trim().to_string();
        if objective.is_empty() {
            return ToolOutcome::error("`objective` must not be empty");
        }
        if objective.chars().count() > 4_000 {
            return ToolOutcome::error("`objective` must be at most 4000 characters");
        }
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
        if session.plan_mode {
            return ToolOutcome::error(
                "Plan mode is active — agree on a plan first; a goal can be created after \
                 the plan is approved.",
            );
        }
        if let Some(existing) = &session.goal {
            if existing.status != GoalStatus::Complete {
                return ToolOutcome::error(format!(
                    "an unfinished goal already exists ({}): finish it with update_goal or \
                     ask the user to clear it. Objective: {}",
                    existing.status.label(),
                    existing.objective
                ));
            }
        }
        let goal = SessionGoal {
            objective: objective.clone(),
            status: GoalStatus::Active,
            token_budget,
            tokens_used: 0,
            time_used_seconds: 0,
            continuations: 0,
        };
        session.goal = Some(goal);
        let budget_note = match token_budget {
            Some(budget) => format!(" Token budget: {budget}."),
            None => String::new(),
        };
        ToolOutcome::ok(format!(
            "Goal created and active.{budget_note} The runtime will keep giving you \
             continuation turns toward it after each completed turn. Work toward the full \
             objective; call update_goal with status \"complete\" only when current evidence \
             proves every requirement is satisfied."
        ))
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
        let mut session = ctx.session.lock().await;
        let Some(goal) = session.goal.as_mut() else {
            return ToolOutcome::error("no goal exists for this session");
        };
        if goal.status == GoalStatus::Complete {
            return ToolOutcome::error("the goal is already complete");
        }
        goal.status = status;
        let summary = goal_summary(goal);
        match status {
            GoalStatus::Complete => ToolOutcome::ok(format!(
                "Goal marked complete. {summary} Report the final usage to the user."
            )),
            _ => ToolOutcome::ok(format!(
                "Goal marked blocked — automatic continuation stops here. {summary} Tell \
                 the user exactly what is blocking and what you need from them."
            )),
        }
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

        let done = UpdateGoal.invoke(json!({"status": "complete"}), &ctx).await;
        assert!(!done.is_error);
        assert_eq!(
            ctx.session.lock().await.goal.as_ref().unwrap().status,
            GoalStatus::Complete
        );

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
        ctx.session.lock().await.plan_mode = true;
        let refused = CreateGoal.invoke(json!({"objective": "x"}), &ctx).await;
        assert!(refused.is_error);
        assert!(refused.content.contains("Plan mode"));

        ctx.session.lock().await.plan_mode = false;
        let empty = GetGoal.invoke(json!({}), &ctx).await;
        assert!(empty.content.contains("No goal"));
        CreateGoal
            .invoke(json!({"objective": "ship it"}), &ctx)
            .await;
        let report = GetGoal.invoke(json!({}), &ctx).await;
        assert!(report.content.contains("active"));
        assert!(report.content.contains("ship it"));
    }
}
