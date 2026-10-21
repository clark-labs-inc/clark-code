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
use agent_loop::{
    plugin::{ToolGate, ToolGateContext},
    AgentMessage, FollowUpSource, Plugin, PluginCapabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::loop_state::SessionState;
use crate::tools::ToolOutcome;

pub(crate) const EFFECT_DETAILS_KEY: &str = "_agent_effect_receipt";

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
            "Clark Code cannot finish yet because these durable or externally visible effects have not \
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
    async fn next_turn_tool_allowlist(&self, ctx: ToolGateContext<'_>) -> Option<HashSet<String>> {
        let session = self.session.lock().await;
        if session.effects.requires_verification_only_turn(&self.run) {
            return Some(["verify_effect".to_string()].into_iter().collect());
        }
        if session.effects.unresolved_diagnostics(&self.run).is_empty() {
            return None;
        }
        Some(
            ctx.available_tool_names
                .iter()
                .filter(|name| **name != "deliver_files")
                .map(|name| (*name).to_string())
                .collect(),
        )
    }

    async fn denial_reason(&self, tool_name: &str, _ctx: ToolGateContext<'_>) -> Option<String> {
        if tool_name == "verify_effect" {
            return None;
        }
        let session = self.session.lock().await;
        let unresolved = session.effects.unresolved_diagnostics(&self.run);
        if tool_name == "deliver_files" && !unresolved.is_empty() {
            return Some(format!(
                "Clark Code cannot deliver generated files while these effects remain unresolved:\n- {}\nIndependently inspect each canonical target, then call `verify_effect` with `verified`, `mismatch`, or `unverifiable` and concrete evidence.",
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
mod tests;
