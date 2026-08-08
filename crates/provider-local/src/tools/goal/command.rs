use crate::loop_state::{GoalStatus, SessionState};

use super::{start_goal, validated_objective};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GoalCommandAction {
    Start,
    ContinueExisting,
}

/// Validate the built-in `/goal` command without changing session state. The
/// same objective is an idempotent continuation request: this is the natural
/// retry after a provider failure has paused a standing goal.
pub(crate) fn validate_goal_command(
    session: &SessionState,
    objective: &str,
) -> Result<GoalCommandAction, String> {
    let objective = validated_objective(objective).map_err(|error| {
        if objective.trim().is_empty() {
            "Add an objective after `/goal`.".to_string()
        } else {
            format!("The `/goal` objective is invalid: {error}.")
        }
    })?;
    if session.planning.plan_mode() {
        return Err(
            "Plan mode is active — finish or leave the plan before starting a standing goal."
                .into(),
        );
    }
    let Some(existing) = session
        .goal
        .as_ref()
        .filter(|goal| goal.status != GoalStatus::Complete)
    else {
        return Ok(GoalCommandAction::Start);
    };
    if existing.objective == objective {
        return match existing.status {
            GoalStatus::Active | GoalStatus::Blocked => {
                Ok(GoalCommandAction::ContinueExisting)
            }
            GoalStatus::BudgetLimited => Err(
                "This goal reached its token budget — resume it with a larger total budget, or start a new conversation for a different goal."
                    .into(),
            ),
            GoalStatus::Complete => unreachable!("completed goals were filtered above"),
        };
    }
    Err(format!(
        "This conversation already has an unfinished {} goal — send a follow-up to continue it, or start a new conversation for a different goal.",
        existing.status_label()
    ))
}

pub(crate) fn apply_goal_command(
    session: &mut SessionState,
    objective: &str,
) -> Result<GoalCommandAction, String> {
    let action = validate_goal_command(session, objective)?;
    if action == GoalCommandAction::Start {
        start_goal(session, objective.trim().to_string(), None)?;
    }
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_goal_command_continues_the_same_blocked_goal() {
        let mut session = SessionState::default();
        apply_goal_command(&mut session, "finish the migration").unwrap();
        let original_id = session.goal.as_ref().unwrap().id.clone();
        session.goal.as_mut().unwrap().status = GoalStatus::Blocked;

        let action = apply_goal_command(&mut session, " finish the migration ").unwrap();

        assert_eq!(action, GoalCommandAction::ContinueExisting);
        assert_eq!(session.goal.as_ref().unwrap().id, original_id);
        assert_eq!(session.goal.as_ref().unwrap().status, GoalStatus::Blocked);
    }

    #[test]
    fn conflicting_goal_command_returns_user_recovery_without_mutating_state() {
        let mut session = SessionState::default();
        apply_goal_command(&mut session, "finish the migration").unwrap();
        session.goal.as_mut().unwrap().status = GoalStatus::Blocked;

        let error = apply_goal_command(&mut session, "rewrite the renderer").unwrap_err();

        assert!(error.contains("send a follow-up to continue it"));
        assert!(error.contains("start a new conversation"));
        assert!(!error.contains("update_goal"));
        assert_eq!(
            session.goal.as_ref().unwrap().objective,
            "finish the migration"
        );
    }
}
