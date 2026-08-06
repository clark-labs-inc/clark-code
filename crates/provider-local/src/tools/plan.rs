//! Plan Mode tools: `enter_plan_mode` (the Claude-Code-`EnterPlanMode` analog —
//! the agent suggests planning first, the user approves), `propose_plan` (the
//! `ExitPlanMode`/`<proposed_plan>` analog — signals "done
//! researching, please approve") and `update_plan` (the structured plan
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
        "In Plan Mode, call once when the implementation plan is decision-complete. Generate \
        arguments in schema order: an atomic cross-step obligation ledger first, then 3-7 typed \
        execution steps that completely cover it with exact files, observable completion evidence, \
        and easy-to-lose requirements, then the concise Markdown rendering. Audit exact identifiers, \
        repetitions, ordering, negative paths, rollback, and metrics before calling. The turn ends \
        for user review."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "global_reminders": {
                    "type": "array",
                    "description": "Generate first: one to five atomic, non-negotiable cross-step obligations distilled from the user, repository, scout findings, and supplied memory. Preserve exact literals and ordering constraints.",
                    "minItems": 1,
                    "maxItems": 5,
                    "items": {"type": "string"}
                },
                "execution_contract": {
                    "type": "array",
                    "description": "Generate second: ordered execution obligations that completely cover the earlier ledger plus step-local requirements. The runtime assigns immutable step IDs in this array order.",
                    "minItems": 1,
                    "maxItems": 7,
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {
                                "type": "string",
                                "description": "A concise objective for this step."
                            },
                            "files": {
                                "type": "array",
                                "description": "Exact files or directories this step is expected to inspect or change.",
                                "minItems": 1,
                                "maxItems": 8,
                                "items": {"type": "string"}
                            },
                            "done_when": {
                                "type": "array",
                                "description": "One to four observable receipts that prove this step and its mapped obligations are complete; include negative-path or rollback evidence when relevant.",
                                "minItems": 1,
                                "maxItems": 4,
                                "items": {"type": "string"}
                            },
                            "reminders": {
                                "type": "array",
                                "description": "One to four exact, easy-to-lose requirements from the obligation audit, including literals, repetitions, ordering, or metrics.",
                                "minItems": 1,
                                "maxItems": 4,
                                "items": {"type": "string"}
                            }
                        },
                        "required": ["title", "files", "done_when", "reminders"],
                        "additionalProperties": false
                    }
                },
                "plan": {
                    "type": "string",
                    "description": "The concise human-readable Markdown rendering of the already-decided invariants and execution contract."
                }
            },
            "required": ["global_reminders", "execution_contract", "plan"],
            "additionalProperties": false
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
        let contract = match crate::planning::parse_proposal_contract(&args) {
            Ok(contract) => contract,
            Err(error) => return ToolOutcome::error(error),
        };
        let plan = ctx.session.lock().await.planning.next_structured_proposal(
            contract.markdown,
            contract.global_reminders,
            contract.execution_contract,
        );
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
    async fn invoke(&self, _args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let mut session = ctx.session.lock().await;
        if session.planning_research_autoactivate {
            session
                .deferred_tools
                .extend(crate::planning::source_tool_names().map(str::to_string));
        }
        drop(session);
        ToolOutcome::ok(
            "Plan Mode entered. Read-only Project Memory, organization knowledge, and Scout \
             schemas are available when configured. Build a provisional implementation model, \
             challenge its assumptions, retrieve broad evidence before narrowing, and iterate \
             until another source read would not materially change the plan. Then emit one hidden \
             proposed_plan block and end the turn.",
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
                            "plan_step_id": {
                                "type": "string",
                                "description": "Immutable ID from the approved execution contract. Required when an approved contract exists."
                            },
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
        let update = match crate::planning::parse_checklist_update_for_plan(
            &args,
            session.planning.execution_checklist.as_ref(),
            session.planning.proposed_plan.as_ref(),
        ) {
            Ok(update) => update,
            Err(error) => return ToolOutcome::error(error),
        };
        session
            .planning
            .record_checklist_update(update.checklist.clone());
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

    fn proposal_args(markdown: &str) -> Value {
        json!({
            "global_reminders": ["Preserve compatibility"],
            "execution_contract": [{
                "title": "Implement the boundary",
                "files": ["src/lib.rs"],
                "done_when": ["The focused regression test passes"],
                "reminders": ["Keep the existing public API"]
            }],
            "plan": markdown
        })
    }

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
            model_override: None,
        }
    }

    #[test]
    fn propose_plan_is_read_only_and_previews_the_plan_text() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        let tool = ProposePlan;
        assert!(!tool.mutating());
        let args = proposal_args("1. do a thing");
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
            .invoke(proposal_args("# Plan\n1. do the thing"), &ctx)
            .await;

        assert!(!outcome.is_error);
        assert!(!docs.path().join("plan.md").exists());
        assert!(outcome.locations.is_empty());
        assert!(
            matches!(outcome.signals.as_slice(), [ToolSignal::ProposedPlan(plan)]
                if plan.markdown.contains("do the thing")
                    && plan.execution_contract[0].id == "step-1"
                    && plan.global_reminders == ["Preserve compatibility"])
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

        let outcome = ProposePlan.invoke(proposal_args("the steps"), &ctx).await;

        assert!(!outcome.is_error);
        assert!(outcome.locations.is_empty());
        assert!(outcome.content.contains("wait for the user's decision"));
    }

    #[tokio::test]
    async fn enter_plan_mode_is_gated_and_returns_condensed_rules() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        ctx.session.lock().await.planning_research_autoactivate = true;
        assert!(EnterPlanMode.mutating(), "must always pass the gate");

        let outcome = EnterPlanMode.invoke(json!({}), &ctx).await;

        assert!(!outcome.is_error);
        assert!(outcome.content.contains("provisional implementation model"));
        assert!(outcome.content.contains("Read-only"));
        assert!(outcome.content.contains("until another source read"));
        let session = ctx.session.lock().await;
        let activated = &session.deferred_tools;
        assert!(activated.contains("memory_recall"));
        assert!(activated.contains("organization_knowledge"));
        assert!(activated.contains("scout_enterprise_query"));
        assert!(!activated.contains("memory"));
        assert!(!activated.contains("scout_enterprise"));
    }

    #[tokio::test]
    async fn enter_plan_mode_respects_control_run_source_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());

        let outcome = EnterPlanMode.invoke(json!({}), &ctx).await;

        assert!(!outcome.is_error);
        assert!(ctx.session.lock().await.deferred_tools.is_empty());
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

    #[tokio::test]
    async fn approved_contract_requires_stable_ids_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path());
        {
            let mut session = ctx.session.lock().await;
            let mut proposal = session.planning.next_structured_proposal(
                "1. Implement".into(),
                vec!["Preserve compatibility".into()],
                vec![agent_core::domain::PlanExecutionStep {
                    id: String::new(),
                    title: "Implement the boundary".into(),
                    files: vec!["src/lib.rs".into()],
                    done_when: vec!["The regression test passes".into()],
                    reminders: vec!["Keep the public API".into()],
                }],
            );
            proposal.status = agent_core::domain::ProposedPlanStatus::Approved;
            session.planning.proposed_plan = Some(proposal);
        }

        let missing_id = UpdatePlan
            .invoke(
                json!({"plan": [{"step": "Implement the boundary", "status": "in_progress"}]}),
                &ctx,
            )
            .await;
        assert!(missing_id.is_error);
        assert!(missing_id.content.contains("plan_step_id"));

        let valid = UpdatePlan
            .invoke(
                json!({"plan": [{
                    "plan_step_id": "step-1",
                    "step": "Implement the boundary",
                    "status": "in_progress"
                }]}),
                &ctx,
            )
            .await;
        assert!(!valid.is_error);
    }
}
