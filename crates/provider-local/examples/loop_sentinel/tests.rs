use crate::model::{
    Confidence, ReasonCode, SentinelAction, SentinelDecision, SentinelVerdict, TerminalStatus,
};
use crate::policy::{
    enforce_decision, host_disposition, scenarios, HostDisposition, InvocationMode,
};
use crate::turn::system_prompt;

#[test]
fn hard_host_boundaries_never_depend_on_the_model() {
    let scenarios = scenarios();
    let cancelled = scenarios
        .iter()
        .find(|scenario| scenario.id == "exact_user_stop_bypass")
        .unwrap();
    assert_eq!(cancelled.invocation, InvocationMode::HostBypass);
    assert_eq!(
        host_disposition(&cancelled.packet),
        HostDisposition::TerminateCancelled
    );
    let exhausted = scenarios
        .iter()
        .find(|scenario| scenario.id == "verification_budget_host_stop")
        .unwrap();
    assert_eq!(exhausted.invocation, InvocationMode::HostBypass);
    assert_eq!(
        host_disposition(&exhausted.packet),
        HostDisposition::TerminateVerificationIncomplete
    );
}

#[test]
fn productive_progress_does_not_trigger_the_runtime_sentinel() {
    for id in [
        "productive_160_step_run",
        "expected_missing_file_progress",
        "twenty_four_failed_exploration_turns",
    ] {
        let scenario = scenarios()
            .into_iter()
            .find(|scenario| scenario.id == id)
            .unwrap();
        assert_eq!(scenario.invocation, InvocationMode::ShadowControl);
        assert_eq!(
            host_disposition(&scenario.packet),
            HostDisposition::NoSentinel
        );
        assert_eq!(scenario.expected_action, Some(SentinelAction::DeferToHost));
        assert_eq!(
            scenario.allowed_terminal_statuses,
            [TerminalStatus::NotTerminal]
        );
    }
}

#[test]
fn ambiguous_non_progress_is_the_only_model_trigger() {
    for scenario in scenarios()
        .into_iter()
        .filter(|scenario| scenario.invocation == InvocationMode::RuntimeSentinel)
    {
        assert_eq!(
            host_disposition(&scenario.packet),
            HostDisposition::InvokeSentinel,
            "{}",
            scenario.id
        );
    }
}

#[test]
fn sentinel_contract_forbids_execution_and_recursive_review() {
    let prompt = system_prompt();
    assert!(prompt.contains("not an outcome-quality critic"));
    assert!(prompt.contains("Never improve the answer"));
    assert!(prompt.contains("exactly once"));
    assert!(prompt.contains("emit no prose"));
    assert!(prompt.contains("can never extend"));
    assert!(prompt.contains("host will reject any stop"));
    assert!(prompt.contains("Failure count alone is never a stop reason"));
    assert!(prompt.contains("dozens of failed turns"));
    assert!(prompt.contains("has not yet been attempted"));
}

fn stop(status: TerminalStatus, event_id: &str) -> SentinelDecision {
    let decision = match status {
        TerminalStatus::Done => SentinelVerdict::StopDone,
        TerminalStatus::Cancelled => SentinelVerdict::StopCancelled,
        TerminalStatus::VerificationIncomplete => SentinelVerdict::StopVerificationIncomplete,
        TerminalStatus::StalledNoProgress => SentinelVerdict::StopStalledNoProgress,
        TerminalStatus::NotTerminal => SentinelVerdict::DeferToHost,
    };
    SentinelDecision {
        decision,
        reason_code: ReasonCode::InsufficientEvidence,
        confidence: Confidence::Low,
        evidence_event_ids: vec![event_id.into()],
    }
}

#[test]
fn host_rejects_false_stops_but_accepts_fact_backed_stops() {
    let scenarios = scenarios();
    let missing = scenarios
        .iter()
        .find(|scenario| scenario.id == "expected_missing_file_progress")
        .unwrap();
    let rejected = enforce_decision(
        &missing.packet,
        &stop(TerminalStatus::Done, "missing-read-result"),
    );
    assert_eq!(rejected.effective_action, SentinelAction::DeferToHost);
    assert!(!rejected.stop_accepted);

    let exploration = scenarios
        .iter()
        .find(|scenario| scenario.id == "twenty_four_failed_exploration_turns")
        .unwrap();
    let rejected = enforce_decision(
        &exploration.packet,
        &stop(TerminalStatus::StalledNoProgress, "explore-24"),
    );
    assert_eq!(rejected.effective_action, SentinelAction::DeferToHost);

    let incident = scenarios
        .iter()
        .find(|scenario| scenario.id == "production_false_effect_guard")
        .unwrap();
    let accepted = enforce_decision(
        &incident.packet,
        &stop(TerminalStatus::Done, "prod-final-182903036"),
    );
    assert_eq!(accepted.effective_action, SentinelAction::Stop);
    assert!(accepted.stop_accepted);

    let cycle = scenarios
        .iter()
        .find(|scenario| scenario.id == "repeated_whole_state_cycle")
        .unwrap();
    let accepted = enforce_decision(
        &cycle.packet,
        &stop(TerminalStatus::StalledNoProgress, "cycle-04"),
    );
    assert_eq!(accepted.effective_action, SentinelAction::Stop);
    assert!(accepted.stop_accepted);
}

#[test]
fn many_failed_turns_with_novelty_remain_productive() {
    let scenario = scenarios()
        .into_iter()
        .find(|scenario| scenario.id == "twenty_four_failed_exploration_turns")
        .unwrap();
    assert_eq!(scenario.packet.failed_tool_count, 24);
    assert!(scenario.packet.novel_evidence_since_last_response);
    assert!(scenario.packet.new_hypothesis_or_target);
    assert_eq!(
        host_disposition(&scenario.packet),
        HostDisposition::NoSentinel
    );
}

#[test]
fn every_scenario_has_unique_citable_events() {
    for scenario in scenarios() {
        let mut ids = scenario
            .packet
            .events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "{}", scenario.id);
        assert!(!ids.is_empty(), "{}", scenario.id);
    }
}
