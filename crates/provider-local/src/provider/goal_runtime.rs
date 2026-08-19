//! Typed host access to the active session's durable goal.

use agent_core::{GoalState, GoalStatus};

use super::*;

impl LocalAgentProvider {
    pub(super) async fn current_goal(&self, session: &SessionId) -> Result<Option<GoalState>> {
        self.ensure_goal_session(session)?;
        Ok(self
            .session
            .lock()
            .await
            .goal
            .as_ref()
            .map(|goal| goal.state(None)))
    }

    pub(super) async fn resume_session_goal(&mut self, session: &SessionId) -> Result<GoalState> {
        self.ensure_goal_session(session)?;
        if self.run_cancellations.has_active() {
            return Err(Error::Unsupported(
                "wait for the active run to finish before resuming its goal".into(),
            ));
        }
        let mut state = self.session.lock().await;
        let goal = state
            .goal
            .as_mut()
            .ok_or_else(|| Error::Other("this session has no goal".into()))?;
        match goal.status {
            GoalStatus::Blocked => {}
            GoalStatus::Active => {
                return Err(Error::Unsupported("the goal is already active".into()));
            }
            GoalStatus::Complete => {
                return Err(Error::Unsupported(
                    "a completed goal cannot be resumed; create a new goal".into(),
                ));
            }
        }
        goal.status = GoalStatus::Active;
        goal.blocker_reason = None;
        goal.blocker_observations = 0;
        goal.last_blocker_continuation = None;
        goal.touch();
        Ok(goal.state(None))
    }

    /// Remove the session's standing goal entirely. User-initiated via the
    /// host; the engine sees `None` on its next continuation decision and
    /// stops, and the projection retires the receipt. Allowed while a run is
    /// active — the in-flight turn finishes, then the loop stops.
    pub(super) async fn clear_session_goal(&self, session: &SessionId) -> Result<()> {
        self.ensure_goal_session(session)?;
        self.session.lock().await.goal = None;
        Ok(())
    }

    fn ensure_goal_session(&self, session: &SessionId) -> Result<()> {
        if self.session_id.as_ref() == Some(session) {
            Ok(())
        } else {
            Err(Error::Unsupported(
                "Clark Code can only inspect the goal owned by the active session".into(),
            ))
        }
    }
}
