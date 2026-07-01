use std::collections::HashMap;

use agent_core::ids::PermissionRequestId;
use clark_agent::AgentMessage;
use tokio::sync::oneshot;

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
    /// destructive suffix past the gate.
    pub allow_commands: Vec<String>,
    /// Shell-command prefixes that are always refused.
    pub deny_commands: Vec<String>,
}

/// Live control surface for the current run, reachable from respond/cancel.
#[derive(Default)]
pub(crate) struct RunControl {
    pending: Option<Pending>,
}

struct Pending {
    id: PermissionRequestId,
    responder: oneshot::Sender<Decision>,
}

impl RunControl {
    pub fn arm(&mut self, id: PermissionRequestId, responder: oneshot::Sender<Decision>) {
        self.pending = Some(Pending { id, responder });
    }

    /// Deliver a user's answer to the in-flight permission request. Returns
    /// `true` if a request was actually waiting.
    pub fn resolve(&mut self, id: &PermissionRequestId, decision: Decision) -> bool {
        match self.pending.take() {
            Some(p) if &p.id == id || id.as_str().is_empty() => {
                let _ = p.responder.send(decision);
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
    }

    #[tokio::test]
    async fn run_control_resolves_matching_request() {
        let mut control = RunControl::default();
        let id = PermissionRequestId::new("perm-1");
        let (tx, rx) = oneshot::channel();
        control.arm(id.clone(), tx);
        assert!(control.resolve(&id, Decision::AllowOnce));
        assert_eq!(rx.await.unwrap(), Decision::AllowOnce);
    }
}
