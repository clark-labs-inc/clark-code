use std::collections::HashMap;

use agent_core::domain::GoalState;
pub(crate) use agent_core::domain::GoalStatus;
use agent_core::ids::{PermissionRequestId, RunId};
use agent_loop::AgentMessage;
use tokio::sync::oneshot;

use crate::project_settings::HooksConfig;
use crate::tools::PermissionMode;

/// Per-session conversation state that persists across turns.
#[derive(Default)]
pub(crate) struct SessionState {
    pub system_prompt: String,
    pub transcript: Vec<AgentMessage>,
    /// Per-tool permission policy; "always allow/reject" mutate it in place.
    pub policy: HashMap<String, PermissionMode>,
    /// Shell-command prefixes the user always allows (skip the gate). Honored
    /// only for Safe/Caution commands, so a trusted prefix can't carry a
    /// destructive suffix past the gate. Union of the global (UI-driven)
    /// allowlist and the project's `.agent/settings.json` `permissions.allow`.
    pub allow_commands: Vec<String>,
    /// Shell-command prefixes that are always refused. Union of the global
    /// denylist and the project's `.agent/settings.json` `permissions.deny`.
    pub deny_commands: Vec<String>,
    /// Host-owned boundaries that remain refusals under every permission mode.
    pub hard_constraints: Vec<String>,
    /// Collaboration-mode and execution-checklist state. Kept independent
    /// from permission policy and standing goals.
    pub planning: crate::planning::PlanningState,
    /// Deferred tool schemas activated for this conversation by `tool_search`.
    pub deferred_tools: std::collections::HashSet<String>,
    /// Production Plan Mode exposes registered read-only evidence sources
    /// immediately. Controlled evaluations can disable that exposure.
    pub planning_research_autoactivate: bool,
    /// Output style: a key into `crate::prompt::OUTPUT_STYLES`, prepended to
    /// each turn's text like the plan-mode reminder. Empty string = default.
    pub output_style: String,
    /// `PreToolUse`/`PostToolUse` hooks from the project's
    /// `.agent/settings.json`, read once at `new_session`.
    pub hooks: HooksConfig,
    /// Durable and externally visible mutations that require an independent
    /// canonical read-back before the agent may finish.
    pub effects: crate::effects::EffectLedger,
    /// The project's configured check/lint/typecheck command (§7
    /// `check_diagnostics`), from `.agent/settings.json` or a per-project
    /// Settings override.
    pub check_command: Option<String>,
    /// First `check_diagnostics` call's output lines this session — later
    /// calls diff against this and report only new lines.
    pub diagnostics_baseline: Option<Vec<String>>,
    /// Live steering queue for the ACTIVE run, when one is in flight: a user
    /// message sent mid-run is injected between tool batches instead of
    /// waiting for the run to end. Set by the engine at run start, cleared
    /// at run end.
    pub steering: Option<std::sync::Arc<crate::engine::EngineSteering>>,
    /// Root execution ledger for the active run. A normal single-agent turn
    /// owns `/root`; optional read-only children attach beneath this identity.
    pub active_execution: Option<crate::root_execution::RootExecutionTrace>,
    /// Host-owned receipts for an active deep Security workflow. Accepted
    /// read-only orchestrations are recorded here before the scan contract can
    /// claim independent passes or saturation.
    pub security_deep: crate::security::SecurityDeepLedger,
    /// Host-issued PoC execution receipts for the active Security workflow.
    /// Model-authored scan JSON can reference these ids, but cannot mint them.
    pub security_poc: crate::security::SecurityPocLedger,
    /// Standing objective the session pursues autonomously: while `Active`, the
    /// engine keeps continuing the run
    /// with goal-continuation turns after each clean completion, until the
    /// model proves the goal complete or gets genuinely blocked.
    pub goal: Option<SessionGoal>,
}

/// One session goal. Created by the model's `create_goal` tool (only on an
/// explicit user request); status transitions to `Complete`/`Blocked` come
/// from `update_goal`, and to `Blocked` from the engine after repeated
/// unrecoverable failures.
#[derive(Clone, Debug)]
pub(crate) struct SessionGoal {
    pub id: String,
    pub objective: String,
    pub status: GoalStatus,
    /// Tokens (input+output) attributed to the goal so far.
    pub tokens_used: u64,
    /// Wall-clock seconds spent in goal-driven turns.
    pub time_used_seconds: u64,
    /// Goal-continuation turns launched by the engine so far.
    pub continuations: u32,
    pub updated_at_ms: u64,
    pub blocker_reason: Option<String>,
    /// Runtime-only audit used to enforce the three-consecutive-turn blocker
    /// rule. It intentionally resets after reopening or explicit user resume.
    pub blocker_observations: u8,
    pub last_blocker_continuation: Option<u32>,
}

impl SessionGoal {
    pub fn from_state(state: GoalState) -> Self {
        Self {
            id: state.id,
            objective: state.objective,
            status: state.status,
            tokens_used: state.tokens_used,
            time_used_seconds: state.time_used_seconds,
            continuations: state.continuations,
            updated_at_ms: state.updated_at_ms,
            blocker_reason: state.blocker_reason,
            blocker_observations: 0,
            last_blocker_continuation: None,
        }
    }

    pub fn state(&self, run: Option<&RunId>) -> GoalState {
        GoalState {
            id: self.id.clone(),
            objective: self.objective.clone(),
            status: self.status,
            run: run.cloned(),
            tokens_used: self.tokens_used,
            time_used_seconds: self.time_used_seconds,
            continuations: self.continuations,
            updated_at_ms: self.updated_at_ms,
            blocker_reason: self.blocker_reason.clone(),
        }
    }

    pub fn touch(&mut self) {
        self.updated_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
    }

    pub fn status_label(&self) -> &'static str {
        match self {
            Self {
                status: GoalStatus::Active,
                ..
            } => "active",
            Self {
                status: GoalStatus::Blocked,
                ..
            } => "blocked",
            Self {
                status: GoalStatus::Complete,
                ..
            } => "complete",
        }
    }
}

/// Live control surface for the current run, reachable from respond/cancel.
#[derive(Default)]
pub(crate) struct RunControl {
    pending: Option<Pending>,
}

struct Pending {
    id: PermissionRequestId,
    responder: oneshot::Sender<Resolution>,
}

/// A resolved permission prompt: the user's decision plus any free-text they
/// attached (today: "keep planning" feedback, threaded back to the model as
/// the rejection reason so the same run can revise its plan).
#[derive(Debug)]
pub(crate) struct Resolution {
    pub decision: Decision,
    pub feedback: Option<String>,
}

impl From<Decision> for Resolution {
    fn from(decision: Decision) -> Self {
        Self {
            decision,
            feedback: None,
        }
    }
}

impl RunControl {
    /// Register the one permission prompt currently presented to the user.
    ///
    /// Refuse to replace an existing responder: dropping that sender wakes its
    /// waiter as if the user cancelled, which can silently abort an otherwise
    /// healthy parallel tool batch. Permission acquisition is serialized by
    /// `PermissionGate`; this check is the defensive backstop that makes any
    /// coordination regression explicit instead of lossy.
    pub fn arm(
        &mut self,
        id: PermissionRequestId,
        responder: oneshot::Sender<Resolution>,
    ) -> Result<(), PermissionRequestId> {
        if let Some(pending) = &self.pending {
            return Err(pending.id.clone());
        }
        self.pending = Some(Pending { id, responder });
        Ok(())
    }

    /// Deliver a user's answer to the in-flight permission request. Returns
    /// `true` if a request was actually waiting.
    pub fn resolve(&mut self, id: &PermissionRequestId, resolution: Resolution) -> bool {
        match self.pending.take() {
            Some(p) if &p.id == id || id.as_str().is_empty() => {
                let _ = p.responder.send(resolution);
                true
            }
            other => {
                self.pending = other;
                false
            }
        }
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }

    /// Clear only the request owned by this waiter. A stale waiter must never
    /// discard a newer prompt that reused the same session control surface.
    pub fn clear_if(&mut self, id: &PermissionRequestId) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| &pending.id == id)
        {
            self.pending = None;
            true
        } else {
            false
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
}

/// How the user resolved a permission prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

impl Decision {
    pub fn from_option(option: &str) -> Decision {
        match option {
            "allow_always" => Decision::AllowAlways,
            "reject_always" => Decision::RejectAlways,
            "reject_once" | "reject" | "deny" => Decision::RejectOnce,
            _ => Decision::AllowOnce,
        }
    }

    pub fn approved(self) -> bool {
        matches!(self, Decision::AllowOnce | Decision::AllowAlways)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_maps_from_option_ids() {
        assert_eq!(Decision::from_option("allow_once"), Decision::AllowOnce);
        assert_eq!(Decision::from_option("allow_always"), Decision::AllowAlways);
        assert_eq!(Decision::from_option("reject_once"), Decision::RejectOnce);
        assert_eq!(
            Decision::from_option("reject_always"),
            Decision::RejectAlways
        );
        assert_eq!(Decision::from_option("deny"), Decision::RejectOnce);
        // Plan-approval options are approvals regardless of the follow-up mode.
    }

    #[tokio::test]
    async fn run_control_resolves_matching_request() {
        let mut control = RunControl::default();
        let id = PermissionRequestId::new("perm-1");
        let (tx, rx) = oneshot::channel();
        control.arm(id.clone(), tx).unwrap();
        assert!(control.resolve(&id, Decision::AllowOnce.into()));
        assert_eq!(rx.await.unwrap().decision, Decision::AllowOnce);
    }

    #[tokio::test]
    async fn run_control_resolution_carries_feedback() {
        let mut control = RunControl::default();
        let id = PermissionRequestId::new("perm-2");
        let (tx, rx) = oneshot::channel();
        control.arm(id.clone(), tx).unwrap();
        assert!(control.resolve(
            &id,
            Resolution {
                decision: Decision::RejectOnce,
                feedback: Some("add tests".to_string()),
            }
        ));
        let resolution = rx.await.unwrap();
        assert_eq!(resolution.decision, Decision::RejectOnce);
        assert_eq!(resolution.feedback.as_deref(), Some("add tests"));
    }

    #[tokio::test]
    async fn run_control_never_overwrites_an_unresolved_permission() {
        let mut control = RunControl::default();
        let first_id = PermissionRequestId::new("perm-first");
        let second_id = PermissionRequestId::new("perm-second");
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();

        control.arm(first_id.clone(), first_tx).unwrap();
        assert_eq!(
            control.arm(second_id, second_tx),
            Err(first_id.clone()),
            "the occupied slot must be reported instead of replacing its sender"
        );
        assert!(control.resolve(&first_id, Decision::AllowOnce.into()));
        assert_eq!(first_rx.await.unwrap().decision, Decision::AllowOnce);
        assert!(
            second_rx.await.is_err(),
            "the rejected responder is not armed"
        );
    }
}
