use std::collections::BTreeSet;

mod fixtures;

use super::*;
use fixtures::*;

#[test]
fn complete_run_replays_with_the_same_fingerprint() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    issue_submit(
        &mut ledger,
        envelope(
            "mapper",
            ScoutRole::Mapper,
            vec![artifact(
                "source-1",
                EvidenceKind::SourceTrace,
                Some(ProofTier::T1Source),
            )],
            vec![proposal(true, true, ProofTier::T1Source)],
            vec![],
        ),
    )
    .unwrap();
    check_exact(&mut ledger, "source-1", "root-source-check");

    ledger.advance(ScoutPhase::Measure).unwrap();
    issue_submit(
        &mut ledger,
        envelope(
            "measurer",
            ScoutRole::Measurer,
            vec![measurement_artifact("measurement-1", Some(0.9))],
            vec![],
            vec![ClaimUpdate {
                claim_id: ClaimId::new("claim-1").unwrap(),
                evidence: evidence_ids(&["measurement-1"]),
                counterevidence: BTreeSet::new(),
                limitations: Vec::new(),
            }],
        ),
    )
    .unwrap();
    check_exact(&mut ledger, "measurement-1", "root-measure-check");

    ledger.advance(ScoutPhase::Check).unwrap();
    let source_hash = ledger
        .snapshot()
        .evidence
        .get(&EvidenceId::new("source-1").unwrap())
        .unwrap()
        .artifact
        .content_sha256
        .clone();
    let mut reproduction = artifact("reproduction-1", EvidenceKind::Reproduction, None);
    reproduction.content_sha256 = source_hash;
    reproduction.reproduces = Some(EvidenceId::new("source-1").unwrap());
    issue_submit(
        &mut ledger,
        envelope(
            "reproducer",
            ScoutRole::Reproducer,
            vec![reproduction],
            vec![],
            vec![ClaimUpdate {
                claim_id: ClaimId::new("claim-1").unwrap(),
                evidence: evidence_ids(&["reproduction-1"]),
                counterevidence: BTreeSet::new(),
                limitations: Vec::new(),
            }],
        ),
    )
    .unwrap();
    check_exact(&mut ledger, "reproduction-1", "independent-replay");

    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    ledger
        .adjudicate(Adjudication {
            claim_id: ClaimId::new("claim-1").unwrap(),
            verdict: ScoutVerdict::Supported,
            test: "independent source, measurement, and reproduction replay".into(),
            reason: "all pinned receipts matched".into(),
            proof_tier: Some(ProofTier::T1Source),
            addressed_counterevidence: BTreeSet::new(),
            instrument_needed: None,
        })
        .unwrap();
    ledger.advance(ScoutPhase::Synthesize).unwrap();
    ledger.seal(SealDisposition::Complete).unwrap();

    let fingerprint = ledger.fingerprint().unwrap();
    let replayed = ScoutLedger::replay(ledger.events().to_vec()).unwrap();
    assert_eq!(replayed.snapshot(), ledger.snapshot());
    assert_eq!(replayed.fingerprint().unwrap(), fingerprint);
    let report = replayed.report_markdown();
    assert!(report.contains("SUPPORTED"));
    assert!(report.contains("Capability census: `census-test`"));
    assert!(report.contains("Measurement: n=100"));
    assert!(report.contains("Ledger SHA-256:"));
}

#[test]
fn assignments_are_host_issued_and_actor_spoofing_fails_replay() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    let worker = envelope("mapper", ScoutRole::Mapper, vec![], vec![], vec![]);
    assert!(ledger
        .submit(worker.clone())
        .unwrap_err()
        .contains("unissued"));
    issue_submit(&mut ledger, worker).unwrap();

    let mut events = ledger.events().to_vec();
    events.last_mut().unwrap().actor = ScoutActor::Root;
    assert!(ScoutLedger::replay(events)
        .unwrap_err()
        .contains("not authorized"));
}

#[test]
fn worker_evidence_cannot_support_a_claim_until_host_checked() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    issue_submit(
        &mut ledger,
        envelope(
            "mapper",
            ScoutRole::Mapper,
            vec![artifact(
                "source-1",
                EvidenceKind::SourceTrace,
                Some(ProofTier::T1Source),
            )],
            vec![proposal(false, false, ProofTier::T1Source)],
            vec![],
        ),
    )
    .unwrap();
    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    let decision = Adjudication {
        claim_id: ClaimId::new("claim-1").unwrap(),
        verdict: ScoutVerdict::Supported,
        test: "source inspection".into(),
        reason: "the source contains the control".into(),
        proof_tier: Some(ProofTier::T1Source),
        addressed_counterevidence: BTreeSet::new(),
        instrument_needed: None,
    };
    assert!(ledger
        .adjudicate(decision.clone())
        .unwrap_err()
        .contains("does not establish"));
    check_exact(&mut ledger, "source-1", "root-check");
    ledger.adjudicate(decision).unwrap();
}

#[test]
fn worker_evidence_cannot_reject_a_claim_until_host_checked() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    issue_submit(
        &mut ledger,
        envelope(
            "mapper",
            ScoutRole::Mapper,
            vec![artifact(
                "source-1",
                EvidenceKind::SourceTrace,
                Some(ProofTier::T1Source),
            )],
            vec![proposal(false, false, ProofTier::T1Source)],
            vec![],
        ),
    )
    .unwrap();
    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    let decision = Adjudication {
        claim_id: ClaimId::new("claim-1").unwrap(),
        verdict: ScoutVerdict::Unsupported,
        test: "source inspection failed the expected control".into(),
        reason: "the expected control was absent".into(),
        proof_tier: None,
        addressed_counterevidence: BTreeSet::new(),
        instrument_needed: None,
    };
    assert!(ledger
        .adjudicate(decision.clone())
        .unwrap_err()
        .contains("host-verified"));
    check_exact(&mut ledger, "source-1", "root-failed-test-check");
    ledger.adjudicate(decision).unwrap();
}

#[test]
fn artifactless_worker_claim_is_explicitly_unfalsifiable() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    let mut claim = proposal(false, false, ProofTier::T1Source);
    claim.evidence.clear();
    claim.missing_instrument = Some("a repository snapshot".into());
    issue_submit(
        &mut ledger,
        envelope("mapper", ScoutRole::Mapper, vec![], vec![claim], vec![]),
    )
    .unwrap();
    assert_eq!(
        ledger
            .snapshot()
            .claims
            .get(&ClaimId::new("claim-1").unwrap())
            .unwrap()
            .status,
        ClaimStatus::Unfalsifiable
    );
}

#[test]
fn measurements_require_versioned_methods_and_consistent_missing_counts() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    let mut evidence = measurement_artifact("measurement-1", Some(0.9));
    let measurement = evidence.measurement.as_mut().unwrap();
    measurement.method_version.clear();
    measurement.missing = 101;
    let worker = envelope("mapper", ScoutRole::Mapper, vec![evidence], vec![], vec![]);
    ledger.issue_assignment(assignment_for(&worker)).unwrap();
    assert!(ledger.submit(worker).unwrap_err().contains("missing count"));
}

#[test]
fn evidence_requires_a_replay_recipe_except_for_explicit_assumptions() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    let mut source = artifact(
        "source-1",
        EvidenceKind::SourceTrace,
        Some(ProofTier::T1Source),
    );
    source.recipe = None;
    assert!(ledger
        .record_evidence(source, RunnerId::new("root-source").unwrap())
        .unwrap_err()
        .contains("replay recipe"));

    let mut assumption = artifact("assumption-1", EvidenceKind::Assumption, None);
    assumption.recipe = None;
    ledger
        .record_evidence(assumption, RunnerId::new("root-assumption").unwrap())
        .unwrap();
}

#[test]
fn t3_offline_poc_requires_passing_positive_and_negative_controls() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    let evidence = artifact(
        "poc-1",
        EvidenceKind::OfflinePoc,
        Some(ProofTier::T3OfflinePoc),
    );
    let mut worker = envelope("mapper", ScoutRole::Mapper, vec![evidence], vec![], vec![]);
    ledger.issue_assignment(assignment_for(&worker)).unwrap();
    assert!(ledger
        .submit(worker.clone())
        .unwrap_err()
        .contains("positive and negative controls"));
    worker.artifacts[0].offline_poc_controls = Some(OfflinePocControls {
        positive_control_sha256: "a".repeat(64),
        negative_control_sha256: "b".repeat(64),
        positive_passed: true,
        negative_passed: true,
    });
    ledger.submit(worker).unwrap();
}

#[test]
fn underpowered_quantitative_null_is_not_an_unsupported_verdict() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    let mut claim = proposal(false, true, ProofTier::T1Source);
    claim.evidence = evidence_ids(&["measurement-1"]);
    issue_submit(
        &mut ledger,
        envelope(
            "mapper",
            ScoutRole::Mapper,
            vec![measurement_artifact("measurement-1", Some(0.2))],
            vec![claim],
            vec![],
        ),
    )
    .unwrap();
    check_exact(&mut ledger, "measurement-1", "root-check");
    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    let error = ledger
        .adjudicate(Adjudication {
            claim_id: ClaimId::new("claim-1").unwrap(),
            verdict: ScoutVerdict::Unsupported,
            test: "low-power comparison".into(),
            reason: "the observed effect was small".into(),
            proof_tier: None,
            addressed_counterevidence: BTreeSet::new(),
            instrument_needed: None,
        })
        .unwrap_err();
    assert!(error.contains("underpowered"));
}

#[test]
fn max_parallel_agents_is_not_a_lifetime_submission_limit() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    for index in 0..4 {
        let worker = envelope(
            &format!("mapper-{index}"),
            ScoutRole::Mapper,
            vec![],
            vec![],
            vec![],
        );
        ledger.issue_assignment(assignment_for(&worker)).unwrap();
    }
    let fifth = envelope("mapper-4", ScoutRole::Mapper, vec![], vec![], vec![]);
    assert!(ledger
        .issue_assignment(assignment_for(&fifth))
        .unwrap_err()
        .contains("parallel-agent"));
    ledger
        .submit(envelope(
            "mapper-0",
            ScoutRole::Mapper,
            vec![],
            vec![],
            vec![],
        ))
        .unwrap();
    ledger.issue_assignment(assignment_for(&fifth)).unwrap();
    ledger.submit(fifth).unwrap();
}

#[test]
fn partial_seals_require_an_explicit_limitation_or_followup() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    let mut claim = proposal(false, false, ProofTier::T1Source);
    claim.evidence.clear();
    claim.missing_instrument = Some("a production trace".into());
    issue_submit(
        &mut ledger,
        envelope("mapper", ScoutRole::Mapper, vec![], vec![claim], vec![]),
    )
    .unwrap();
    advance_to(&mut ledger, ScoutPhase::Synthesize);
    assert!(ledger
        .seal(SealDisposition::Partial)
        .unwrap_err()
        .contains("limitation"));
}

#[test]
fn correction_reasons_are_append_only_and_preserved_by_replay() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    issue_submit(
        &mut ledger,
        envelope(
            "mapper",
            ScoutRole::Mapper,
            vec![artifact(
                "source-1",
                EvidenceKind::SourceTrace,
                Some(ProofTier::T1Source),
            )],
            vec![proposal(false, false, ProofTier::T1Source)],
            vec![],
        ),
    )
    .unwrap();
    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    let mut corrected = proposal(false, false, ProofTier::T1Source);
    corrected.id = ClaimId::new("claim-2").unwrap();
    ledger
        .supersede(
            ClaimId::new("claim-1").unwrap(),
            corrected,
            WorkerAssignmentId::new("root-correction").unwrap(),
            "the original pooled two naming eras".into(),
        )
        .unwrap();
    let replayed = ScoutLedger::replay(ledger.events().to_vec()).unwrap();
    let original = replayed
        .snapshot()
        .claims
        .get(&ClaimId::new("claim-1").unwrap())
        .unwrap();
    assert_eq!(
        original.supersession_reason.as_deref(),
        Some("the original pooled two naming eras")
    );
}

#[test]
fn phase_snapshot_scope_and_hash_boundaries_fail_closed() {
    let mut ledger = ScoutLedger::new(charter()).unwrap();
    ledger.advance(ScoutPhase::Map).unwrap();
    let wrong_role = envelope("measurer", ScoutRole::Measurer, vec![], vec![], vec![]);
    assert!(ledger
        .issue_assignment(assignment_for(&wrong_role))
        .unwrap_err()
        .contains("cannot be assigned"));

    let mut wrong_snapshot = envelope("mapper-stale", ScoutRole::Mapper, vec![], vec![], vec![]);
    wrong_snapshot.snapshot_id = "stale".into();
    assert!(ledger
        .issue_assignment(assignment_for(&wrong_snapshot))
        .unwrap_err()
        .contains("different snapshot"));

    let mut wrong_scope = artifact(
        "source-1",
        EvidenceKind::SourceTrace,
        Some(ProofTier::T1Source),
    );
    wrong_scope.scope = "undeclared".into();
    let worker = envelope(
        "mapper-scope",
        ScoutRole::Mapper,
        vec![wrong_scope],
        vec![],
        vec![],
    );
    ledger.issue_assignment(assignment_for(&worker)).unwrap();
    assert!(ledger
        .submit(worker)
        .unwrap_err()
        .contains("undeclared scope"));
}
