use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::effects::{EffectVerification, EFFECT_DETAILS_KEY};

use super::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};

pub struct VerifyEffect;

#[async_trait]
impl ToolExecutor for VerifyEffect {
    fn name(&self) -> &str {
        "verify_effect"
    }

    fn description(&self) -> &str {
        "Record the result of independently reading a durable or externally visible effect. Call \
         this only after observing canonical state through a separate read-only tool or command; \
         successful creation output is not verification. A mismatch remains unresolved until the \
         resource is repaired and read again."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "effect_id": {"type": "string", "description": "Receipt id returned by the mutating tool."},
                "status": {
                    "type": "string",
                    "enum": ["verified", "mismatch", "unverifiable"],
                    "description": "Whether canonical state matches, differs, or cannot be read back."
                },
                "evidence": {
                    "type": "string",
                    "description": "Concrete read-back evidence or the concrete reason canonical verification is unavailable."
                },
                "expected": {"type": "string", "description": "Optional exact scalar expected value for deterministic byte-for-byte comparison. Omit both comparison fields when evidence summarizes multiple facts."},
                "observed": {"type": "string", "description": "Optional exact scalar copied from canonical state. Must be byte-identical to expected for verified, different for mismatch. Omit both fields for summary evidence."}
            },
            "required": ["effect_id", "status", "evidence"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let effect_id = match arg_str(&args, "effect_id") {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        let status = match arg_str(&args, "status") {
            Ok(value) if value == "verified" => EffectVerification::Verified,
            Ok(value) if value == "mismatch" => EffectVerification::Mismatch,
            Ok(value) if value == "unverifiable" => EffectVerification::Unverifiable,
            Ok(other) => return ToolOutcome::error(format!("unsupported status `{other}`")),
            Err(error) => return ToolOutcome::error(error),
        };
        let evidence = match arg_str(&args, "evidence") {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            Ok(_) => return ToolOutcome::error("evidence must not be empty"),
            Err(error) => return ToolOutcome::error(error),
        };
        let expected = args.get("expected").and_then(Value::as_str);
        let observed = args.get("observed").and_then(Value::as_str);
        if expected.is_none() && evidence.split_whitespace().count() < 2 {
            return ToolOutcome::error(
                "evidence must be a concrete observation or reason, not a placeholder",
            );
        }
        match (expected, observed, status) {
            (Some(_), None, _) | (None, Some(_), _) => {
                return ToolOutcome::error(
                    "expected and observed must be supplied together, or both omitted for summary evidence",
                )
            }
            (Some(expected), Some(observed), EffectVerification::Verified)
                if expected != observed =>
            {
                return ToolOutcome::error(
                    "cannot record verified: expected and observed are not byte-identical; if they are human-readable summaries, omit both fields and provide concrete evidence",
                )
            }
            (Some(expected), Some(observed), EffectVerification::Mismatch)
                if expected == observed =>
            {
                return ToolOutcome::error(
                    "cannot record mismatch: expected and observed are byte-identical",
                )
            }
            (Some(_), Some(_), EffectVerification::Unverifiable) => {
                return ToolOutcome::error(
                    "unverifiable cannot include canonical comparison values; provide the concrete reason read-back is unavailable",
                )
            }
            _ => {}
        }

        let receipt = {
            let mut session = ctx.session.lock().await;
            match session.effects.verify(&effect_id, status, evidence) {
                Ok(receipt) => receipt,
                Err(error) => return ToolOutcome::error(error),
            }
        };
        let details = json!({ (EFFECT_DETAILS_KEY): receipt });
        match receipt.verification {
            EffectVerification::Verified => ToolOutcome::ok(format!(
                "Effect `{}` is verified from canonical read-back. Include this evidence in the final answer: {}",
                receipt.id,
                receipt.evidence.as_deref().unwrap_or_default()
            ))
            .with_details(details),
            EffectVerification::Unverifiable => ToolOutcome::ok(format!(
                "Effect `{}` is explicitly unverifiable. State this limitation and reason in the final answer: {}",
                receipt.id,
                receipt.evidence.as_deref().unwrap_or_default()
            ))
            .with_details(details),
            EffectVerification::Mismatch => ToolOutcome::error(format!(
                "Canonical state for effect `{}` does not match. Repair the resource, read it again, then call `verify_effect` with the same id.",
                receipt.id
            ))
            .with_details(details),
            EffectVerification::Pending => unreachable!(),
        }
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
    async fn malformed_exact_comparison_does_not_create_a_false_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = context(dir.path());
        ctx.session.lock().await.effects.register(
            agent_core::ids::RunId::new("run-1"),
            "effect-1",
            "fake_publisher",
            crate::effects::EffectIntent::opaque_external("published resource"),
        );

        let outcome = VerifyEffect
            .invoke(
                json!({
                    "effect_id": "effect-1",
                    "status": "verified",
                    "evidence": "Read canonical resource body",
                    "expected": "Detailed explanation and test results",
                    "observed": "-"
                }),
                &ctx,
            )
            .await;

        assert!(outcome.is_error);
        assert!(outcome.content.contains("not byte-identical"));
        assert_eq!(
            ctx.session
                .lock()
                .await
                .effects
                .get("effect-1")
                .unwrap()
                .verification,
            EffectVerification::Pending
        );
    }

    #[tokio::test]
    async fn summary_evidence_can_verify_without_fake_exact_values() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = context(dir.path());
        ctx.session.lock().await.effects.register(
            agent_core::ids::RunId::new("run-1"),
            "effect-1",
            "fake_publisher",
            crate::effects::EffectIntent::opaque_external("published resource"),
        );

        let outcome = VerifyEffect
            .invoke(
                json!({
                    "effect_id": "effect-1",
                    "status": "verified",
                    "evidence": "Canonical refs, branch, and working tree all match the requested state."
                }),
                &ctx,
            )
            .await;

        assert!(!outcome.is_error, "{}", outcome.content);
        assert_eq!(
            ctx.session
                .lock()
                .await
                .effects
                .get("effect-1")
                .unwrap()
                .verification,
            EffectVerification::Verified
        );
    }

    #[tokio::test]
    async fn one_character_evidence_is_rejected_as_a_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = context(dir.path());
        ctx.session.lock().await.effects.register(
            agent_core::ids::RunId::new("run-1"),
            "effect-2",
            "fake_publisher",
            crate::effects::EffectIntent::opaque_external("published resource"),
        );

        let outcome = VerifyEffect
            .invoke(
                json!({
                    "effect_id": "effect-2",
                    "status": "verified",
                    "evidence": "-"
                }),
                &ctx,
            )
            .await;

        assert!(outcome.is_error);
        assert!(outcome.content.contains("not a placeholder"));
        assert_eq!(
            ctx.session
                .lock()
                .await
                .effects
                .get("effect-2")
                .unwrap()
                .verification,
            EffectVerification::Pending
        );
    }
}
