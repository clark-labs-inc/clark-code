use super::*;

fn run(id: &str) -> RunId {
    RunId::new(id)
}

#[test]
fn successful_execution_stays_pending_until_independent_verification() {
    let mut ledger = EffectLedger::default();
    ledger.register(
        run("run-1"),
        "call-1",
        "publisher",
        EffectIntent::opaque_external("published a user-facing resource"),
    );

    let prompt = ledger
        .completion_prompt(&run("run-1"))
        .expect("completion must block");
    assert!(prompt.contains("call-1"));
    assert!(prompt.contains("canonical state"));
    assert!(prompt.contains("`effect` to `none`"));

    ledger
        .verify(
            "call-1",
            EffectVerification::Verified,
            "Read the canonical resource and confirmed its complete body".into(),
        )
        .unwrap();
    assert!(ledger.completion_prompt(&run("run-1")).is_none());
}

#[test]
fn one_character_canonical_result_is_a_mismatch_and_keeps_gate_closed() {
    let mut ledger = EffectLedger::default();
    ledger.register(
        run("run-1"),
        "call-2",
        "publisher",
        EffectIntent::opaque_external("published a user-facing resource"),
    );
    ledger
        .verify(
            "call-2",
            EffectVerification::Mismatch,
            "Canonical read-back was `-`, not the requested explanation and tests".into(),
        )
        .unwrap();

    let prompt = ledger
        .completion_prompt(&run("run-1"))
        .expect("mismatch must block");
    assert!(prompt.contains("Mismatch"));
    assert_eq!(
        ledger.get("call-2").unwrap().verification,
        EffectVerification::Mismatch
    );
}

#[test]
fn explicit_unverifiable_receipt_allows_honest_completion() {
    let mut ledger = EffectLedger::default();
    ledger.register(
        run("run-1"),
        "call-3",
        "publisher",
        EffectIntent::opaque_external("sent a remote request"),
    );
    ledger
        .verify(
            "call-3",
            EffectVerification::Unverifiable,
            "Provider exposes no read endpoint; only request id req-7 is available".into(),
        )
        .unwrap();
    assert!(ledger.completion_prompt(&run("run-1")).is_none());
    assert!(!ledger.has_unresolved());
}

#[test]
fn unresolved_state_tracks_pending_and_mismatched_receipts() {
    let mut ledger = EffectLedger::default();
    ledger.register(
        run("run-1"),
        "call-1",
        "publisher",
        EffectIntent::opaque_external("published a resource"),
    );
    assert!(ledger.has_unresolved());

    ledger
        .verify(
            "call-1",
            EffectVerification::Verified,
            "Canonical read-back matched".into(),
        )
        .unwrap();
    assert!(!ledger.has_unresolved());
}

#[test]
fn repeated_completion_attempts_narrow_the_next_turn_to_verification() {
    let mut ledger = EffectLedger::default();
    ledger.register(
        run("run-1"),
        "call-1",
        "publisher",
        EffectIntent::opaque_external("published a resource"),
    );

    assert!(!ledger.requires_verification_only_turn(&run("run-1")));
    assert!(ledger.completion_prompt(&run("run-1")).is_some());
    assert!(!ledger.requires_verification_only_turn(&run("run-1")));
    assert!(ledger.completion_prompt(&run("run-1")).is_some());
    assert!(ledger.requires_verification_only_turn(&run("run-1")));
}

#[tokio::test]
async fn fake_publisher_success_then_one_character_readback_forces_follow_up() {
    let session = Arc::new(Mutex::new(SessionState::default()));
    let receipt = session.lock().await.effects.register(
        run("run-1"),
        "publish-1",
        "fake_publisher",
        EffectIntent::declared_external(
            EffectAction::Publish,
            Some("resource://example".into()),
            "published a user-facing resource",
        ),
    );
    let mut successful_execution = ToolOutcome::ok("exit_code: 0\nresource://example");
    attach_pending_receipt(&mut successful_execution, &receipt);
    assert!(successful_execution
        .content
        .contains("verification required"));

    session
        .lock()
        .await
        .effects
        .verify(
            "publish-1",
            EffectVerification::Mismatch,
            "Canonical read-back contained only `-`".into(),
        )
        .unwrap();

    let follow_up = EffectCompletionGuard::new(session, run("run-1"))
        .next_follow_up_messages()
        .await;
    assert_eq!(follow_up.len(), 1);
    let AgentMessage::System { content, .. } = &follow_up[0] else {
        panic!("completion gate must inject system guidance");
    };
    assert!(content.contains("cannot finish yet"));
    assert!(content.contains("Repair any mismatch"));
}

#[tokio::test]
async fn unresolved_receipts_only_gate_their_originating_run() {
    let session = Arc::new(Mutex::new(SessionState::default()));
    session.lock().await.effects.register(
        run("run-1"),
        "publish-1",
        "fake_publisher",
        EffectIntent::opaque_external("published a user-facing resource"),
    );

    let original_follow_up = EffectCompletionGuard::new(session.clone(), run("run-1"))
        .next_follow_up_messages()
        .await;
    let later_follow_up = EffectCompletionGuard::new(session.clone(), run("run-2"))
        .next_follow_up_messages()
        .await;

    assert_eq!(original_follow_up.len(), 1);
    assert!(later_follow_up.is_empty());
    assert_eq!(
        session
            .lock()
            .await
            .effects
            .unresolved_diagnostics(&run("run-1"))
            .len(),
        1
    );
    assert_eq!(
        session
            .lock()
            .await
            .effects
            .unresolved_diagnostics(&run("run-2"))
            .len(),
        0
    );
}

#[tokio::test]
async fn file_delivery_is_denied_with_exact_pending_receipt_diagnostics() {
    let session = Arc::new(Mutex::new(SessionState::default()));
    session.lock().await.effects.register(
        run("run-1"),
        "publish-7",
        "fake_publisher",
        EffectIntent::declared_external(
            EffectAction::Publish,
            Some("resource://example".into()),
            "published the requested resource",
        ),
    );
    let guard = EffectCompletionGuard::new(session, run("run-1"));
    let reason = guard
        .denial_reason(
            "deliver_files",
            ToolGateContext {
                iteration: 1,
                messages: &[],
                conversation_id: Some("conversation"),
                available_tool_names: &["deliver_files", "verify_effect"],
            },
        )
        .await
        .expect("pending receipt must block final publication");

    assert!(reason.contains("publish-7"));
    assert!(reason.contains("fake_publisher"));
    assert!(reason.contains("published the requested resource"));
    assert!(reason.contains("verify_effect"));
}
