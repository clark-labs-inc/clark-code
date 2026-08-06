use serde::Serialize;

use crate::model::{SentinelAction, SentinelDecision, TerminalStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationMode {
    RuntimeSentinel,
    ShadowControl,
    HostBypass,
}

impl InvocationMode {
    pub fn calls_model(self) -> bool {
        self != Self::HostBypass
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostDisposition {
    TerminateCancelled,
    TerminateVerificationIncomplete,
    InvokeSentinel,
    NoSentinel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct StopEnforcement {
    pub effective_action: SentinelAction,
    pub stop_accepted: bool,
    pub rejection_reason: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EventFact {
    pub id: String,
    pub fact: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LoopPacket {
    pub schema_version: u32,
    pub conversation_id: Option<String>,
    pub observed_at: String,
    pub terminal_answer_committed: bool,
    pub host_follow_up_requested: bool,
    pub user_cancel_requested: bool,
    pub assistant_only_streak: u32,
    pub completed_tool_count: u32,
    pub failed_tool_count: u32,
    pub new_state_delta_since_last_response: bool,
    pub novel_evidence_since_last_response: bool,
    pub new_hypothesis_or_target: bool,
    pub cycle_signature_repeat_count: u32,
    pub finite_exploration_active: bool,
    pub exploration_frontier_remaining: u32,
    pub unresolved_effect_count: u32,
    pub verification_attempts: u32,
    pub verification_retry_budget: u32,
    pub verification_tool_available: bool,
    pub pending_obligation: Option<String>,
    pub events: Vec<EventFact>,
}

impl LoopPacket {
    pub fn has_event(&self, id: &str) -> bool {
        self.events.iter().any(|event| event.id == id)
    }
}

#[derive(Clone)]
pub struct Scenario {
    pub id: &'static str,
    pub source: &'static str,
    pub invocation: InvocationMode,
    pub expected_host_disposition: HostDisposition,
    pub packet: LoopPacket,
    pub expected_action: Option<SentinelAction>,
    pub allowed_terminal_statuses: Vec<TerminalStatus>,
}

pub fn host_disposition(packet: &LoopPacket) -> HostDisposition {
    if packet.user_cancel_requested {
        return HostDisposition::TerminateCancelled;
    }
    if packet.unresolved_effect_count > 0
        && packet.verification_attempts >= packet.verification_retry_budget
    {
        return HostDisposition::TerminateVerificationIncomplete;
    }
    if packet.new_state_delta_since_last_response
        || packet.novel_evidence_since_last_response
        || packet.new_hypothesis_or_target
    {
        return HostDisposition::NoSentinel;
    }
    if (packet.terminal_answer_committed && packet.host_follow_up_requested)
        || packet.assistant_only_streak >= 2
        || packet.cycle_signature_repeat_count >= 3
    {
        return HostDisposition::InvokeSentinel;
    }
    HostDisposition::NoSentinel
}

pub fn enforce_decision(packet: &LoopPacket, decision: &SentinelDecision) -> StopEnforcement {
    if decision.decision.action() == SentinelAction::DeferToHost {
        return StopEnforcement {
            effective_action: SentinelAction::DeferToHost,
            stop_accepted: false,
            rejection_reason: None,
        };
    }
    let rejection_reason = match decision.decision.terminal_status() {
        TerminalStatus::Done
            if packet.terminal_answer_committed && packet.host_follow_up_requested =>
        {
            None
        }
        TerminalStatus::Cancelled if packet.user_cancel_requested => None,
        TerminalStatus::VerificationIncomplete
            if packet.unresolved_effect_count > 0
                && (packet.verification_attempts >= packet.verification_retry_budget
                    || (!packet.verification_tool_available
                        && (packet.assistant_only_streak >= 2
                            || packet.terminal_answer_committed))) =>
        {
            None
        }
        TerminalStatus::StalledNoProgress
            if packet.cycle_signature_repeat_count >= 3
                && !packet.new_state_delta_since_last_response
                && !packet.novel_evidence_since_last_response
                && !packet.new_hypothesis_or_target =>
        {
            None
        }
        TerminalStatus::Done => Some("done_requires_committed_terminal_answer_and_host_reprompt"),
        TerminalStatus::Cancelled => Some("cancelled_requires_native_user_cancellation"),
        TerminalStatus::VerificationIncomplete => {
            Some("verification_stop_requires_exhaustion_or_no_recovery_path")
        }
        TerminalStatus::StalledNoProgress => {
            Some("stalled_requires_repeated_whole_state_cycle_without_novelty")
        }
        TerminalStatus::NotTerminal => Some("stop_cannot_use_not_terminal_status"),
    };
    StopEnforcement {
        effective_action: if rejection_reason.is_none() {
            SentinelAction::Stop
        } else {
            SentinelAction::DeferToHost
        },
        stop_accepted: rejection_reason.is_none(),
        rejection_reason,
    }
}

fn event(id: &str, fact: &str) -> EventFact {
    EventFact {
        id: id.into(),
        fact: fact.into(),
    }
}

fn packet(events: Vec<EventFact>) -> LoopPacket {
    LoopPacket {
        schema_version: 1,
        conversation_id: None,
        observed_at: "synthetic-control".into(),
        terminal_answer_committed: false,
        host_follow_up_requested: false,
        user_cancel_requested: false,
        assistant_only_streak: 0,
        completed_tool_count: 0,
        failed_tool_count: 0,
        new_state_delta_since_last_response: false,
        novel_evidence_since_last_response: false,
        new_hypothesis_or_target: false,
        cycle_signature_repeat_count: 0,
        finite_exploration_active: false,
        exploration_frontier_remaining: 0,
        unresolved_effect_count: 0,
        verification_attempts: 0,
        verification_retry_budget: 3,
        verification_tool_available: false,
        pending_obligation: None,
        events,
    }
}

pub fn scenarios() -> Vec<Scenario> {
    let mut incident = packet(vec![
        event(
            "prod-final-182903036",
            "A complete user-facing final answer was committed at 2026-08-02T18:29:03.036Z.",
        ),
        event(
            "prod-git-classification",
            "Seven completed inspections were git -C status/diff/log commands; no write tool or state-changing Git subcommand completed.",
        ),
        event(
            "prod-completion-hook",
            "The host completion hook requested another model turn solely because those inspections were classified as unresolved effects.",
        ),
        event(
            "prod-tool-gate",
            "The requested verify_effect recovery tool was not available to that model turn.",
        ),
    ]);
    incident.conversation_id = Some("bf36da49-7925-4e4d-a315-e929b314907c".into());
    incident.observed_at = "2026-08-02T18:29:03.036Z".into();
    incident.terminal_answer_committed = true;
    incident.host_follow_up_requested = true;
    incident.assistant_only_streak = 1;
    incident.completed_tool_count = 7;
    incident.unresolved_effect_count = 7;
    incident.verification_tool_available = false;
    incident.pending_obligation = Some(
        "Host claims seven effects need verification, but the event record shows read-only Git inspection and no mutation target.".into(),
    );

    let mut recursive = packet(vec![
        event(
            "prod-terminal-answer",
            "A terminal answer was already visible before the follow-up sequence.",
        ),
        event(
            "prod-prose-01",
            "A host follow-up produced assistant prose with no tool call or state delta.",
        ),
        event(
            "prod-prose-02",
            "The next follow-up again produced assistant prose with no tool call or state delta.",
        ),
        event(
            "prod-prose-context",
            "The assistant began explaining its own prior malformed narrative; no new user requirement appeared.",
        ),
    ]);
    recursive.conversation_id = Some("bf36da49-7925-4e4d-a315-e929b314907c".into());
    recursive.observed_at = "2026-08-02T18:43:42.727Z".into();
    recursive.terminal_answer_committed = true;
    recursive.host_follow_up_requested = true;
    recursive.assistant_only_streak = 2;
    recursive.completed_tool_count = 7;
    recursive.unresolved_effect_count = 7;
    recursive.verification_tool_available = false;
    recursive.pending_obligation = Some("Host repeats an unavailable verification request.".into());

    let mut repeated_cycle = packet(vec![
        event(
            "cycle-01",
            "The same tool, arguments, result, pending obligations, and control state recurred.",
        ),
        event(
            "cycle-02",
            "That whole-state signature recurred a second time with no new evidence or hypothesis.",
        ),
        event(
            "cycle-03",
            "That whole-state signature recurred a third time with no frontier change.",
        ),
        event(
            "cycle-04",
            "A fourth identical state was reached; the next proposed action is the same action again.",
        ),
    ]);
    repeated_cycle.completed_tool_count = 4;
    repeated_cycle.failed_tool_count = 4;
    repeated_cycle.cycle_signature_repeat_count = 4;
    repeated_cycle.finite_exploration_active = true;
    repeated_cycle.exploration_frontier_remaining = 6;
    repeated_cycle.pending_obligation = Some(
        "Exploration has other untried frontier items, but the executor keeps selecting the same failed action.".into(),
    );

    let mut bounded_recovery = packet(vec![
        event(
            "effect-write",
            "A state-changing external command completed once.",
        ),
        event(
            "verification-missing",
            "No canonical read-back has occurred yet.",
        ),
        event(
            "recovery-tool-ready",
            "verify_effect is available and one bounded recovery turn remains.",
        ),
    ]);
    bounded_recovery.assistant_only_streak = 2;
    bounded_recovery.unresolved_effect_count = 1;
    bounded_recovery.verification_attempts = 0;
    bounded_recovery.verification_retry_budget = 1;
    bounded_recovery.verification_tool_available = true;
    bounded_recovery.pending_obligation = Some(
        "Verify one concrete external target with the already available recovery tool.".into(),
    );

    let mut productive = packet(vec![
        event(
            "productive-step-159",
            "read_file step-159 completed and returned new fixture evidence.",
        ),
        event(
            "productive-step-160",
            "read_file step-160 completed and returned distinct new fixture evidence.",
        ),
        event(
            "productive-next",
            "The declared finite sequence has one terminal response remaining.",
        ),
    ]);
    productive.completed_tool_count = 160;
    productive.new_state_delta_since_last_response = true;
    productive.pending_obligation =
        Some("Emit the already-prepared terminal completion sentinel.".into());

    let mut missing_file = packet(vec![
        event(
            "missing-read-request",
            "The task explicitly required one read of intentionally-absent.txt.",
        ),
        event(
            "missing-read-result",
            "read_file returned the expected not-found result exactly once.",
        ),
        event(
            "missing-task-contract",
            "The finite next step is one specified terminal response with no further tool call.",
        ),
    ]);
    missing_file.completed_tool_count = 1;
    missing_file.failed_tool_count = 1;
    missing_file.novel_evidence_since_last_response = true;
    missing_file.pending_obligation = Some("Return the specified terminal sentinel once.".into());

    let mut exploratory_failures = packet(vec![
        event(
            "explore-21",
            "Failed probe 21 ruled out a distinct cache-key hypothesis.",
        ),
        event(
            "explore-22",
            "Failed probe 22 ruled out a different serialization boundary.",
        ),
        event(
            "explore-23",
            "Failed probe 23 narrowed the fault to two remaining provider paths.",
        ),
        event(
            "explore-24",
            "Failed probe 24 tested a new provider path and produced a distinct error signature.",
        ),
    ]);
    exploratory_failures.completed_tool_count = 24;
    exploratory_failures.failed_tool_count = 24;
    exploratory_failures.novel_evidence_since_last_response = true;
    exploratory_failures.new_hypothesis_or_target = true;
    exploratory_failures.cycle_signature_repeat_count = 1;
    exploratory_failures.finite_exploration_active = true;
    exploratory_failures.exploration_frontier_remaining = 2;
    exploratory_failures.pending_obligation = Some(
        "Test the two remaining distinct provider-path hypotheses, then synthesize the evidence."
            .into(),
    );

    let mut cancelled = packet(vec![
        event(
            "user-stop-184342727",
            "The active user input exactly matched the reserved stop command.",
        ),
        event(
            "cancel-contract",
            "Exact stop is a native cancellation boundary and must not be sent to the model.",
        ),
    ]);
    cancelled.conversation_id = Some("bf36da49-7925-4e4d-a315-e929b314907c".into());
    cancelled.observed_at = "2026-08-02T18:43:42.727Z".into();
    cancelled.user_cancel_requested = true;

    let mut exhausted = packet(vec![
        event(
            "effect-unresolved",
            "One external effect remains unverified.",
        ),
        event("recovery-01", "Recovery turn one returned prose only."),
        event("recovery-02", "Recovery turn two returned prose only."),
        event(
            "recovery-03",
            "Recovery turn three returned prose only; the budget is exhausted.",
        ),
    ]);
    exhausted.unresolved_effect_count = 1;
    exhausted.verification_attempts = 3;
    exhausted.verification_retry_budget = 3;

    vec![
        Scenario {
            id: "production_false_effect_guard",
            source: "production trajectory bf36da49 at first false completion block",
            invocation: InvocationMode::RuntimeSentinel,
            expected_host_disposition: HostDisposition::InvokeSentinel,
            packet: incident,
            expected_action: Some(SentinelAction::Stop),
            allowed_terminal_statuses: vec![
                TerminalStatus::Done,
                TerminalStatus::VerificationIncomplete,
            ],
        },
        Scenario {
            id: "production_recursive_prose",
            source: "production trajectory bf36da49 after repeated assistant-only output",
            invocation: InvocationMode::RuntimeSentinel,
            expected_host_disposition: HostDisposition::InvokeSentinel,
            packet: recursive,
            expected_action: Some(SentinelAction::Stop),
            allowed_terminal_statuses: vec![
                TerminalStatus::Done,
                TerminalStatus::VerificationIncomplete,
            ],
        },
        Scenario {
            id: "repeated_whole_state_cycle",
            source: "deterministic zero-novelty cycle control",
            invocation: InvocationMode::RuntimeSentinel,
            expected_host_disposition: HostDisposition::InvokeSentinel,
            packet: repeated_cycle,
            expected_action: Some(SentinelAction::Stop),
            allowed_terminal_statuses: vec![TerminalStatus::StalledNoProgress],
        },
        Scenario {
            id: "one_bounded_recovery_remaining",
            source: "deterministic recovery control",
            invocation: InvocationMode::RuntimeSentinel,
            expected_host_disposition: HostDisposition::InvokeSentinel,
            packet: bounded_recovery,
            expected_action: Some(SentinelAction::DeferToHost),
            allowed_terminal_statuses: vec![TerminalStatus::NotTerminal],
        },
        Scenario {
            id: "productive_160_step_run",
            source: "loop_termination 160-iteration productive control",
            invocation: InvocationMode::ShadowControl,
            expected_host_disposition: HostDisposition::NoSentinel,
            packet: productive,
            expected_action: Some(SentinelAction::DeferToHost),
            allowed_terminal_statuses: vec![TerminalStatus::NotTerminal],
        },
        Scenario {
            id: "expected_missing_file_progress",
            source: "free_tier_stress missing-file self-stop control",
            invocation: InvocationMode::ShadowControl,
            expected_host_disposition: HostDisposition::NoSentinel,
            packet: missing_file,
            expected_action: Some(SentinelAction::DeferToHost),
            allowed_terminal_statuses: vec![TerminalStatus::NotTerminal],
        },
        Scenario {
            id: "twenty_four_failed_exploration_turns",
            source: "long unsuccessful but novel exploration control",
            invocation: InvocationMode::ShadowControl,
            expected_host_disposition: HostDisposition::NoSentinel,
            packet: exploratory_failures,
            expected_action: Some(SentinelAction::DeferToHost),
            allowed_terminal_statuses: vec![TerminalStatus::NotTerminal],
        },
        Scenario {
            id: "exact_user_stop_bypass",
            source: "production native cancellation contract",
            invocation: InvocationMode::HostBypass,
            expected_host_disposition: HostDisposition::TerminateCancelled,
            packet: cancelled,
            expected_action: None,
            allowed_terminal_statuses: vec![TerminalStatus::Cancelled],
        },
        Scenario {
            id: "verification_budget_host_stop",
            source: "loop_termination bounded unresolved-effect control",
            invocation: InvocationMode::HostBypass,
            expected_host_disposition: HostDisposition::TerminateVerificationIncomplete,
            packet: exhausted,
            expected_action: None,
            allowed_terminal_statuses: vec![TerminalStatus::VerificationIncomplete],
        },
    ]
}
