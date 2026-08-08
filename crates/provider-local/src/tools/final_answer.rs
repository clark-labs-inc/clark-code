use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};

pub(crate) const FINAL_ANSWER_TOOL: &str = "final_answer";
pub(crate) const FINAL_ANSWER_DETAILS_KEY: &str = "_agent_final_answer";

pub struct FinalAnswer;

#[async_trait]
impl ToolExecutor for FinalAnswer {
    fn name(&self) -> &str {
        FINAL_ANSWER_TOOL
    }

    fn description(&self) -> &str {
        "Deliver the final user-facing answer and end the run. Call this only after the requested work, checks, and every effect verification are complete."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Complete final answer to show the user."
                }
            },
            "required": ["content"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn terminates_run(&self) -> bool {
        true
    }

    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let content = match arg_str(&args, "content") {
            Ok(content) if !content.trim().is_empty() => content.trim().to_string(),
            _ => return ToolOutcome::error("`content` must be a non-empty final answer"),
        };
        ToolOutcome::ok("Final answer delivered.")
            .with_details(json!({ FINAL_ANSWER_DETAILS_KEY: content }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio_util::sync::CancellationToken;

    use crate::tools::ReadTracker;

    use super::*;

    fn context(root: &std::path::Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(crate::sandbox::Sandbox::new(root).unwrap()),
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

    #[tokio::test]
    async fn preserves_the_complete_structured_answer() {
        let root = tempfile::tempdir().unwrap();
        let outcome = FinalAnswer
            .invoke(
                json!({"content": "  Done.\n\nEvidence: exact.  "}),
                &context(root.path()),
            )
            .await;

        assert!(!outcome.is_error);
        assert_eq!(
            outcome.details[FINAL_ANSWER_DETAILS_KEY],
            "Done.\n\nEvidence: exact."
        );
    }
}
