//! Plan Mode tools: `enter_plan_mode` (the Claude-Code-`EnterPlanMode` analog —
//! the agent suggests planning first, the user approves), `propose_plan` (the
//! Claude-Code-`ExitPlanMode`/Codex-`<proposed_plan>` analog — signals "done
//! researching, please approve") and `update_plan` (the Codex `update_plan`
//! checklist analog — an always-available, advisory TODO tracker, independent
//! of Plan Mode).

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::{arg_str, ToolCtx, ToolExecutor, ToolOutcome, ToolSignal};

pub struct ProposePlan;

#[async_trait]
impl ToolExecutor for ProposePlan {
    fn name(&self) -> &str {
        "propose_plan"
    }
    fn description(&self) -> &str {
        "In Plan Mode, call once when the implementation plan is decision-complete. Provide 3-7 \
        terse Markdown steps with exact paths and verification, without preamble or code blocks; \
        the turn ends for user review."
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
        false
    }
    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        arg_str(args, "plan").ok()
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        if !ctx.session.lock().await.planning.plan_mode() {
            return ToolOutcome::error("propose_plan is only available in Plan Mode");
        }
        let markdown = arg_str(&args, "plan")
            .unwrap_or_default()
            .trim()
            .to_string();
        if markdown.is_empty() {
            return ToolOutcome::error("the proposed plan cannot be empty");
        }
        let plan = ctx.session.lock().await.planning.next_proposal(markdown);
        ToolOutcome::ok(
            "Plan proposed for review. Plan Mode remains active; end this turn and wait for the user's decision.",
        )
        .with_signal(ToolSignal::ProposedPlan(plan))
    }
}

pub struct EnterPlanMode;

#[async_trait]
impl ToolExecutor for EnterPlanMode {
    fn name(&self) -> &str {
        "enter_plan_mode"
    }
    fn description(&self) -> &str {
        "Suggest Plan Mode for large or materially ambiguous implementation work where agreeing on \
        an approach would prevent rework. The user must approve."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "One short, plain sentence shown to the user on why \
                        planning first helps here."
                }
            }
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    fn mutating(&self) -> bool {
        // Collaboration-mode requests require an explicit user decision.
        true
    }
    // Runs only after the user approved; the gate has already flipped the
    // session into plan mode. The full workflow reminder arrives with the next
    // user turn — this result carries the condensed rules for the rest of the
    // current turn.
    async fn invoke(&self, _args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        ToolOutcome::ok(
            "Plan Mode entered. Research read-only and ask only material decisions. When the plan \
             is decision-complete, call `propose_plan` once and end the turn.",
        )
    }
}

pub struct UpdatePlan;

#[async_trait]
impl ToolExecutor for UpdatePlan {
    fn name(&self) -> &str {
        "update_plan"
    }
    fn description(&self) -> &str {
        "Replace the visible execution checklist with the full step list. Keep exactly one step \
        `in_progress` until completion; move each step through `in_progress` before `completed`, \
        and explain changed steps. Unavailable in Plan Mode."
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
        let mut session = _ctx.session.lock().await;
        if session.planning.plan_mode() {
            return ToolOutcome::error(
                "update_plan is a TODO/checklist tool and is not allowed in Plan Mode",
            );
        }
        let update = match crate::planning::parse_checklist_update(
            &args,
            session.planning.execution_checklist.as_ref(),
        ) {
            Ok(update) => update,
            Err(error) => return ToolOutcome::error(error),
        };
        session.planning.execution_checklist = Some(update.checklist.clone());
        ToolOutcome::ok("Plan updated").with_signal(ToolSignal::ExecutionChecklist {
            checklist: update.checklist,
            explanation: update.explanation,
        })
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

    #[test]
    fn propose_plan_is_read_only_and_previews_the_plan_text() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        let tool = ProposePlan;
        assert!(!tool.mutating());
        let args = json!({"plan": "1. do a thing"});
        assert_eq!(tool.preview(&args, &ctx), Some("1. do a thing".to_string()));
    }

    #[tokio::test]
    async fn propose_plan_emits_typed_state_without_writing_a_file() {
        let project = tempfile::tempdir().unwrap();
        let docs = tempfile::tempdir().unwrap();
        let mut ctx = test_ctx(project.path());
        ctx.sandbox = Arc::new(
            Sandbox::new(project.path())
                .unwrap()
                .with_docs(docs.path().to_path_buf()),
        );
        ctx.session
            .lock()
            .await
            .planning
            .set_mode(agent_core::provider::CollaborationMode::Plan);

        let outcome = ProposePlan
            .invoke(json!({"plan": "# Plan\n1. do the thing"}), &ctx)
            .await;

        assert!(!outcome.is_error);
        assert!(!docs.path().join("plan.md").exists());
        assert!(outcome.locations.is_empty());
        assert!(
            matches!(outcome.signals.as_slice(), [ToolSignal::ProposedPlan(plan)] if plan.markdown.contains("do the thing"))
        );
    }

    #[tokio::test]
    async fn propose_plan_without_docs_workspace_still_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        ctx.session
            .lock()
            .await
            .planning
            .set_mode(agent_core::provider::CollaborationMode::Plan);

        let outcome = ProposePlan.invoke(json!({"plan": "the steps"}), &ctx).await;

        assert!(!outcome.is_error);
        assert!(outcome.locations.is_empty());
        assert!(outcome.content.contains("wait for the user's decision"));
    }

    #[tokio::test]
    async fn enter_plan_mode_is_gated_and_returns_condensed_rules() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        assert!(EnterPlanMode.mutating(), "must always pass the gate");

        let outcome = EnterPlanMode.invoke(json!({}), &ctx).await;

        assert!(!outcome.is_error);
        assert!(outcome.content.contains("propose_plan"));
        assert!(outcome.content.contains("read-only"));
        assert!(outcome.content.contains("decision-complete"));
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
