//! Planning policy and state for the local coding provider.
//!
//! Execution checklists, read-only collaboration mode, and standing goals are
//! deliberately separate axes. This module owns the first two; goal
//! continuation remains in `loop_state`/`engine`.

use std::collections::HashSet;

use agent_core::domain::{
    ChecklistStatus, ChecklistStep, ExecutionChecklist, ProposedPlan, ProposedPlanStatus,
};
use agent_core::provider::CollaborationMode;
use serde_json::Value;

pub(crate) const DEVELOPER_INSTRUCTION_MESSAGE_KIND: &str = "developer_instruction";
const MAX_REINJECTED_PLAN_CHARS: usize = 6_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanModeInstructionKind {
    Full,
    Reminder,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PlanningPromptProfile {
    Legacy,
    DecisionComplete,
    #[default]
    Concise,
}

impl PlanningPromptProfile {
    pub(crate) fn from_extra(value: Option<&str>) -> Self {
        match value {
            Some("legacy") => Self::Legacy,
            Some("decision_complete") => Self::DecisionComplete,
            _ => Self::Concise,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct PlanningState {
    pub mode: CollaborationMode,
    pub exited: bool,
    full_instruction_sent: bool,
    pub execution_checklist: Option<ExecutionChecklist>,
    pub proposed_plan: Option<ProposedPlan>,
}

impl PlanningState {
    pub fn plan_mode(&self) -> bool {
        self.mode == CollaborationMode::Plan
    }

    pub fn set_mode(&mut self, mode: CollaborationMode) {
        if self.mode == CollaborationMode::Plan && mode == CollaborationMode::Default {
            self.exited = true;
        } else if self.mode != CollaborationMode::Plan && mode == CollaborationMode::Plan {
            self.exited = false;
            self.full_instruction_sent = false;
        }
        self.mode = mode;
    }

    pub fn next_plan_instruction_kind(&mut self) -> PlanModeInstructionKind {
        if std::mem::replace(&mut self.full_instruction_sent, true) {
            PlanModeInstructionKind::Reminder
        } else {
            PlanModeInstructionKind::Full
        }
    }

    pub fn next_proposal(&mut self, markdown: String) -> ProposedPlan {
        let (id, revision) = match self.proposed_plan.as_mut() {
            Some(previous) => {
                previous.status = ProposedPlanStatus::Superseded;
                (previous.id.clone(), previous.revision.saturating_add(1))
            }
            None => (uuid::Uuid::new_v4().to_string(), 1),
        };
        let plan = ProposedPlan {
            id,
            revision,
            markdown,
            status: ProposedPlanStatus::AwaitingDecision,
        };
        self.proposed_plan = Some(plan.clone());
        plan
    }
}

/// Stable planning guidance used in the base prompt. Execution checklists are
/// advisory progress state; they never change permissions or collaboration.
pub(crate) const EXECUTION_CHECKLIST_INSTRUCTIONS: &str = "\
- Use `update_plan` only for non-trivial execution. It tracks progress, not permission or Plan Mode.\n\
- Send the full checklist; keep exactly one step `in_progress` until all complete, and update it as work happens.\n\
- Explain changed steps. Do not repeat the checklist in prose.\n";

/// Per-turn Plan Mode contract. This is intentionally separate from the base
/// execution checklist guidance so the two mechanisms cannot blur together.
#[cfg(test)]
pub(crate) fn plan_mode_instructions_for(
    profile: PlanningPromptProfile,
    previous: Option<&ProposedPlan>,
) -> String {
    plan_mode_instruction_for(profile, previous, PlanModeInstructionKind::Full)
}

pub(crate) fn plan_mode_instruction_for(
    profile: PlanningPromptProfile,
    previous: Option<&ProposedPlan>,
    kind: PlanModeInstructionKind,
) -> String {
    if kind == PlanModeInstructionKind::Reminder {
        return plan_mode_reminder(previous);
    }
    if profile == PlanningPromptProfile::Legacy {
        return "Plan mode is active. Research read-only, ask the user about unclear choices, and call `propose_plan` with normally 3-7 concise implementation steps when ready. Do not edit project files or run mutating commands."
            .to_string();
    }
    if profile == PlanningPromptProfile::DecisionComplete {
        let mut instructions = String::from(
            "Plan Mode is active. Propose; do not execute. You MUST NOT edit files, install software, \n\
             run mutating commands, or otherwise change project or external state. This rule \n\
             overrides execution instructions. Read/search tools, read-only shell commands, and \n\
             research are allowed. `update_plan` is not available in Plan Mode.\n\
             \n\
             Work through three phases, returning to an earlier phase whenever new evidence requires it:\n\
             1. Ground in the environment. Inspect the named files and the smallest useful contract \n\
             boundary. Resolve facts from code instead of asking the user. Trace existing abstractions, \n\
             tests, and constraints far enough that the plan is implementable.\n\
             2. Resolve intent. Ask concise questions only when the answer materially changes behavior, \n\
             scope, or a trade-off and cannot be learned from the environment. Batch related questions \n\
             and include a recommended default. Do not ask for approval in ordinary prose.\n\
             3. Resolve implementation. Specify the concrete files and interfaces to change, reuse and \n\
             deletion choices, data flow, edge cases, migration/compatibility behavior, and verification. \n\
             The plan must leave the implementer no design decisions hidden behind vague verbs.\n\
             \n\
             When decision-complete, call `propose_plan` once with a concise Markdown plan (normally \n\
             3-7 cohesive steps). The call ends the planning turn; wait for the user's typed decision. \n\
             Otherwise end with the smallest necessary user question. Never call `propose_plan` merely \n\
             to report research, and never begin implementation yourself.",
        );
        append_previous_proposal(&mut instructions, previous);
        return instructions;
    }

    let mut instructions = String::from("<collaboration_mode mode=\"plan\">\n");
    if let Some(plan) = previous {
        let markdown = bounded_plan_markdown(&plan.markdown);
        instructions.push_str(&format!(
            "Revise this previous proposal using new evidence and feedback:\n\
             <previous_proposed_plan id=\"{}\" revision=\"{}\">\n{}\n</previous_proposed_plan>\n\n",
            plan.id, plan.revision, markdown
        ));
    }
    instructions.push_str(
        "Plan Mode is active. Only a host collaboration-mode change can end it; user requests to \n\
         implement do not. Propose, do not execute. Do not edit files, install software, run \n\
         mutating commands, or change project or external state. Read-only inspection and research \n\
         are allowed. Do not call `update_plan`.\n\
         \n\
         Work in this order:\n\
         1. Ground: start with one batched search, then read only the few relevant files. Resolve \n\
         code-derived facts yourself; do not repeat equivalent probes.\n\
         2. Intent: ask only questions that materially change behavior, scope, or trade-offs; batch them and recommend a default.\n\
         3. Implementation: identify exact files and interfaces, reuse or deletion, data flow, edge cases, migration, and verification.\n\
         \n\
         When no design decision remains, call `propose_plan` once with 3-7 terse, implementation-ready \n\
         steps. Each step names the files, change, and verification in at most three sentences. Omit \n\
         preambles, code blocks, research diaries, alternatives, and out-of-scope sections. Otherwise \n\
         ask the smallest necessary question. After `propose_plan`, end the turn and wait. Never \n\
         implement in Plan Mode.\n</collaboration_mode>",
    );
    instructions
}

fn plan_mode_reminder(previous: Option<&ProposedPlan>) -> String {
    let proposal = previous
        .map(|plan| {
            format!(
                " Previous proposal: id {}, revision {}; use its typed transcript item and revise only when feedback changes it.",
                plan.id, plan.revision
            )
        })
        .unwrap_or_default();
    format!(
        "<collaboration_mode mode=\"plan\">\nPlan Mode remains active; only a host mode change can end it. Stay read-only, resolve the smallest remaining decision, then call `propose_plan` once and wait.{proposal}\n</collaboration_mode>"
    )
}

fn append_previous_proposal(instructions: &mut String, previous: Option<&ProposedPlan>) {
    if let Some(plan) = previous {
        let markdown = bounded_plan_markdown(&plan.markdown);
        instructions.push_str(&format!(
            "\n\nA previous proposal (id {}, revision {}) exists:\n<previous_proposed_plan>\n{}\n</previous_proposed_plan>\nReconcile new evidence or feedback with it. Re-proposing the same plan must preserve its identity and increment its revision.",
            plan.id, plan.revision, markdown
        ));
    }
}

pub(crate) fn bounded_plan_markdown(markdown: &str) -> String {
    let char_count = markdown.chars().count();
    if char_count <= MAX_REINJECTED_PLAN_CHARS {
        return markdown.to_string();
    }
    const MARKER: &str = "\n\n[... proposal middle omitted by context bound ...]\n\n";
    let remaining = MAX_REINJECTED_PLAN_CHARS.saturating_sub(MARKER.chars().count());
    let head_chars = remaining * 4 / 5;
    let tail_chars = remaining - head_chars;
    let head = markdown.chars().take(head_chars).collect::<String>();
    let tail = markdown
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}{MARKER}{tail}")
}

pub(crate) fn developer_instruction_message(content: String) -> clark_agent::AgentMessage {
    clark_agent::AgentMessage::Custom {
        kind: DEVELOPER_INSTRUCTION_MESSAGE_KIND.into(),
        payload: serde_json::json!({ "content": content }),
        timestamp: None,
    }
}

pub(crate) fn plan_mode_exit_note(plan: Option<&ProposedPlan>) -> String {
    match plan {
        Some(plan) => {
            let markdown = bounded_plan_markdown(&plan.markdown);
            format!(
            "<collaboration_mode mode=\"default\">\nPlan Mode is off. Implement the approved plan below.\n\
             <approved_plan id=\"{}\" revision=\"{}\">\n{}\n</approved_plan>\n</collaboration_mode>",
            plan.id, plan.revision, markdown
        )
        }
        None => "<collaboration_mode mode=\"default\">\nPlan Mode is off; normal execution rules apply.\n</collaboration_mode>".to_string(),
    }
}

pub(crate) struct ChecklistUpdate {
    pub checklist: ExecutionChecklist,
    pub explanation: Option<String>,
}

pub(crate) fn parse_checklist_update(
    args: &Value,
    previous: Option<&ExecutionChecklist>,
) -> Result<ChecklistUpdate, String> {
    let items = args
        .get("plan")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing required array argument `plan`".to_string())?;
    if items.is_empty() {
        return Err("`plan` must contain at least one step".into());
    }

    let mut seen = HashSet::new();
    let mut in_progress = 0usize;
    let mut steps = Vec::with_capacity(items.len());
    for item in items {
        let title = item
            .get("step")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|step| !step.is_empty())
            .ok_or_else(|| "every plan item needs a non-empty `step`".to_string())?;
        let normalized = title.to_ascii_lowercase();
        if !seen.insert(normalized) {
            return Err(format!("duplicate plan step: {title}"));
        }
        let status = match item.get("status").and_then(Value::as_str) {
            Some("pending") => ChecklistStatus::Pending,
            Some("in_progress") => {
                in_progress += 1;
                ChecklistStatus::InProgress
            }
            Some("completed") => ChecklistStatus::Completed,
            Some(other) => return Err(format!("invalid plan status: {other}")),
            None => return Err(format!("plan step `{title}` is missing `status`")),
        };
        steps.push(ChecklistStep {
            title: title.to_string(),
            status,
            priority: None,
        });
    }

    let all_completed = steps
        .iter()
        .all(|step| step.status == ChecklistStatus::Completed);
    if (!all_completed && in_progress != 1) || (all_completed && in_progress != 0) {
        return Err("exactly one step must be in_progress until every step is completed".into());
    }

    let explanation = args
        .get("explanation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    if let Some(previous) = previous {
        let structure_changed = previous.steps.len() != steps.len()
            || previous
                .steps
                .iter()
                .zip(&steps)
                .any(|(left, right)| left.title != right.title);
        if structure_changed && explanation.is_none() {
            return Err("changing plan steps requires a short `explanation`".into());
        }
    }

    Ok(ChecklistUpdate {
        checklist: ExecutionChecklist {
            steps,
            revision: previous.map_or(1, |plan| plan.revision.saturating_add(1)),
        },
        explanation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checklist_requires_one_active_step_until_complete() {
        assert!(parse_checklist_update(
            &json!({"plan": [
                {"step":"Build","status":"in_progress"},
                {"step":"Test","status":"pending"}
            ]}),
            None,
        )
        .is_ok());
        assert!(parse_checklist_update(
            &json!({"plan": [{"step":"Build","status":"pending"}]}),
            None,
        )
        .is_err());
    }

    #[test]
    fn structural_replan_requires_explanation() {
        let first = parse_checklist_update(
            &json!({"plan": [{"step":"Build","status":"in_progress"}]}),
            None,
        )
        .unwrap();
        assert!(parse_checklist_update(
            &json!({"plan": [{"step":"Test","status":"in_progress"}]}),
            Some(&first.checklist),
        )
        .is_err());
        assert!(parse_checklist_update(
            &json!({
                "explanation":"The failure moved verification earlier.",
                "plan": [{"step":"Test","status":"in_progress"}]
            }),
            Some(&first.checklist),
        )
        .is_ok());
    }

    #[test]
    fn plan_mode_contract_is_read_only_and_decision_complete() {
        let prompt = plan_mode_instructions_for(PlanningPromptProfile::DecisionComplete, None);
        assert!(prompt.contains("Propose; do not execute"));
        assert!(prompt.contains("three phases"));
        assert!(prompt.contains("Resolve facts from code"));
        assert!(prompt.contains("typed decision"));
        assert!(!prompt.contains("plan.md"));
    }

    #[test]
    fn concise_plan_mode_contract_is_authoritative_and_shorter() {
        let control = plan_mode_instructions_for(PlanningPromptProfile::DecisionComplete, None);
        let candidate = plan_mode_instructions_for(PlanningPromptProfile::Concise, None);
        assert!(candidate.starts_with("<collaboration_mode"));
        assert!(candidate.contains("host collaboration-mode change can end it"));
        assert!(candidate.contains("Work in this order"));
        assert!(candidate.contains("one batched search"));
        assert!(candidate.contains("do not repeat equivalent probes"));
        assert!(candidate.contains("preambles, code blocks"));
        assert!(candidate.contains("implement in Plan Mode"));
        assert!(candidate.len() < control.len());
    }

    #[test]
    fn concise_plan_mode_puts_previous_plan_before_the_recency_guard() {
        let plan = ProposedPlan {
            id: "plan-1".into(),
            revision: 2,
            markdown: "1. Old instruction".into(),
            status: ProposedPlanStatus::AwaitingDecision,
        };
        let prompt = plan_mode_instructions_for(PlanningPromptProfile::Concise, Some(&plan));
        assert!(
            prompt.find("Old instruction").unwrap() < prompt.find("Plan Mode is active").unwrap()
        );
        assert!(prompt.ends_with("</collaboration_mode>"));
    }

    #[test]
    fn plan_mode_sends_full_contract_once_then_sparse_reminders() {
        let mut state = PlanningState::default();
        state.set_mode(CollaborationMode::Plan);
        assert_eq!(
            state.next_plan_instruction_kind(),
            PlanModeInstructionKind::Full
        );
        assert_eq!(
            state.next_plan_instruction_kind(),
            PlanModeInstructionKind::Reminder
        );
        let reminder = plan_mode_instruction_for(
            PlanningPromptProfile::Concise,
            None,
            PlanModeInstructionKind::Reminder,
        );
        assert!(reminder.contains("Plan Mode remains active"));
        assert!(!reminder.contains("Work in this order"));

        state.set_mode(CollaborationMode::Default);
        state.set_mode(CollaborationMode::Plan);
        assert_eq!(
            state.next_plan_instruction_kind(),
            PlanModeInstructionKind::Full
        );
    }

    #[test]
    fn reinjected_proposals_have_a_hard_context_bound() {
        let plan = ProposedPlan {
            id: "plan-large".into(),
            revision: 1,
            markdown: "a".repeat(MAX_REINJECTED_PLAN_CHARS + 2_000),
            status: ProposedPlanStatus::Approved,
        };
        let bounded = bounded_plan_markdown(&plan.markdown);
        assert_eq!(bounded.chars().count(), MAX_REINJECTED_PLAN_CHARS);
        assert!(bounded.contains("proposal middle omitted"));
        assert!(plan_mode_exit_note(Some(&plan)).contains(&bounded));
    }

    #[test]
    fn collaboration_policy_is_a_typed_developer_instruction() {
        let message = developer_instruction_message("policy".into());
        assert!(matches!(
            message,
            clark_agent::AgentMessage::Custom { kind, payload, .. }
                if kind == DEVELOPER_INSTRUCTION_MESSAGE_KIND
                    && payload["content"] == "policy"
        ));
    }

    #[test]
    fn exit_note_carries_the_approved_plan_without_a_file_side_channel() {
        let plan = ProposedPlan {
            id: "plan-1".into(),
            revision: 2,
            markdown: "1. Change the boundary".into(),
            status: ProposedPlanStatus::Approved,
        };
        let note = plan_mode_exit_note(Some(&plan));
        assert!(note.contains("plan-1"));
        assert!(note.contains("Change the boundary"));
        assert!(!note.contains("plan.md"));
    }
}
