//! Plan Mode tools: `enter_plan_mode` (the Claude-Code-`EnterPlanMode` analog —
//! the agent suggests planning first, the user approves), `propose_plan` (the
//! Claude-Code-`ExitPlanMode`/Codex-`<proposed_plan>` analog — signals "done
//! researching, please approve") and `update_plan` (the Codex `update_plan`
//! checklist analog — an always-available, advisory TODO tracker, independent
//! of Plan Mode).

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
        the system — not for pure research/explanation tasks. Write the complete plan as terse \
        markdown: normally 3–7 steps, exact paths, reuse, and verification; omit the research \
        diary, preamble, and exhaustive alternatives. The user will approve it or provide \
        concrete feedback."
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
    // Runs only after the user approved (the gate forces an explicit decision).
    // Persist the approved plan into the session's document workspace so it
    // survives the gate: the write surfaces as an inline document card in the
    // conversation, and the model gets a stable path to consult while building.
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let plan = arg_str(&args, "plan").unwrap_or_default();
        let saved = ctx.sandbox.docs_root().and_then(|docs| {
            let path = crate::workspace::plan_file(docs);
            std::fs::write(&path, plan.as_bytes()).ok()?;
            Some(path)
        });
        let mut content = String::from(
            "The user approved your plan. You can now build it. Start by setting up your task \
            checklist with `update_plan` if the work has multiple steps.",
        );
        if let Some(path) = &saved {
            content.push_str(&format!(
                "\n\nYour plan is saved at: {} — refer back to it during implementation.",
                path.display()
            ));
        }
        content.push_str(&format!("\n\n## Approved plan\n{plan}"));
        let outcome = ToolOutcome::ok(content);
        match saved {
            Some(path) => outcome.with_location(path.display().to_string(), None),
            None => outcome,
        }
    }
}

pub struct EnterPlanMode;

#[async_trait]
impl ToolExecutor for EnterPlanMode {
    fn name(&self) -> &str {
        "enter_plan_mode"
    }
    fn description(&self) -> &str {
        "Suggest switching to Plan Mode before building. Call this for large, multi-file, or \
        ambiguous requests where agreeing on an approach first would save rework — not for \
        small fixes, pure research/explanation tasks, or when the user gave precise \
        instructions. The user must approve; if they decline, proceed directly without a \
        separate planning phase."
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
        // Not literally a mutation, but it must always pass through the
        // permission gate — entering plan mode is the user's call.
        true
    }
    // Runs only after the user approved; the gate has already flipped the
    // session into plan mode. The full workflow reminder arrives with the next
    // user turn — this result carries the condensed rules for the rest of the
    // current turn.
    async fn invoke(&self, _args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let mut content = String::from(
            "Entered plan mode — the user wants to agree on a plan before anything changes. \
            Research read-only. Be terse: one batched search, then read only the few relevant \
            files; no repeated probes or narrated research. Ask only about decisions the user \
            must make. Once what / where / reuse / verify are known, call `propose_plan` with \
            normally 3–7 concise steps.",
        );
        if let Some(docs) = ctx.sandbox.docs_root() {
            content.push_str(&format!(
                " Draft your plan as you research in {} — it renders live for the user.",
                crate::workspace::plan_file(docs).display()
            ));
        }
        ToolOutcome::ok(content)
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
            progress: None,
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

    #[tokio::test]
    async fn propose_plan_persists_the_approved_plan_into_the_docs_workspace() {
        let project = tempfile::tempdir().unwrap();
        let docs = tempfile::tempdir().unwrap();
        let mut ctx = test_ctx(project.path());
        ctx.sandbox = Arc::new(
            Sandbox::new(project.path())
                .unwrap()
                .with_docs(docs.path().to_path_buf()),
        );

        let outcome = ProposePlan
            .invoke(json!({"plan": "# Plan\n1. do the thing"}), &ctx)
            .await;

        assert!(!outcome.is_error);
        let path = ctx.sandbox.docs_root().unwrap().join("plan.md");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Plan\n1. do the thing"
        );
        // The location makes the event sink surface the plan as an inline
        // document card.
        assert_eq!(outcome.locations.len(), 1);
        assert!(outcome.locations[0].path.ends_with("plan.md"));
        assert!(outcome.content.contains("## Approved plan"));
        assert!(outcome.content.contains("update_plan"));
    }

    #[tokio::test]
    async fn propose_plan_without_docs_workspace_still_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());

        let outcome = ProposePlan.invoke(json!({"plan": "the steps"}), &ctx).await;

        assert!(!outcome.is_error);
        assert!(outcome.locations.is_empty());
        assert!(outcome.content.contains("the steps"));
        assert!(!outcome.content.contains("saved at"));
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
        assert!(outcome.content.contains("one batched search"));
        assert!(outcome.content.contains("no repeated probes"));
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
