//! Generic lifecycle for durable or externally visible tool effects.
//!
//! A process exit only proves that the invocation returned successfully. It
//! does not prove that a remote or durable resource contains what the user
//! requested. Tools declare an [`EffectIntent`], successful calls become
//! pending [`EffectReceipt`]s, and the model resolves them through an
//! independent observation before the agent is allowed to stop.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use agent_core::ids::RunId;
use clark_agent::{
    plugin::{ToolGate, ToolGateContext},
    AgentMessage, FollowUpSource, Plugin, PluginCapabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::loop_state::SessionState;
use crate::tools::ToolOutcome;

pub(crate) const EFFECT_DETAILS_KEY: &str = "_clark_effect_receipt";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EffectIntent {
    pub(crate) action: EffectAction,
    pub(crate) target_hint: Option<String>,
    pub(crate) description: String,
}

impl EffectIntent {
    pub(crate) fn opaque_external(description: impl Into<String>) -> Self {
        Self {
            action: EffectAction::Mutate,
            target_hint: None,
            description: description.into(),
        }
    }

    pub(crate) fn declared_external(
        action: EffectAction,
        target_hint: Option<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            action,
            target_hint,
            description: description.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EffectAction {
    Create,
    Update,
    Publish,
    Send,
    Delete,
    Mutate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EffectVerification {
    Pending,
    Verified,
    Unverifiable,
    Mismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EffectReceipt {
    pub(crate) id: String,
    /// Run that created this verification obligation. Receipts remain in the
    /// session ledger for audit and explicit later verification, but only the
    /// originating run may be held open by them.
    pub(crate) run: RunId,
    pub(crate) tool_name: String,
    pub(crate) action: EffectAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_hint: Option<String>,
    pub(crate) description: String,
    pub(crate) verification: EffectVerification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) evidence: Option<String>,
    #[serde(default)]
    pub(crate) completion_reminders: u8,
}

#[derive(Default)]
pub(crate) struct EffectLedger {
    receipts: BTreeMap<String, EffectReceipt>,
}

impl EffectLedger {
    pub(crate) fn register(
        &mut self,
        run: RunId,
        id: impl Into<String>,
        tool_name: impl Into<String>,
        intent: EffectIntent,
    ) -> EffectReceipt {
        let receipt = EffectReceipt {
            id: id.into(),
            run,
            tool_name: tool_name.into(),
            action: intent.action,
            target_hint: intent.target_hint,
            description: intent.description,
            verification: EffectVerification::Pending,
            evidence: None,
            completion_reminders: 0,
        };
        self.receipts.insert(receipt.id.clone(), receipt.clone());
        receipt
    }

    pub(crate) fn verify(
        &mut self,
        id: &str,
        verification: EffectVerification,
        evidence: String,
    ) -> Result<EffectReceipt, String> {
        if verification == EffectVerification::Pending {
            return Err("verification status cannot be pending".to_string());
        }
        let receipt = self
            .receipts
            .get_mut(id)
            .ok_or_else(|| format!("unknown effect receipt `{id}`"))?;
        receipt.verification = verification;
        receipt.evidence = Some(evidence);
        Ok(receipt.clone())
    }

    pub(crate) fn completion_prompt(&mut self, run: &RunId) -> Option<String> {
        let unresolved = self
            .receipts
            .values_mut()
            .filter(|receipt| {
                &receipt.run == run
                    && matches!(
                        receipt.verification,
                        EffectVerification::Pending | EffectVerification::Mismatch
                    )
            })
            .map(|receipt| {
                receipt.completion_reminders = receipt.completion_reminders.saturating_add(1);
                format!(
                    "- `{}` from `{}`: {} ({:?})",
                    receipt.id, receipt.tool_name, receipt.description, receipt.verification
                )
            })
            .collect::<Vec<_>>();
        if unresolved.is_empty() {
            return None;
        }
        Some(format!(
            "Clark cannot finish yet because these durable or externally visible effects have not \
             been independently verified:\n{}\nUse an appropriate read-only tool or command to inspect \
             each target's canonical state, then call `verify_effect`. When inspection uses `bash`, \
             set `effect` to `none` so the observation is not recorded as another mutation. Command \
             success, a created \
             URL, or the text you intended to send is not verification. If canonical read-back is \
             genuinely unavailable, record `unverifiable` with the concrete reason. Repair any \
             mismatch before giving the final answer, and report the verification evidence in that \
             answer.",
            unresolved.join("\n")
        ))
    }

    pub(crate) fn unresolved_diagnostics(&self, run: &RunId) -> Vec<String> {
        self.receipts
            .values()
            .filter(|receipt| {
                &receipt.run == run
                    && matches!(
                        receipt.verification,
                        EffectVerification::Pending | EffectVerification::Mismatch
                    )
            })
            .map(|receipt| {
                format!(
                    "`{}` from `{}`: {} ({:?})",
                    receipt.id, receipt.tool_name, receipt.description, receipt.verification
                )
            })
            .collect()
    }

    pub(crate) fn has_unresolved(&self) -> bool {
        self.receipts.values().any(|receipt| {
            matches!(
                receipt.verification,
                EffectVerification::Pending | EffectVerification::Mismatch
            )
        })
    }

    fn requires_verification_only_turn(&self, run: &RunId) -> bool {
        let unresolved = self.receipts.values().filter(|receipt| {
            &receipt.run == run
                && matches!(
                    receipt.verification,
                    EffectVerification::Pending | EffectVerification::Mismatch
                )
        });
        let mut count = 0usize;
        let ready = unresolved.fold(true, |ready, receipt| {
            count = count.saturating_add(1);
            ready && receipt.completion_reminders >= 2
        });
        count > 0 && ready
    }

    #[cfg(test)]
    pub(crate) fn get(&self, id: &str) -> Option<&EffectReceipt> {
        self.receipts.get(id)
    }
}

pub(crate) fn attach_pending_receipt(outcome: &mut ToolOutcome, receipt: &EffectReceipt) {
    if !outcome.details.is_object() {
        outcome.details = json!({});
    }
    outcome.details[EFFECT_DETAILS_KEY] = serde_json::to_value(receipt).unwrap_or(Value::Null);
    outcome.content.push_str(&format!(
        "\n\n[verification required]\nThis call may have changed durable or externally visible \
         state. Its effect receipt is `{}`. Independently read the canonical result, then call \
         `verify_effect`; do not treat this successful tool result as proof of the final state.",
        receipt.id
    ));
}

pub(crate) struct EffectCompletionGuard {
    session: Arc<Mutex<SessionState>>,
    run: RunId,
}

impl EffectCompletionGuard {
    pub(crate) fn new(session: Arc<Mutex<SessionState>>, run: RunId) -> Self {
        Self { session, run }
    }
}

impl Plugin for EffectCompletionGuard {
    fn name(&self) -> &'static str {
        "external_effect_completion"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities::follow_up().with_tool_gate()
    }
}

#[async_trait::async_trait]
impl ToolGate for EffectCompletionGuard {
    async fn next_turn_tool_allowlist(&self, _ctx: ToolGateContext<'_>) -> Option<HashSet<String>> {
        self.session
            .lock()
            .await
            .effects
            .requires_verification_only_turn(&self.run)
            .then(|| ["verify_effect".to_string()].into_iter().collect())
    }

    async fn denial_reason(&self, tool_name: &str, _ctx: ToolGateContext<'_>) -> Option<String> {
        if tool_name == "verify_effect" {
            return None;
        }
        let session = self.session.lock().await;
        let unresolved = session.effects.unresolved_diagnostics(&self.run);
        if tool_name == crate::tools::final_answer::FINAL_ANSWER_TOOL && !unresolved.is_empty() {
            return Some(format!(
                "Clark cannot publish the final answer while these effects remain unresolved:\n- {}\nIndependently inspect each canonical target, then call `verify_effect` with `verified`, `mismatch`, or `unverifiable` and concrete evidence.",
                unresolved.join("\n- ")
            ));
        }
        session
            .effects
            .requires_verification_only_turn(&self.run)
            .then(|| {
                "Canonical evidence is already in context; resolve the pending effect with `verify_effect` before using another tool.".to_string()
            })
    }

    fn conflict_priority(&self) -> i32 {
        100
    }

    fn suppresses_advisory_gates(&self, _ctx: ToolGateContext<'_>) -> bool {
        true
    }
}

#[async_trait::async_trait]
impl FollowUpSource for EffectCompletionGuard {
    async fn next_follow_up_messages(&self) -> Vec<AgentMessage> {
        let prompt = self
            .session
            .lock()
            .await
            .effects
            .completion_prompt(&self.run);
        prompt
            .map(|content| {
                vec![AgentMessage::System {
                    content,
                    timestamp: None,
                }]
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str) -> RunId {
        RunId::new(id)
    }

    #[test]
    fn successful_execution_stays_pending_until_independent_verification() {
        let mut ledger = EffectLedger::default();
        ledger.register(
            run("run-1"),
            "call-1",
            "publisher",
            EffectIntent::opaque_external("published a user-facing resource"),
        );

        let prompt = ledger
            .completion_prompt(&run("run-1"))
            .expect("completion must block");
        assert!(prompt.contains("call-1"));
        assert!(prompt.contains("canonical state"));
        assert!(prompt.contains("`effect` to `none`"));

        ledger
            .verify(
                "call-1",
                EffectVerification::Verified,
                "Read the canonical resource and confirmed its complete body".into(),
            )
            .unwrap();
        assert!(ledger.completion_prompt(&run("run-1")).is_none());
    }

    #[test]
    fn one_character_canonical_result_is_a_mismatch_and_keeps_gate_closed() {
        let mut ledger = EffectLedger::default();
        ledger.register(
            run("run-1"),
            "call-2",
            "publisher",
            EffectIntent::opaque_external("published a user-facing resource"),
        );
        ledger
            .verify(
                "call-2",
                EffectVerification::Mismatch,
                "Canonical read-back was `-`, not the requested explanation and tests".into(),
            )
            .unwrap();

        let prompt = ledger
            .completion_prompt(&run("run-1"))
            .expect("mismatch must block");
        assert!(prompt.contains("Mismatch"));
        assert_eq!(
            ledger.get("call-2").unwrap().verification,
            EffectVerification::Mismatch
        );
    }

    #[test]
    fn explicit_unverifiable_receipt_allows_honest_completion() {
        let mut ledger = EffectLedger::default();
        ledger.register(
            run("run-1"),
            "call-3",
            "publisher",
            EffectIntent::opaque_external("sent a remote request"),
        );
        ledger
            .verify(
                "call-3",
                EffectVerification::Unverifiable,
                "Provider exposes no read endpoint; only request id req-7 is available".into(),
            )
            .unwrap();
        assert!(ledger.completion_prompt(&run("run-1")).is_none());
        assert!(!ledger.has_unresolved());
    }

    #[test]
    fn unresolved_state_tracks_pending_and_mismatched_receipts() {
        let mut ledger = EffectLedger::default();
        ledger.register(
            run("run-1"),
            "call-1",
            "publisher",
            EffectIntent::opaque_external("published a resource"),
        );
        assert!(ledger.has_unresolved());

        ledger
            .verify(
                "call-1",
                EffectVerification::Verified,
                "Canonical read-back matched".into(),
            )
            .unwrap();
        assert!(!ledger.has_unresolved());
    }

    #[test]
    fn repeated_completion_attempts_narrow_the_next_turn_to_verification() {
        let mut ledger = EffectLedger::default();
        ledger.register(
            run("run-1"),
            "call-1",
            "publisher",
            EffectIntent::opaque_external("published a resource"),
        );

        assert!(!ledger.requires_verification_only_turn(&run("run-1")));
        assert!(ledger.completion_prompt(&run("run-1")).is_some());
        assert!(!ledger.requires_verification_only_turn(&run("run-1")));
        assert!(ledger.completion_prompt(&run("run-1")).is_some());
        assert!(ledger.requires_verification_only_turn(&run("run-1")));
    }

    #[tokio::test]
    async fn fake_publisher_success_then_one_character_readback_forces_follow_up() {
        let session = Arc::new(Mutex::new(SessionState::default()));
        let receipt = session.lock().await.effects.register(
            run("run-1"),
            "publish-1",
            "fake_publisher",
            EffectIntent::declared_external(
                EffectAction::Publish,
                Some("resource://example".into()),
                "published a user-facing resource",
            ),
        );
        let mut successful_execution = ToolOutcome::ok("exit_code: 0\nresource://example");
        attach_pending_receipt(&mut successful_execution, &receipt);
        assert!(successful_execution
            .content
            .contains("verification required"));

        session
            .lock()
            .await
            .effects
            .verify(
                "publish-1",
                EffectVerification::Mismatch,
                "Canonical read-back contained only `-`".into(),
            )
            .unwrap();

        let follow_up = EffectCompletionGuard::new(session, run("run-1"))
            .next_follow_up_messages()
            .await;
        assert_eq!(follow_up.len(), 1);
        let AgentMessage::System { content, .. } = &follow_up[0] else {
            panic!("completion gate must inject system guidance");
        };
        assert!(content.contains("cannot finish yet"));
        assert!(content.contains("Repair any mismatch"));
    }

    #[tokio::test]
    async fn unresolved_receipts_only_gate_their_originating_run() {
        let session = Arc::new(Mutex::new(SessionState::default()));
        session.lock().await.effects.register(
            run("run-1"),
            "publish-1",
            "fake_publisher",
            EffectIntent::opaque_external("published a user-facing resource"),
        );

        let original_follow_up = EffectCompletionGuard::new(session.clone(), run("run-1"))
            .next_follow_up_messages()
            .await;
        let later_follow_up = EffectCompletionGuard::new(session.clone(), run("run-2"))
            .next_follow_up_messages()
            .await;

        assert_eq!(original_follow_up.len(), 1);
        assert!(later_follow_up.is_empty());
        assert_eq!(
            session
                .lock()
                .await
                .effects
                .unresolved_diagnostics(&run("run-1"))
                .len(),
            1
        );
        assert_eq!(
            session
                .lock()
                .await
                .effects
                .unresolved_diagnostics(&run("run-2"))
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn final_answer_is_denied_with_exact_pending_receipt_diagnostics() {
        let session = Arc::new(Mutex::new(SessionState::default()));
        session.lock().await.effects.register(
            run("run-1"),
            "publish-7",
            "fake_publisher",
            EffectIntent::declared_external(
                EffectAction::Publish,
                Some("resource://example".into()),
                "published the requested resource",
            ),
        );
        let guard = EffectCompletionGuard::new(session, run("run-1"));
        let reason = guard
            .denial_reason(
                crate::tools::final_answer::FINAL_ANSWER_TOOL,
                ToolGateContext {
                    iteration: 1,
                    messages: &[],
                    conversation_id: Some("conversation"),
                    available_tool_names: &["final_answer", "verify_effect"],
                },
            )
            .await
            .expect("pending receipt must block final publication");

        assert!(reason.contains("publish-7"));
        assert!(reason.contains("fake_publisher"));
        assert!(reason.contains("published the requested resource"));
        assert!(reason.contains("verify_effect"));
    }
}
