//! Planning policy and state for the local coding provider.
//!
//! Execution checklists, read-only collaboration mode, and standing goals are
//! deliberately separate axes. This module owns the first two; goal
//! continuation remains in `loop_state`/`engine`.

mod research;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_core::domain::{
    ChecklistStatus, ChecklistStep, ExecutionChecklist, PlanExecutionStep, ProposedPlan,
    ProposedPlanStatus,
};
use agent_core::provider::CollaborationMode;
use agent_loop::{
    AgentMessage, ContextTransform, FollowUpSource, Plugin, PluginCapabilities, TransformContext,
};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::loop_state::SessionState;

pub(crate) use research::{available_source_tools, source_tool_names};

pub(crate) const DEVELOPER_INSTRUCTION_MESSAGE_KIND: &str = "developer_instruction";
const MAX_EXECUTION_CONTRACT_CHARS: usize = 12_000;
const PERIODIC_EXECUTION_REMINDER_TURNS: usize = 3;
const MAX_COMPLETION_REMINDERS: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanModeInstructionKind {
    Full,
    Reminder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExecutionReminderReason {
    StepCompleted(Vec<String>),
    Periodic,
    CompletionAudit,
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
    pending_execution_reminder: Option<ExecutionReminderReason>,
    completion_reminders: u8,
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

    #[cfg(test)]
    pub fn next_proposal(&mut self, markdown: String) -> ProposedPlan {
        self.next_structured_proposal(markdown, Vec::new(), Vec::new())
    }

    pub fn next_structured_proposal(
        &mut self,
        markdown: String,
        global_reminders: Vec<String>,
        mut execution_contract: Vec<PlanExecutionStep>,
    ) -> ProposedPlan {
        let (id, revision) = match self.proposed_plan.as_mut() {
            Some(previous) => {
                previous.status = ProposedPlanStatus::Superseded;
                (previous.id.clone(), previous.revision.saturating_add(1))
            }
            None => (uuid::Uuid::new_v4().to_string(), 1),
        };
        for (index, step) in execution_contract.iter_mut().enumerate() {
            step.id = format!("step-{}", index + 1);
        }
        let plan = ProposedPlan {
            id,
            revision,
            markdown,
            status: ProposedPlanStatus::AwaitingDecision,
            global_reminders,
            execution_contract,
        };
        self.proposed_plan = Some(plan.clone());
        self.execution_checklist = None;
        self.pending_execution_reminder = None;
        self.completion_reminders = 0;
        plan
    }

    /// Record Agent Desktop's hidden Plan Mode artifact. The model-facing protocol is
    /// deliberately just Markdown; durable typed execution contracts remain a
    /// compatibility path for older transcripts and explicit structured
    /// callers, but are not required for a normal Plan Mode proposal.
    pub fn next_markdown_proposal(&mut self, markdown: String) -> ProposedPlan {
        self.next_structured_proposal(markdown, Vec::new(), Vec::new())
    }

    pub fn approve_execution(&mut self) {
        self.execution_checklist = None;
        self.pending_execution_reminder = None;
        self.completion_reminders = 0;
    }

    fn queue_execution_reminder(&mut self, reason: ExecutionReminderReason) {
        self.pending_execution_reminder = Some(reason);
    }

    pub fn record_checklist_update(&mut self, checklist: ExecutionChecklist) {
        let previous_status = self
            .execution_checklist
            .as_ref()
            .map(|previous| {
                previous
                    .steps
                    .iter()
                    .filter_map(|step| {
                        step.plan_step_id
                            .as_ref()
                            .map(|id| (id.clone(), step.status))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let newly_completed = checklist
            .steps
            .iter()
            .filter(|step| {
                step.status == ChecklistStatus::Completed
                    && step.plan_step_id.as_ref().is_some_and(|id| {
                        previous_status.get(id) != Some(&ChecklistStatus::Completed)
                    })
            })
            .filter_map(|step| step.plan_step_id.clone())
            .collect::<Vec<_>>();
        self.execution_checklist = Some(checklist);
        if !newly_completed.is_empty() {
            self.completion_reminders = 0;
            self.queue_execution_reminder(ExecutionReminderReason::StepCompleted(newly_completed));
        }
    }

    /// A completed standing goal is authoritative completion for its active
    /// work. Keep the persisted checklist in sync even if the model omitted a
    /// redundant final `update_plan` call.
    pub fn complete_execution_checklist(&mut self) -> Option<ExecutionChecklist> {
        let checklist = self.execution_checklist.as_mut()?;
        if checklist
            .steps
            .iter()
            .all(|step| step.status == ChecklistStatus::Completed)
        {
            return None;
        }
        for step in &mut checklist.steps {
            step.status = ChecklistStatus::Completed;
        }
        checklist.revision = checklist.revision.saturating_add(1);
        Some(checklist.clone())
    }
}

/// Stable planning guidance used in the base prompt. Execution checklists are
/// advisory progress state; they never change permissions or collaboration.
pub(crate) const EXECUTION_CHECKLIST_INSTRUCTIONS: &str = "\
- Use `update_plan` only for non-trivial execution. It tracks progress, not permission or Plan Mode.\n\
- Before substantial work, make its steps collectively cover every explicit requirement and compatibility constraint; preserve exact literals that could otherwise be lost.\n\
- Send the full checklist; keep exactly one step `in_progress` until all complete, and update it as work happens.\n\
- When an approved execution contract supplies `plan_step_id` values, preserve every ID exactly in `update_plan`.\n\
- Explain changed steps. Do not repeat the checklist in prose.\n";

pub(crate) struct ProposalContract {
    pub markdown: String,
    pub global_reminders: Vec<String>,
    pub execution_contract: Vec<PlanExecutionStep>,
}

pub(crate) fn parse_proposal_contract(args: &Value) -> Result<ProposalContract, String> {
    let global_reminders =
        bounded_string_array(args.get("global_reminders"), "global_reminders", 1, 5, 500)?;
    let raw_steps = args
        .get("execution_contract")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing required array argument `execution_contract`".to_string())?;
    if !(1..=7).contains(&raw_steps.len()) {
        return Err("execution_contract must contain between 1 and 7 steps".into());
    }

    let mut titles = HashSet::new();
    let mut execution_contract = Vec::with_capacity(raw_steps.len());
    for (index, raw) in raw_steps.iter().enumerate() {
        let label = format!("execution_contract[{}]", index + 1);
        let title = bounded_string(raw.get("title"), &format!("{label}.title"), 200)?;
        if !titles.insert(title.to_ascii_lowercase()) {
            return Err(format!("duplicate execution step title: {title}"));
        }
        let files = bounded_string_array(raw.get("files"), &format!("{label}.files"), 1, 8, 300)?;
        let done_when = bounded_string_array(
            raw.get("done_when"),
            &format!("{label}.done_when"),
            1,
            4,
            500,
        )?;
        let reminders = bounded_string_array(
            raw.get("reminders"),
            &format!("{label}.reminders"),
            1,
            4,
            500,
        )?;
        execution_contract.push(PlanExecutionStep {
            id: String::new(),
            title,
            files,
            done_when,
            reminders,
        });
    }
    let markdown = bounded_string(args.get("plan"), "plan", 12_000)?;
    let contract_chars = serde_json::to_string(&(&global_reminders, &execution_contract))
        .map_err(|error| format!("cannot serialize execution contract: {error}"))?
        .chars()
        .count();
    if contract_chars > MAX_EXECUTION_CONTRACT_CHARS {
        return Err(format!(
            "execution contract exceeds {MAX_EXECUTION_CONTRACT_CHARS} serialized characters"
        ));
    }
    Ok(ProposalContract {
        markdown,
        global_reminders,
        execution_contract,
    })
}

fn bounded_string(value: Option<&Value>, label: &str, max_chars: usize) -> Result<String, String> {
    let text = value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("`{label}` must be a non-empty string"))?;
    if text.chars().count() > max_chars {
        return Err(format!("`{label}` exceeds {max_chars} characters"));
    }
    Ok(text.to_string())
}

fn bounded_string_array(
    value: Option<&Value>,
    label: &str,
    min_items: usize,
    max_items: usize,
    max_chars: usize,
) -> Result<Vec<String>, String> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing required array argument `{label}`"))?;
    if !(min_items..=max_items).contains(&items.len()) {
        return Err(format!(
            "`{label}` must contain between {min_items} and {max_items} items"
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, value)| {
            bounded_string(Some(value), &format!("{label}[{}]", index + 1), max_chars)
        })
        .collect()
}

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
        return "Plan Mode is active. Research read-only, ask the user about unclear choices, then emit one `<proposed_plan>` block when ready. Put a concise Markdown implementation plan inside the block, including exact files and verification. Do not edit project files or run mutating commands. End the turn after the block."
            .to_string();
    }
    if profile == PlanningPromptProfile::DecisionComplete {
        let mut instructions = format!(
            "Plan Mode is active. Propose; do not execute. You MUST NOT edit files, install software, \n\
             run mutating commands, or otherwise change project or external state. This rule \n\
             overrides execution instructions. Read/search tools, read-only shell commands, and \n\
             research are allowed. `update_plan` is not available in Plan Mode.\n\
             \n\
             Work through five phases, returning to an earlier phase whenever new evidence requires it:\n\
             1. Ground in the environment. Inspect the named files and the smallest useful contract \n\
             boundary. Resolve facts from code instead of asking the user. Trace existing abstractions, \n\
             tests, and constraints far enough that the plan is implementable.\n\
             2. Resolve intent. Ask concise questions only when the answer materially changes behavior, \n\
             scope, or a trade-off and cannot be learned from the environment. Batch related questions \n\
             and include a recommended default. Do not ask for approval in ordinary prose.\n\
             3. Resolve implementation. Specify the concrete files and interfaces to change, reuse and \n\
             deletion choices, data flow, edge cases, migration/compatibility behavior, and verification. \n\
             The plan must leave the implementer no design decisions hidden behind vague verbs.\n\
             {}\
             5. Audit coverage before proposing. Privately inventory every atomic obligation from the \n\
             user, repository evidence, scout findings, and supplied memory. Preserve exact identifiers, \n\
             repetitions, ordering constraints, negative paths, rollback requirements, and metrics. Map \n\
             every obligation to a typed step and observable completion evidence; revise the contract if \n\
             any obligation is uncovered. Do not expose private chain-of-thought or a research diary.\n\
             \n\
             When decision-complete, emit exactly one `<proposed_plan>` block containing a concise Markdown \n\
             rendering of the implementation plan. Include exact files and interfaces, dependencies and \n\
             ordering, edge cases and rollback, and observable verification. Preserve every obligation from \n\
             the user, repository evidence, scout findings, and supplied memory, but do not expose a private \n\
             chain-of-thought or research diary. End the planning turn after the block and wait for the user's \n\
             decision. Otherwise end with the smallest necessary user question. Never emit a plan merely to \n\
             report research, and never begin implementation yourself.",
            research::PROGRESSIVE_RESEARCH_PHASE,
        );
        append_previous_proposal(&mut instructions, previous);
        return instructions;
    }

    let mut instructions = String::from("<collaboration_mode mode=\"plan\">\n");
    if let Some(plan) = previous {
        instructions.push_str(&format!(
            "Revise this previous proposal using new evidence and feedback:\n\
             <previous_proposed_plan id=\"{}\" revision=\"{}\">\n{}\n</previous_proposed_plan>\n\n",
            plan.id, plan.revision, plan.markdown
        ));
    }
    instructions.push_str(&format!(
        "Plan Mode is active. Only a host collaboration-mode change can end it; user requests to \n\
         implement do not. Propose, do not execute. Do not edit files, install software, run \n\
         mutating commands, or change project or external state. Read-only inspection and research \n\
         are allowed. Do not call `update_plan`.\n\
         \n\
         Work in this order:\n\
         1. Ground: inspect the task's local contract boundary and repository structure. Resolve \n\
         code-derived facts yourself; use broad orientation before narrow reads, and do not repeat equivalent probes.\n\
         2. Intent: ask only questions that materially change behavior, scope, or trade-offs; batch them and recommend a default.\n\
         3. Implementation: identify exact files and interfaces, reuse or deletion, data flow, edge cases, migration, and verification.\n\
         {}\
         5. Coverage audit: privately inventory every atomic obligation from the user, repository, scout \n\
         findings, and supplied memory. Preserve exact names, repetitions, ordering, negative paths, rollback, \n\
         and metrics. Map each obligation to a typed step and observable evidence; revise before proposing if \n\
         anything is uncovered. Do not expose private chain-of-thought or a research diary.\n\
         \n\
         When no design decision remains, emit exactly one hidden `<proposed_plan>` block. The block must be \n\
         concise Markdown with ordered implementation steps, exact files and interfaces, dependencies, edge \n\
         cases, rollback or compatibility behavior, and observable verification. Preserve every atomic user, \n\
         repository, scout, and memory obligation in those steps. Do not expose private chain-of-thought, a \n\
         research diary, alternatives, or out-of-scope sections. The host removes the block from the visible \n\
         transcript and stores it as a first-class proposal. End the turn after the block and wait for approval. \n\
         Otherwise ask the smallest necessary question. Never implement in Plan Mode.\n</collaboration_mode>",
        research::PROGRESSIVE_RESEARCH_PHASE,
    ));
    instructions
}

/// Returns the exact first-turn Plan Mode instruction for benchmark receipts.
///
/// This intentionally accepts the serialized profile name used by
/// `LocalConfig` so evaluation code can hash the same contract that the
/// provider injects without duplicating the prompt text.
#[doc(hidden)]
pub fn planning_prompt_contract_for_eval(profile: &str) -> String {
    plan_mode_instruction_for(
        PlanningPromptProfile::from_extra(Some(profile)),
        None,
        PlanModeInstructionKind::Full,
    )
}

/// Returns the complete proposal text delivered to an executor after a typed
/// Plan Mode approval. Evaluation code uses this to prove that the stored and
/// delivered bytes are identical.
#[doc(hidden)]
pub fn complete_plan_markdown_for_eval(markdown: &str) -> String {
    markdown.to_string()
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
        "<collaboration_mode mode=\"plan\">\nPlan Mode remains active; only a host mode change can end it. Stay read-only. If evidence or a design decision is unresolved, continue the provisional-model -> challenge -> retrieve -> revise loop with the visible read-only sources. Then emit one `<proposed_plan>` block and wait.{proposal}\n</collaboration_mode>"
    )
}

const PROPOSED_PLAN_OPEN: &str = "<proposed_plan>";
const PROPOSED_PLAN_CLOSE: &str = "</proposed_plan>";

/// Extract the hidden Plan Mode artifact from an assistant response. This is
/// intentionally a framing parser, not a semantic Markdown parser: the LLM
/// owns the plan's content and ordering, while the host owns only lifecycle
/// and visibility.
pub(crate) fn extract_proposed_plan(text: &str) -> Option<String> {
    let start = text.find(PROPOSED_PLAN_OPEN)? + PROPOSED_PLAN_OPEN.len();
    let end = text[start..].find(PROPOSED_PLAN_CLOSE)? + start;
    let markdown = text[start..end].trim();
    (!markdown.is_empty() && markdown.chars().count() <= 12_000).then(|| markdown.to_string())
}

/// Remove hidden proposal framing before assistant content reaches the visible
/// transcript or a subsequent model context.
pub(crate) fn strip_proposed_plan(text: &str) -> String {
    let mut visible = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(open_offset) = text[cursor..].find(PROPOSED_PLAN_OPEN) {
        let open = cursor + open_offset;
        visible.push_str(&text[cursor..open]);
        let body_start = open + PROPOSED_PLAN_OPEN.len();
        let Some(close_offset) = text[body_start..].find(PROPOSED_PLAN_CLOSE) else {
            visible.push_str(&text[open..]);
            return visible.trim().to_string();
        };
        cursor = body_start + close_offset + PROPOSED_PLAN_CLOSE.len();
    }
    visible.push_str(&text[cursor..]);
    visible.trim().to_string()
}

fn append_previous_proposal(instructions: &mut String, previous: Option<&ProposedPlan>) {
    if let Some(plan) = previous {
        instructions.push_str(&format!(
            "\n\nA previous proposal (id {}, revision {}) exists:\n<previous_proposed_plan>\n{}\n</previous_proposed_plan>\nReconcile new evidence or feedback with it. Re-proposing the same plan must preserve its identity and increment its revision.",
            plan.id, plan.revision, plan.markdown
        ));
    }
}

pub(crate) fn developer_instruction_message(content: String) -> agent_loop::AgentMessage {
    agent_loop::AgentMessage::Custom {
        kind: DEVELOPER_INSTRUCTION_MESSAGE_KIND.into(),
        payload: serde_json::json!({ "content": content }),
        timestamp: None,
    }
}

pub(crate) fn plan_mode_exit_note(plan: Option<&ProposedPlan>) -> String {
    match plan {
        Some(plan) => {
            if plan.execution_contract.is_empty() {
                return format!(
                    "<collaboration_mode mode=\"default\">\nPlan Mode is off. Implement the approved plan below.\n\
                     <approved_plan id=\"{}\" revision=\"{}\">\n{}\n</approved_plan>\n</collaboration_mode>",
                    plan.id, plan.revision, plan.markdown
                );
            }
            let contract = serde_json::to_string(&ApprovedExecutionContract {
                plan_id: &plan.id,
                revision: plan.revision,
                global_reminders: &plan.global_reminders,
                execution_contract: &plan.execution_contract,
            })
            .unwrap_or_else(|_| "{}".to_string());
            format!(
                "<collaboration_mode mode=\"default\">\nPlan Mode is off. Implement the approved plan. \
                 The JSON contract is authoritative and its step IDs are immutable. Initialize `update_plan` \
                 with every ID before substantial work, keep it synchronized, and satisfy each `done_when` \
                 before completing that step.\n\
                 <approved_execution_contract>{contract}</approved_execution_contract>\n\
                 <approved_plan_markdown>\n{}\n</approved_plan_markdown>\n\
                 </collaboration_mode>",
                plan.markdown
            )
        }
        None => "<collaboration_mode mode=\"default\">\nPlan Mode is off; normal execution rules apply.\n</collaboration_mode>".to_string(),
    }
}

#[derive(Serialize)]
struct ApprovedExecutionContract<'a> {
    plan_id: &'a str,
    revision: u32,
    global_reminders: &'a [String],
    execution_contract: &'a [PlanExecutionStep],
}

#[derive(Serialize)]
struct ExecutionReminderPayload<'a> {
    plan_id: &'a str,
    revision: u32,
    reason: &'a str,
    global_reminders: &'a [String],
    completed_step_ids: Vec<&'a str>,
    current_step: Option<&'a PlanExecutionStep>,
    remaining_steps: Vec<ExecutionStepSummary<'a>>,
}

#[derive(Serialize)]
struct ExecutionStepSummary<'a> {
    id: &'a str,
    title: &'a str,
}

fn render_execution_reminder(
    plan: &ProposedPlan,
    checklist: Option<&ExecutionChecklist>,
    reason: &ExecutionReminderReason,
) -> Option<String> {
    if plan.status != ProposedPlanStatus::Approved || plan.execution_contract.is_empty() {
        return None;
    }
    let statuses = checklist
        .map(|checklist| {
            checklist
                .steps
                .iter()
                .filter_map(|step| step.plan_step_id.as_deref().map(|id| (id, step.status)))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let current_step = plan
        .execution_contract
        .iter()
        .find(|step| statuses.get(step.id.as_str()) == Some(&ChecklistStatus::InProgress))
        .or_else(|| {
            plan.execution_contract
                .iter()
                .find(|step| statuses.get(step.id.as_str()) != Some(&ChecklistStatus::Completed))
        });
    let remaining_steps = plan
        .execution_contract
        .iter()
        .filter(|step| statuses.get(step.id.as_str()) != Some(&ChecklistStatus::Completed))
        .map(|step| ExecutionStepSummary {
            id: &step.id,
            title: &step.title,
        })
        .collect();
    let completed_step_ids = plan
        .execution_contract
        .iter()
        .filter(|step| statuses.get(step.id.as_str()) == Some(&ChecklistStatus::Completed))
        .map(|step| step.id.as_str())
        .collect();
    let reason_label = match reason {
        ExecutionReminderReason::StepCompleted(_) => "step_completed",
        ExecutionReminderReason::Periodic => "periodic",
        ExecutionReminderReason::CompletionAudit => "completion_audit",
    };
    let payload = serde_json::to_string(&ExecutionReminderPayload {
        plan_id: &plan.id,
        revision: plan.revision,
        reason: reason_label,
        global_reminders: &plan.global_reminders,
        completed_step_ids,
        current_step,
        remaining_steps,
    })
    .ok()?;
    let instruction = match reason {
        ExecutionReminderReason::StepCompleted(_) => {
            "A plan step changed to completed. Check its `done_when` evidence, then continue with the current step; if the evidence is missing, restore the step to `in_progress`."
        }
        ExecutionReminderReason::Periodic => {
            "Stay governed by the approved execution contract. Work on the current step and preserve the global reminders; do not silently replace or omit obligations."
        }
        ExecutionReminderReason::CompletionAudit => {
            "You attempted to finish while the approved execution contract is unresolved. Reconcile every step against its `done_when`, update the typed checklist, and continue working. Do not claim completion from prose or checklist state alone."
        }
    };
    Some(format!(
        "<approved_plan_reminder>{payload}</approved_plan_reminder>\n{instruction}"
    ))
}

pub(crate) fn execution_continuation_note(
    plan: Option<&ProposedPlan>,
    checklist: Option<&ExecutionChecklist>,
) -> Option<String> {
    render_execution_reminder(plan?, checklist, &ExecutionReminderReason::Periodic)
}

pub(crate) struct PlanReminderTransform {
    session: Arc<Mutex<SessionState>>,
}

impl PlanReminderTransform {
    pub(crate) fn new(session: Arc<Mutex<SessionState>>) -> Self {
        Self { session }
    }
}

impl Plugin for PlanReminderTransform {
    fn name(&self) -> &'static str {
        "approved_plan_reminder"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities::context_transform()
    }
}

#[async_trait::async_trait]
impl ContextTransform for PlanReminderTransform {
    async fn transform(
        &self,
        mut messages: Vec<AgentMessage>,
        context: &TransformContext<'_>,
    ) -> Vec<AgentMessage> {
        let reminder = {
            let mut session = self.session.lock().await;
            let reason = session
                .planning
                .pending_execution_reminder
                .take()
                .or_else(|| {
                    (context.iteration > 0
                        && context.iteration % PERIODIC_EXECUTION_REMINDER_TURNS == 0)
                        .then_some(ExecutionReminderReason::Periodic)
                });
            reason.and_then(|reason| {
                render_execution_reminder(
                    session.planning.proposed_plan.as_ref()?,
                    session.planning.execution_checklist.as_ref(),
                    &reason,
                )
            })
        };
        if let Some(reminder) = reminder {
            messages.push(developer_instruction_message(reminder));
        }
        messages
    }
}

pub(crate) struct PlanCompletionGuard {
    session: Arc<Mutex<SessionState>>,
}

impl PlanCompletionGuard {
    pub(crate) fn new(session: Arc<Mutex<SessionState>>) -> Self {
        Self { session }
    }
}

impl Plugin for PlanCompletionGuard {
    fn name(&self) -> &'static str {
        "approved_plan_completion"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities::follow_up()
    }
}

#[async_trait::async_trait]
impl FollowUpSource for PlanCompletionGuard {
    async fn next_follow_up_messages(&self) -> Vec<AgentMessage> {
        let reminder = {
            let mut session = self.session.lock().await;
            let resolved = execution_contract_resolved(
                session.planning.proposed_plan.as_ref(),
                session.planning.execution_checklist.as_ref(),
            );
            if resolved || session.planning.completion_reminders >= MAX_COMPLETION_REMINDERS {
                None
            } else {
                session.planning.completion_reminders =
                    session.planning.completion_reminders.saturating_add(1);
                session.planning.proposed_plan.as_ref().and_then(|plan| {
                    render_execution_reminder(
                        plan,
                        session.planning.execution_checklist.as_ref(),
                        &ExecutionReminderReason::CompletionAudit,
                    )
                })
            }
        };
        reminder
            .map(|content| vec![developer_instruction_message(content)])
            .unwrap_or_default()
    }
}

fn execution_contract_resolved(
    plan: Option<&ProposedPlan>,
    checklist: Option<&ExecutionChecklist>,
) -> bool {
    let Some(plan) = plan.filter(|plan| {
        plan.status == ProposedPlanStatus::Approved && !plan.execution_contract.is_empty()
    }) else {
        return true;
    };
    let Some(checklist) = checklist else {
        return false;
    };
    let statuses = checklist
        .steps
        .iter()
        .filter_map(|step| step.plan_step_id.as_deref().map(|id| (id, step.status)))
        .collect::<HashMap<_, _>>();
    plan.execution_contract
        .iter()
        .all(|step| statuses.get(step.id.as_str()) == Some(&ChecklistStatus::Completed))
}

pub(crate) struct ChecklistUpdate {
    pub checklist: ExecutionChecklist,
    pub explanation: Option<String>,
}

#[cfg(test)]
pub(crate) fn parse_checklist_update(
    args: &Value,
    previous: Option<&ExecutionChecklist>,
) -> Result<ChecklistUpdate, String> {
    parse_checklist_update_for_plan(args, previous, None)
}

pub(crate) fn parse_checklist_update_for_plan(
    args: &Value,
    previous: Option<&ExecutionChecklist>,
    approved_plan: Option<&ProposedPlan>,
) -> Result<ChecklistUpdate, String> {
    let items = args
        .get("plan")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing required array argument `plan`".to_string())?;
    if items.is_empty() {
        return Err("`plan` must contain at least one step".into());
    }

    let mut seen = HashSet::new();
    let mut seen_plan_ids = HashSet::new();
    let mut in_progress = 0usize;
    let mut steps = Vec::with_capacity(items.len());
    for item in items {
        let plan_step_id = item
            .get("plan_step_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(id) = plan_step_id.as_ref() {
            if !seen_plan_ids.insert(id.clone()) {
                return Err(format!("duplicate plan_step_id: {id}"));
            }
        }
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
            plan_step_id,
            title: title.to_string(),
            status,
            priority: None,
        });
    }

    if let Some(plan) = approved_plan.filter(|plan| {
        plan.status == ProposedPlanStatus::Approved && !plan.execution_contract.is_empty()
    }) {
        let expected = plan
            .execution_contract
            .iter()
            .map(|step| (step.id.as_str(), step.title.as_str()))
            .collect::<HashMap<_, _>>();
        if steps.len() != expected.len() {
            return Err(format!(
                "the approved execution contract requires exactly {} checklist steps",
                expected.len()
            ));
        }
        for step in &steps {
            let id = step.plan_step_id.as_deref().ok_or_else(|| {
                "every checklist step requires `plan_step_id` from the approved execution contract"
                    .to_string()
            })?;
            let expected_title = expected
                .get(id)
                .ok_or_else(|| format!("unknown approved plan_step_id: {id}"))?;
            if step.title != *expected_title {
                return Err(format!(
                    "plan_step_id `{id}` must preserve its approved title `{expected_title}`"
                ));
            }
        }
        if seen_plan_ids.len() != expected.len() {
            return Err(
                "the checklist must preserve every approved plan_step_id exactly once".into(),
            );
        }
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
            || previous.steps.iter().zip(&steps).any(|(left, right)| {
                left.plan_step_id != right.plan_step_id || left.title != right.title
            });
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
    use tokio_util::sync::CancellationToken;

    fn approved_structured_plan() -> ProposedPlan {
        ProposedPlan {
            id: "plan-1".into(),
            revision: 2,
            markdown: "1. Implement\n2. Verify".into(),
            status: ProposedPlanStatus::Approved,
            global_reminders: vec!["Preserve compatibility".into()],
            execution_contract: vec![
                PlanExecutionStep {
                    id: "step-1".into(),
                    title: "Implement".into(),
                    files: vec!["src/lib.rs".into()],
                    done_when: vec!["The focused regression test passes".into()],
                    reminders: vec!["Keep the public API stable".into()],
                },
                PlanExecutionStep {
                    id: "step-2".into(),
                    title: "Verify".into(),
                    files: vec!["tests/regression.rs".into()],
                    done_when: vec!["The targeted suite passes".into()],
                    reminders: vec!["Include the failure path".into()],
                },
            ],
        }
    }

    fn approved_checklist(first: ChecklistStatus, second: ChecklistStatus) -> ExecutionChecklist {
        ExecutionChecklist {
            revision: 1,
            steps: vec![
                ChecklistStep {
                    plan_step_id: Some("step-1".into()),
                    title: "Implement".into(),
                    status: first,
                    priority: None,
                },
                ChecklistStep {
                    plan_step_id: Some("step-2".into()),
                    title: "Verify".into(),
                    status: second,
                    priority: None,
                },
            ],
        }
    }

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
    fn structured_proposal_parser_follows_decide_before_render_order() {
        let args = json!({
            "global_reminders": ["Preserve compatibility"],
            "execution_contract": [{
                "title": "Implement",
                "files": ["src/lib.rs"],
                "done_when": ["The regression test passes"],
                "reminders": ["Keep the public API"]
            }],
            "plan": "1. Implement"
        });
        let parsed = parse_proposal_contract(&args).unwrap();
        assert_eq!(parsed.global_reminders, ["Preserve compatibility"]);
        assert_eq!(parsed.execution_contract[0].title, "Implement");
        assert!(parsed.execution_contract[0].id.is_empty());
    }

    #[test]
    fn approved_contract_wire_order_is_stable() {
        let plan = approved_structured_plan();
        let wire = serde_json::to_string(&ApprovedExecutionContract {
            plan_id: &plan.id,
            revision: plan.revision,
            global_reminders: &plan.global_reminders,
            execution_contract: &plan.execution_contract,
        })
        .unwrap();
        let positions = [
            "\"plan_id\"",
            "\"revision\"",
            "\"global_reminders\"",
            "\"execution_contract\"",
            "\"id\"",
            "\"title\"",
            "\"files\"",
            "\"done_when\"",
            "\"reminders\"",
        ]
        .map(|key| wire.find(key).unwrap());
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{wire}");

        let note = plan_mode_exit_note(Some(&plan));
        assert!(
            note.find("<approved_execution_contract>").unwrap()
                < note.find("<approved_plan_markdown>").unwrap()
        );
    }

    #[tokio::test]
    async fn completed_step_queues_a_recency_edge_reminder() {
        let session = Arc::new(Mutex::new(SessionState::default()));
        {
            let mut state = session.lock().await;
            state.planning.proposed_plan = Some(approved_structured_plan());
            state.planning.execution_checklist = Some(approved_checklist(
                ChecklistStatus::InProgress,
                ChecklistStatus::Pending,
            ));
            state.planning.record_checklist_update(approved_checklist(
                ChecklistStatus::Completed,
                ChecklistStatus::InProgress,
            ));
        }
        let transform = PlanReminderTransform::new(session);
        let signal = CancellationToken::new();
        let base = TransformContext::for_test(&signal);
        let context = TransformContext {
            iteration: 1,
            ..base
        };
        let messages = transform.transform(Vec::new(), &context).await;
        assert_eq!(messages.len(), 1);
        let AgentMessage::Custom { payload, .. } = &messages[0] else {
            panic!("expected a typed developer reminder");
        };
        let content = payload["content"].as_str().unwrap();
        assert!(content.contains("\"reason\":\"step_completed\""));
        assert!(content.contains("\"completed_step_ids\":[\"step-1\"]"));
        assert!(content.contains("\"id\":\"step-2\""));
    }

    #[tokio::test]
    async fn periodic_reminder_fires_every_three_model_turns() {
        let session = Arc::new(Mutex::new(SessionState::default()));
        session.lock().await.planning.proposed_plan = Some(approved_structured_plan());
        let transform = PlanReminderTransform::new(session);
        let signal = CancellationToken::new();

        let quiet = transform
            .transform(
                Vec::new(),
                &TransformContext {
                    iteration: 2,
                    ..TransformContext::for_test(&signal)
                },
            )
            .await;
        assert!(quiet.is_empty());

        let reminded = transform
            .transform(
                Vec::new(),
                &TransformContext {
                    iteration: 3,
                    ..TransformContext::for_test(&signal)
                },
            )
            .await;
        assert_eq!(reminded.len(), 1);
        let AgentMessage::Custom { payload, .. } = &reminded[0] else {
            panic!("expected a typed developer reminder");
        };
        assert!(payload["content"]
            .as_str()
            .unwrap()
            .contains("\"reason\":\"periodic\""));
    }

    #[tokio::test]
    async fn completion_guard_reopens_an_unresolved_approved_plan() {
        let session = Arc::new(Mutex::new(SessionState::default()));
        {
            let mut state = session.lock().await;
            state.planning.proposed_plan = Some(approved_structured_plan());
            state.planning.execution_checklist = Some(approved_checklist(
                ChecklistStatus::Completed,
                ChecklistStatus::InProgress,
            ));
        }
        let guard = PlanCompletionGuard::new(session.clone());
        let follow_up = guard.next_follow_up_messages().await;
        assert_eq!(follow_up.len(), 1);
        let AgentMessage::Custom { payload, .. } = &follow_up[0] else {
            panic!("expected a typed completion audit");
        };
        assert!(payload["content"]
            .as_str()
            .unwrap()
            .contains("\"reason\":\"completion_audit\""));

        session.lock().await.planning.execution_checklist = Some(approved_checklist(
            ChecklistStatus::Completed,
            ChecklistStatus::Completed,
        ));
        assert!(guard.next_follow_up_messages().await.is_empty());
    }

    #[test]
    fn plan_mode_contract_is_read_only_and_decision_complete() {
        let prompt = plan_mode_instructions_for(PlanningPromptProfile::DecisionComplete, None);
        assert!(prompt.contains("Propose; do not execute"));
        assert!(prompt.contains("five phases"));
        assert!(prompt.contains("Resolve facts from code"));
        assert!(prompt.contains("provisional implementation model"));
        assert!(prompt.contains("draft -> challenge -> retrieve -> revise"));
        assert!(prompt.contains("wait for the user's"));
        assert!(!prompt.contains("plan.md"));
    }

    #[test]
    fn concise_plan_mode_contract_is_authoritative_and_shorter() {
        let control = plan_mode_instructions_for(PlanningPromptProfile::DecisionComplete, None);
        let candidate = plan_mode_instructions_for(PlanningPromptProfile::Concise, None);
        assert!(candidate.starts_with("<collaboration_mode"));
        assert!(candidate.contains("host collaboration-mode change can end it"));
        assert!(candidate.contains("Work in this order"));
        assert!(candidate.contains("broad orientation before narrow reads"));
        assert!(candidate.contains("do not assume you can know every useful question up front"));
        assert!(candidate.contains("`memory_recall`"));
        assert!(candidate.contains("do not repeat equivalent probes"));
        assert!(candidate.contains("hidden `<proposed_plan>` block"));
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
            global_reminders: Vec::new(),
            execution_contract: Vec::new(),
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
        assert!(reminder.contains("challenge -> retrieve -> revise"));
        assert!(!reminder.contains("Work in this order"));

        state.set_mode(CollaborationMode::Default);
        state.set_mode(CollaborationMode::Plan);
        assert_eq!(
            state.next_plan_instruction_kind(),
            PlanModeInstructionKind::Full
        );
    }

    #[test]
    fn approved_proposals_are_reinjected_byte_for_byte() {
        let plan = ProposedPlan {
            id: "plan-large".into(),
            revision: 1,
            markdown: "a".repeat(10_000),
            status: ProposedPlanStatus::Approved,
            global_reminders: Vec::new(),
            execution_contract: Vec::new(),
        };
        let exit_note = plan_mode_exit_note(Some(&plan));
        assert!(exit_note.contains(&plan.markdown));
        assert!(!exit_note.contains("proposal middle omitted"));
    }

    #[test]
    fn collaboration_policy_is_a_typed_developer_instruction() {
        let message = developer_instruction_message("policy".into());
        assert!(matches!(
            message,
            agent_loop::AgentMessage::Custom { kind, payload, .. }
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
            global_reminders: Vec::new(),
            execution_contract: Vec::new(),
        };
        let note = plan_mode_exit_note(Some(&plan));
        assert!(note.contains("plan-1"));
        assert!(note.contains("Change the boundary"));
        assert!(!note.contains("plan.md"));
    }

    #[test]
    fn hidden_proposed_plan_is_framed_without_semantic_host_parsing() {
        let response = "A short preamble\n<proposed_plan>\n1. Inspect\n2. Verify\n</proposed_plan>";
        assert_eq!(
            extract_proposed_plan(response).as_deref(),
            Some("1. Inspect\n2. Verify")
        );
        assert_eq!(strip_proposed_plan(response), "A short preamble");
        assert!(extract_proposed_plan("<proposed_plan></proposed_plan>").is_none());
    }
}
