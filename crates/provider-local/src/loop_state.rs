use std::collections::HashMap;

use agent_core::ids::PermissionRequestId;
use clark_agent::AgentMessage;
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
    /// allowlist and the project's `.clark/settings.json` `permissions.allow`.
    pub allow_commands: Vec<String>,
    /// Shell-command prefixes that are always refused. Union of the global
    /// denylist and the project's `.clark/settings.json` `permissions.deny`.
    pub deny_commands: Vec<String>,
    /// Plan Mode: while true, every mutating tool except `propose_plan` is
    /// denied by the [`crate::permissions::PermissionGate`].
    pub plan_mode: bool,
    /// One-shot: set when plan mode ends (plan approved, or the user switched
    /// modes) so the next turn opens with a short "plan mode is off" note and
    /// the model stops treating the session as read-only.
    pub plan_exited: bool,
    /// Output style: a key into `crate::prompt::OUTPUT_STYLES`, prepended to
    /// each turn's text like the plan-mode reminder. Empty string = default.
    pub output_style: String,
    /// `PreToolUse`/`PostToolUse` hooks from the project's
    /// `.clark/settings.json`, read once at `new_session`.
    pub hooks: HooksConfig,
    /// The project's configured check/lint/typecheck command (§7
    /// `check_diagnostics`), from `.clark/settings.json` or a per-project
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
    /// Standing objective the session pursues autonomously (the Codex
    /// `/goal` analog): while `Active`, the engine keeps continuing the run
    /// with goal-continuation turns after each clean completion, until the
    /// model proves the goal complete, gets genuinely blocked, or the token
    /// budget runs out.
    pub goal: Option<SessionGoal>,
}

/// One session goal. Created by the model's `create_goal` tool (only on an
/// explicit user request); status transitions to `Complete`/`Blocked` come
/// from `update_goal`, and to `Blocked`/`BudgetLimited` from the engine
/// (errors, iteration caps, budget crossings).
#[derive(Clone, Debug)]
pub(crate) struct SessionGoal {
    pub objective: String,
    pub status: GoalStatus,
    /// Optional cap on tokens (input+output) spent pursuing the goal.
    pub token_budget: Option<u64>,
    /// Tokens (input+output) attributed to the goal so far.
    pub tokens_used: u64,
    /// Wall-clock seconds spent in goal-driven turns.
    pub time_used_seconds: u64,
    /// Goal-continuation turns launched by the engine so far.
    pub continuations: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GoalStatus {
    Active,
    /// The engine or the model stopped: repeated blocker, terminal error, or
    /// the continuation cap. Needs the user to intervene.
    Blocked,
    /// The token budget is exhausted; one wrap-up turn runs, then the run
    /// stops.
    BudgetLimited,
    Complete,
}

impl GoalStatus {
    pub fn label(self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Blocked => "blocked",
            GoalStatus::BudgetLimited => "budget-limited",
            GoalStatus::Complete => "complete",
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
    pub fn arm(&mut self, id: PermissionRequestId, responder: oneshot::Sender<Resolution>) {
        self.pending = Some(Pending { id, responder });
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
            // Plan approval: both variants approve; they differ only in which
            // client-side mode the app switches to afterwards.
            "approve_auto" | "approve_review" => Decision::AllowOnce,
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
        assert_eq!(Decision::from_option("approve_auto"), Decision::AllowOnce);
        assert_eq!(Decision::from_option("approve_review"), Decision::AllowOnce);
    }

    #[tokio::test]
    async fn run_control_resolves_matching_request() {
        let mut control = RunControl::default();
        let id = PermissionRequestId::new("perm-1");
        let (tx, rx) = oneshot::channel();
        control.arm(id.clone(), tx);
        assert!(control.resolve(&id, Decision::AllowOnce.into()));
        assert_eq!(rx.await.unwrap().decision, Decision::AllowOnce);
    }

    #[tokio::test]
    async fn run_control_resolution_carries_feedback() {
        let mut control = RunControl::default();
        let id = PermissionRequestId::new("perm-2");
        let (tx, rx) = oneshot::channel();
        control.arm(id.clone(), tx);
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
}
