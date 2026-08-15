use super::*;

fn ledger() -> ExecutionLedger {
    ExecutionLedger::new_root(
        ExecutionId::new("run-1").unwrap(),
        ExecutionPolicy::default(),
    )
    .unwrap()
}

#[test]
fn root_execution_replays_to_the_same_snapshot() {
    let ledger = ledger();
    ledger.start_attempt().unwrap();
    ledger.checkpoint("checkpoint-1").unwrap();
    ledger.record_steering().unwrap();
    ledger.tool_started("tool-1", "edit_file", true).unwrap();
    ledger
        .tool_finished(
            "tool-1",
            ToolExecutionStatus::Completed,
            BTreeSet::from(["src/lib.rs".to_string()]),
        )
        .unwrap();
    ledger
        .record_usage(UsageCharge {
            input_tokens: 100,
            output_tokens: 25,
            cost_usd: 0.01,
            ..Default::default()
        })
        .unwrap();
    ledger.transition(ExecutionState::Verifying, None).unwrap();
    ledger
        .finalize_evidence(EvidenceReceipt {
            changed_paths: BTreeSet::from(["src/lib.rs".to_string()]),
            ..Default::default()
        })
        .unwrap();
    ledger.transition(ExecutionState::Completed, None).unwrap();

    let snapshot = ledger.snapshot();
    assert_eq!(ExecutionLedger::replay(&ledger.events()).unwrap(), snapshot);
    let encoded = serde_json::to_vec(&ledger.events()).unwrap();
    let decoded: Vec<ExecutionEvent> = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, ledger.events());
    assert_eq!(snapshot.writer, AgentPath::root());
    assert_eq!(snapshot.attempts.len(), 1);
    assert_eq!(snapshot.steering_messages, 1);
    assert_eq!(snapshot.usage.weighted_tokens, 200.0);
    assert!(snapshot.evidence.changed_paths.contains("src/lib.rs"));
}

#[test]
fn recovery_requires_a_transient_failure_and_keeps_the_proven_tool_boundary() {
    let ledger = ledger();
    ledger.start_attempt().unwrap();
    ledger.tool_started("read-1", "read_file", false).unwrap();
    assert!(
        !ledger
            .recovery_decision(FailureClass::TransientTransport)
            .allowed
    );
    ledger
        .tool_finished("read-1", ToolExecutionStatus::Completed, BTreeSet::new())
        .unwrap();
    ledger.tool_started("tool-1", "write_file", true).unwrap();
    assert!(
        !ledger
            .recovery_decision(FailureClass::TransientTransport)
            .allowed
    );
    ledger
        .tool_finished("tool-1", ToolExecutionStatus::Completed, BTreeSet::new())
        .unwrap();
    assert!(!ledger.recovery_decision(FailureClass::Tool).allowed);
    assert!(
        ledger
            .recovery_decision(FailureClass::TransientTransport)
            .allowed
    );
    ledger
        .schedule_recovery(FailureClass::TransientTransport, "connection reset")
        .unwrap();
    ledger.start_attempt().unwrap();
    assert!(
        ledger
            .recovery_decision(FailureClass::TransientTransport)
            .allowed
    );
}

#[test]
fn root_recovery_has_no_implicit_attempt_ceiling() {
    let ledger = ledger();
    ledger.start_attempt().unwrap();

    for attempt in 1..=8 {
        assert!(
            ledger
                .recovery_decision(FailureClass::TransientTransport)
                .allowed,
            "attempt {attempt} must remain recoverable"
        );
        ledger
            .schedule_recovery(FailureClass::TransientTransport, "temporary outage")
            .unwrap();
        ledger.start_attempt().unwrap();
    }

    assert_eq!(ledger.snapshot().attempts.len(), 9);
}

#[test]
fn awaiting_permission_is_not_a_recovery_boundary() {
    let ledger = ledger();
    ledger.start_attempt().unwrap();
    ledger
        .transition(ExecutionState::AwaitingInput, None)
        .unwrap();
    let decision = ledger.recovery_decision(FailureClass::TransientTransport);
    assert!(!decision.allowed);
    assert!(decision
        .reasons
        .iter()
        .any(|reason| reason.contains("not at a running boundary")));
}

#[test]
fn children_attach_beneath_the_same_root_identity_without_a_writer_role() {
    let ledger = ledger();
    let path = AgentPath::parse("/root/reviewer").unwrap();
    ledger
        .attach_child(path.clone(), AgentRole::Reviewer)
        .unwrap();
    ledger
        .update_child(path.clone(), AgentStatus::Running)
        .unwrap();
    let snapshot = ledger.snapshot();
    assert_eq!(snapshot.children[&path].role, AgentRole::Reviewer);
    assert_eq!(snapshot.writer, AgentPath::root());
}
