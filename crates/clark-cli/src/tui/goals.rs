use agent_core::{
    AgentEvent, ChecklistStatus, CollaborationMode, ExecutionChecklist,
    GoalState as ProviderGoalState, GoalStatus, ProposedPlan, ProposedPlanStatus,
};
#[cfg(test)]
use agent_core::{ResumeItem, ResumeTranscript};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlanGoalState {
    collaboration_mode: CollaborationMode,
    proposed_plan: Option<ProposedPlan>,
    execution_checklist: Option<ExecutionChecklist>,
    goal: Option<ProviderGoalState>,
}

impl Default for PlanGoalState {
    fn default() -> Self {
        Self::new(CollaborationMode::Default)
    }
}

impl PlanGoalState {
    pub(crate) fn new(collaboration_mode: CollaborationMode) -> Self {
        Self {
            collaboration_mode,
            proposed_plan: None,
            execution_checklist: None,
            goal: None,
        }
    }

    pub(crate) fn with_goal(
        collaboration_mode: CollaborationMode,
        goal: Option<ProviderGoalState>,
    ) -> Self {
        let mut state = Self::new(collaboration_mode);
        state.goal = goal;
        state
    }

    pub(crate) fn observe_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::ExecutionChecklistUpdated { checklist, .. } => {
                self.execution_checklist = Some(checklist.clone());
            }
            AgentEvent::ProposedPlanUpdated { plan, .. } => {
                self.proposed_plan = Some(plan.clone());
            }
            AgentEvent::GoalUpdated { goal, .. } => {
                self.goal = Some(goal.clone());
                if goal.status == GoalStatus::Complete {
                    if let Some(checklist) = self.execution_checklist.as_mut() {
                        for step in &mut checklist.steps {
                            step.status = ChecklistStatus::Completed;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn restore(
        collaboration_mode: CollaborationMode,
        transcript: &ResumeTranscript,
    ) -> Self {
        let mut state = Self::new(collaboration_mode);
        for item in &transcript.items {
            match item {
                ResumeItem::Goal { goal } => state.goal = Some(goal.clone()),
                ResumeItem::ProposedPlan { plan } => state.proposed_plan = Some(plan.clone()),
                _ => {}
            }
        }
        state
    }

    #[cfg(test)]
    pub(crate) fn append_resume_items(&self, transcript: &mut ResumeTranscript) {
        transcript.items.retain(|item| {
            !matches!(
                item,
                ResumeItem::Goal { .. } | ResumeItem::ProposedPlan { .. }
            )
        });
        if let Some(plan) = &self.proposed_plan {
            transcript
                .items
                .push(ResumeItem::ProposedPlan { plan: plan.clone() });
        }
        if let Some(goal) = &self.goal {
            transcript
                .items
                .push(ResumeItem::Goal { goal: goal.clone() });
        }
    }

    pub(crate) fn desired_height(&self) -> u16 {
        if self.panel_lines().is_empty() {
            0
        } else {
            u16::try_from(self.panel_lines().len())
                .unwrap_or(4)
                .min(4)
                .saturating_add(2)
        }
    }

    pub(crate) fn panel_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.collaboration_mode == CollaborationMode::Plan {
            lines.push("Plan mode · read-only research and planning".into());
        }
        if let Some(plan) = &self.proposed_plan {
            lines.push(format!(
                "Plan r{} · {} · {}",
                plan.revision,
                plan_status(plan.status),
                first_nonempty_line(&plan.markdown).unwrap_or("untitled plan")
            ));
        }
        if let Some(checklist) = &self.execution_checklist {
            let completed = checklist
                .steps
                .iter()
                .filter(|step| step.status == ChecklistStatus::Completed)
                .count();
            let current = checklist
                .steps
                .iter()
                .find(|step| step.status == ChecklistStatus::InProgress)
                .map(|step| format!(" · now: {}", step.title))
                .unwrap_or_default();
            lines.push(format!(
                "Checklist · {completed}/{} complete{current}",
                checklist.steps.len()
            ));
        }
        if let Some(goal) = &self.goal {
            lines.push(format!(
                "Goal · {} · {} · {} continuation{} · {} tokens",
                goal_status(goal.status),
                goal.objective,
                goal.continuations,
                if goal.continuations == 1 { "" } else { "s" },
                goal.tokens_used
            ));
            if let Some(reason) = goal.blocker_reason.as_deref() {
                lines.push(format!("Blocked · {reason}"));
            }
        }
        lines
    }

    pub(crate) fn set_goal(&mut self, goal: ProviderGoalState) {
        self.goal = Some(goal);
    }

    pub(crate) fn goal_report(&self) -> String {
        let Some(goal) = &self.goal else {
            return "Clark durable goal\nNo goal is set for this session. Ask Clark to start a durable goal; /goal then inspects or resumes its typed provider state."
                .into();
        };
        let budget = goal.token_budget.map_or_else(
            || "no user token budget".to_string(),
            |budget| format!("{} / {budget} tokens", goal.tokens_used),
        );
        format!(
            "Clark durable goal\nID: {}\nStatus: {}\nObjective: {}\nUsage: {budget} · {} seconds · {} continuation{}\nLast update: {}{}",
            goal.id,
            goal_status(goal.status),
            goal.objective,
            goal.time_used_seconds,
            goal.continuations,
            if goal.continuations == 1 { "" } else { "s" },
            goal.updated_at_ms,
            goal.blocker_reason
                .as_deref()
                .map(|reason| format!("\nBlocker: {reason}"))
                .unwrap_or_default()
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PlanGoalCommand {
    Inspect,
    Resume(Option<u64>),
}

pub(crate) fn parse(line: &str) -> Option<Result<PlanGoalCommand, String>> {
    let command_line = line.trim().strip_prefix('/')?;
    let mut words = command_line.split_whitespace();
    match words.next()? {
        "goal" => match words.collect::<Vec<_>>().as_slice() {
            [] | ["status"] => Some(Ok(PlanGoalCommand::Inspect)),
            ["resume"] => Some(Ok(PlanGoalCommand::Resume(None))),
            ["resume", "--tokens", value] => Some(
                value
                    .parse::<u64>()
                    .ok()
                    .filter(|budget| *budget > 0)
                    .map(|budget| PlanGoalCommand::Resume(Some(budget)))
                    .ok_or_else(|| "Goal token budget must be a positive integer.".into()),
            ),
            ["resume", ..] => Some(Err(
                "Usage: /goal resume [--tokens NEW_TOTAL_TOKEN_BUDGET]".into()
            )),
            _ => None,
        },
        _ => None,
    }
}

fn first_nonempty_line(markdown: &str) -> Option<&str> {
    markdown
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
}

fn plan_status(status: ProposedPlanStatus) -> &'static str {
    match status {
        ProposedPlanStatus::AwaitingDecision => "awaiting decision",
        ProposedPlanStatus::Approved => "approved",
        ProposedPlanStatus::Superseded => "superseded",
    }
}

fn goal_status(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "active",
        GoalStatus::Blocked => "blocked",
        GoalStatus::BudgetLimited => "budget limited",
        GoalStatus::Complete => "complete",
    }
}

#[cfg(test)]
#[path = "goals_tests.rs"]
mod tests;
