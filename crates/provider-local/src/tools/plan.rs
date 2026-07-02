//! Plan Mode tools: `propose_plan` (the Claude-Code-`ExitPlanMode`/Codex-
//! `<proposed_plan>` analog — signals "done researching, please approve") and
//! `update_plan` (the Codex `update_plan` checklist analog — an
//! always-available, advisory TODO tracker, independent of Plan Mode).

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};

pub struct ProposePlan;

#[async_trait]
impl ToolExecutor for ProposePlan {
    fn name(&self) -> &str {
        "propose_plan"
    }
    fn description(&self) -> &str {
        "Call this when you've finished researching and are ready for the user to review your \
        plan. Only for tasks that require planning implementation steps that will change code or \
        the system — not for pure research/explanation tasks. Write the complete plan, in \
        markdown, as the `plan` argument; the user will approve it (letting you proceed) or ask \
        you to keep planning."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "The complete plan, in markdown, for the user to review."
                }
            },
            "required": ["plan"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    fn mutating(&self) -> bool {
        true
    }
    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        arg_str(args, "plan").ok()
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        ToolOutcome::ok("The user approved your plan. You can now implement it.")
    }
}

pub struct UpdatePlan;

#[async_trait]
impl ToolExecutor for UpdatePlan {
    fn name(&self) -> &str {
        "update_plan"
    }
    fn description(&self) -> &str {
        "Update the task checklist shown to the user. Provide the full list of steps each time \
        (not a diff), each with a status. At most one step may be `in_progress` at a time; move a \
        step to `in_progress` before marking it `completed` (don't skip straight to completed). \
        Not usable while Plan Mode is active — that's a separate read-only research phase; use \
        propose_plan there instead."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "explanation": {
                    "type": "string",
                    "description": "Optional short explanation for this plan update."
                },
                "plan": {
                    "type": "array",
                    "description": "The full list of steps.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "step": {"type": "string", "description": "Short step text."},
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            }
                        },
                        "required": ["step", "status"]
                    }
                }
            },
            "required": ["plan"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        if !args.get("plan").is_some_and(Value::is_array) {
            return ToolOutcome::error("missing required array argument `plan`");
        }
        ToolOutcome::ok("Plan updated")
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
        }
    }

    #[test]
    fn propose_plan_is_mutating_and_previews_the_plan_text() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        let tool = ProposePlan;
        assert!(tool.mutating());
        let args = json!({"plan": "1. do a thing"});
        assert_eq!(tool.preview(&args, &ctx), Some("1. do a thing".to_string()));
    }

    #[test]
    fn update_plan_is_not_mutating() {
        assert!(!UpdatePlan.mutating());
    }

    #[tokio::test]
    async fn update_plan_requires_plan_array() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        let outcome = UpdatePlan.invoke(json!({}), &ctx).await;
        assert!(outcome.is_error);
    }

    #[tokio::test]
    async fn update_plan_accepts_a_valid_plan() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        let outcome = UpdatePlan
            .invoke(
                json!({"plan": [{"step": "a", "status": "in_progress"}]}),
                &ctx,
            )
            .await;
        assert!(!outcome.is_error);
    }
}
