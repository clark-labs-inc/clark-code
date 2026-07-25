use std::collections::BTreeSet;

use agent_orchestration::{
    compute_scout_measurement, ScoutActor, ScoutAdjudication, ScoutAssignment, ScoutCapabilities,
    ScoutCharter, ScoutClaimId, ScoutClaimProposal, ScoutClaimUpdate, ScoutConfidenceInterval,
    ScoutEventKind, ScoutEvidenceArtifact, ScoutEvidenceCheck, ScoutEvidenceId, ScoutEvidenceKind,
    ScoutLedger, ScoutLimits, ScoutMeasurement, ScoutMeasurementMethod, ScoutPhase, ScoutRole,
    ScoutRunId, ScoutRunnerId, ScoutVerdict, ScoutWorkerAssignmentId, ScoutWorkerEnvelope,
    SealDisposition, VerificationOutcome,
};
use serde_json::{json, Value};

fn charter() -> ScoutCharter {
    ScoutCharter {
        run_id: ScoutRunId::new("benchmark-run").unwrap(),
        objective: "Evaluate the Scout evidence contract".into(),
        snapshot_id: "fixture-v1".into(),
        capability_census_id: "benchmark-census".into(),
        capability_fingerprint: "c".repeat(64),
        scopes: BTreeSet::from(["repo".into()]),
        exclusions: BTreeSet::from(["secret-values".into(), "production-write".into()]),
        capabilities: ScoutCapabilities {
            production_read_only: true,
            network_allowed: false,
            denied: BTreeSet::from(["secret-values".into()]),
        },
        limits: ScoutLimits::default(),
        minimum_power: 0.8,
    }
}

fn evidence(
    id: &str,
    kind: ScoutEvidenceKind,
    tier: Option<agent_orchestration::ProofTier>,
) -> ScoutEvidenceArtifact {
    ScoutEvidenceArtifact {
        id: ScoutEvidenceId::new(id).unwrap(),
        kind,
        source: format!("fixture/{id}.json"),
        content_sha256: format!("{:0>64}", id.len()),
        observed_at_ms: 1,
        snapshot_id: "fixture-v1".into(),
        scope: "repo".into(),
        recipe: Some(format!(r#"{{"version":"fixture-v1","id":"{id}"}}"#)),
        proof_tier: tier,
        measurement: None,
        offline_poc_controls: None,
        reproduces: None,
    }
}

fn claim(evidence_ids: &[&str], quantitative: bool, headline: bool) -> ScoutClaimProposal {
    ScoutClaimProposal {
        id: ScoutClaimId::new("claim-1").unwrap(),
        text: "The fixture contains the expected control".into(),
        headline,
        quantitative,
        required_tier: Some(agent_orchestration::ProofTier::T1Source),
        evidence: evidence_ids
            .iter()
            .map(|id| ScoutEvidenceId::new(*id).unwrap())
            .collect(),
        counterevidence: BTreeSet::new(),
        assumptions: vec!["the fixture snapshot is pinned".into()],
        missing_instrument: None,
    }
}

fn envelope(
    id: &str,
    artifacts: Vec<ScoutEvidenceArtifact>,
    claims: Vec<ScoutClaimProposal>,
) -> ScoutWorkerEnvelope {
    ScoutWorkerEnvelope {
        assignment_id: ScoutWorkerAssignmentId::new(id).unwrap(),
        role: ScoutRole::Mapper,
        snapshot_id: "fixture-v1".into(),
        artifacts,
        claims,
        claim_updates: Vec::new(),
        coverage: "bounded fixture".into(),
        limitations: Vec::new(),
        requested_followups: Vec::new(),
    }
}

fn assignment(id: &str) -> ScoutAssignment {
    ScoutAssignment {
        id: ScoutWorkerAssignmentId::new(id).unwrap(),
        role: ScoutRole::Mapper,
        objective: "Map the bounded fixture".into(),
        snapshot_id: "fixture-v1".into(),
        scopes: BTreeSet::from(["repo".into()]),
    }
}

fn advance_to(ledger: &mut ScoutLedger, target: ScoutPhase) {
    while ledger.snapshot().phase != target {
        ledger
            .advance(ledger.snapshot().phase.next().unwrap())
            .unwrap();
    }
}

pub fn complete_replay() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    let source = evidence(
        "source-1",
        ScoutEvidenceKind::SourceTrace,
        Some(agent_orchestration::ProofTier::T1Source),
    );
    let source_hash = source.content_sha256.clone();
    ledger.record_evidence(source, ScoutRunnerId::new("root-source")?)?;
    ledger.issue_assignment(assignment("mapper"))?;
    ledger.submit(envelope(
        "mapper",
        Vec::new(),
        vec![claim(&["source-1"], false, true)],
    ))?;
    ledger.advance(ScoutPhase::Measure)?;
    ledger.advance(ScoutPhase::Check)?;
    let mut reproduction = evidence("reproduction-1", ScoutEvidenceKind::Reproduction, None);
    reproduction.content_sha256 = source_hash.clone();
    reproduction.reproduces = Some(ScoutEvidenceId::new("source-1")?);
    ledger.record_evidence(reproduction, ScoutRunnerId::new("independent-reproducer")?)?;
    ledger.check_evidence(ScoutEvidenceCheck {
        evidence_id: ScoutEvidenceId::new("reproduction-1")?,
        verifier: ScoutRunnerId::new("independent-verifier")?,
        outcome: VerificationOutcome::Exact,
        observed_sha256: Some(source_hash),
        checked_at_ms: 2,
        recipe: "fixture replay".into(),
        reason: "fresh replay matched".into(),
    })?;
    ledger.issue_assignment(ScoutAssignment {
        id: ScoutWorkerAssignmentId::new("reproducer")?,
        role: ScoutRole::Reproducer,
        objective: "Attach the independently replayed receipt".into(),
        snapshot_id: "fixture-v1".into(),
        scopes: BTreeSet::from(["repo".into()]),
    })?;
    ledger.submit(ScoutWorkerEnvelope {
        assignment_id: ScoutWorkerAssignmentId::new("reproducer")?,
        role: ScoutRole::Reproducer,
        snapshot_id: "fixture-v1".into(),
        artifacts: Vec::new(),
        claims: Vec::new(),
        claim_updates: vec![ScoutClaimUpdate {
            claim_id: ScoutClaimId::new("claim-1")?,
            evidence: BTreeSet::from([ScoutEvidenceId::new("reproduction-1")?]),
            counterevidence: BTreeSet::new(),
            limitations: Vec::new(),
        }],
        coverage: "independent reproduction".into(),
        limitations: Vec::new(),
        requested_followups: Vec::new(),
    })?;
    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    ledger.adjudicate(ScoutAdjudication {
        claim_id: ScoutClaimId::new("claim-1")?,
        verdict: ScoutVerdict::Supported,
        test: "host source read plus independent replay".into(),
        reason: "receipts matched".into(),
        proof_tier: Some(agent_orchestration::ProofTier::T1Source),
        addressed_counterevidence: BTreeSet::new(),
        instrument_needed: None,
    })?;
    ledger.advance(ScoutPhase::Synthesize)?;
    ledger.seal(SealDisposition::Complete)?;
    let fingerprint = ledger.fingerprint()?;
    let replayed = ScoutLedger::replay(ledger.events().to_vec())?;
    if replayed.fingerprint()? != fingerprint || replayed.snapshot() != ledger.snapshot() {
        return Err("ledger replay drifted".into());
    }
    Ok((
        "complete ledger replays byte-for-byte".into(),
        json!({"fingerprint": fingerprint, "events": ledger.events().len()}),
    ))
}

pub fn unissued_assignment_rejected() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    let error = ledger
        .submit(envelope("forged-worker", Vec::new(), Vec::new()))
        .expect_err("unissued worker must fail");
    if !error.contains("unissued") {
        return Err(format!("unexpected rejection: {error}"));
    }
    Ok(("unissued worker rejected".into(), json!({"error": error})))
}

pub fn worker_self_certification_rejected() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    ledger.issue_assignment(assignment("mapper"))?;
    ledger.submit(envelope(
        "mapper",
        vec![evidence(
            "source-1",
            ScoutEvidenceKind::SourceTrace,
            Some(agent_orchestration::ProofTier::T1Source),
        )],
        vec![claim(&["source-1"], false, false)],
    ))?;
    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    let error = ledger
        .adjudicate(ScoutAdjudication {
            claim_id: ScoutClaimId::new("claim-1")?,
            verdict: ScoutVerdict::Supported,
            test: "worker hash".into(),
            reason: "worker asserted it".into(),
            proof_tier: Some(agent_orchestration::ProofTier::T1Source),
            addressed_counterevidence: BTreeSet::new(),
            instrument_needed: None,
        })
        .expect_err("unverified worker evidence must fail");
    if !error.contains("does not establish") {
        return Err(format!("unexpected rejection: {error}"));
    }
    Ok((
        "worker hash cannot certify itself".into(),
        json!({"error": error}),
    ))
}

pub fn missing_replay_recipe_rejected() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    let mut source = evidence(
        "source-1",
        ScoutEvidenceKind::SourceTrace,
        Some(agent_orchestration::ProofTier::T1Source),
    );
    source.recipe = None;
    let error = ledger
        .record_evidence(source, ScoutRunnerId::new("root-source")?)
        .expect_err("evidence without a replay recipe must fail");
    if !error.contains("replay recipe") {
        return Err(format!("unexpected rejection: {error}"));
    }
    Ok((
        "non-assumption evidence without a replay recipe is rejected".into(),
        json!({"error": error}),
    ))
}

pub fn unverified_failed_test_rejected() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    ledger.issue_assignment(assignment("mapper"))?;
    ledger.submit(envelope(
        "mapper",
        vec![evidence(
            "source-1",
            ScoutEvidenceKind::SourceTrace,
            Some(agent_orchestration::ProofTier::T1Source),
        )],
        vec![claim(&["source-1"], false, false)],
    ))?;
    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    let error = ledger
        .adjudicate(ScoutAdjudication {
            claim_id: ScoutClaimId::new("claim-1")?,
            verdict: ScoutVerdict::Unsupported,
            test: "worker-reported failed source test".into(),
            reason: "the expected control was absent".into(),
            proof_tier: None,
            addressed_counterevidence: BTreeSet::new(),
            instrument_needed: None,
        })
        .expect_err("unverified worker failure must not reject a claim");
    if !error.contains("host-verified") {
        return Err(format!("unexpected rejection: {error}"));
    }
    Ok((
        "worker-reported failure cannot reject a claim without host verification".into(),
        json!({"error": error}),
    ))
}

pub fn t3_controls_required() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    ledger.issue_assignment(assignment("mapper"))?;
    let artifact = evidence(
        "poc-1",
        ScoutEvidenceKind::OfflinePoc,
        Some(agent_orchestration::ProofTier::T3OfflinePoc),
    );
    let error = ledger
        .submit(envelope("mapper", vec![artifact], Vec::new()))
        .expect_err("T3 without controls must fail");
    if !error.contains("positive and negative controls") {
        return Err(format!("unexpected rejection: {error}"));
    }
    Ok((
        "T3 requires typed positive and negative controls".into(),
        json!({"error": error}),
    ))
}

pub fn underpowered_null_rejected() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    let mut measurement = evidence(
        "measurement-1",
        ScoutEvidenceKind::Measurement,
        Some(agent_orchestration::ProofTier::T1Source),
    );
    measurement.measurement = Some(ScoutMeasurement {
        sample_size: 10,
        missing: 0,
        estimate: 0.5,
        interval: ScoutConfidenceInterval {
            lower: 0.236_593,
            upper: 0.763_407,
            confidence: 0.95,
        },
        method: "wilson_score".into(),
        method_version: "scout-wilson-v1".into(),
        seed: None,
        power: Some(0.2),
    });
    ledger.record_evidence(measurement, ScoutRunnerId::new("measurement-runner")?)?;
    ledger.issue_assignment(assignment("mapper"))?;
    ledger.submit(envelope(
        "mapper",
        Vec::new(),
        vec![claim(&["measurement-1"], true, false)],
    ))?;
    advance_to(&mut ledger, ScoutPhase::Adjudicate);
    let error = ledger
        .adjudicate(ScoutAdjudication {
            claim_id: ScoutClaimId::new("claim-1")?,
            verdict: ScoutVerdict::Unsupported,
            test: "small sample".into(),
            reason: "no observed difference".into(),
            proof_tier: None,
            addressed_counterevidence: BTreeSet::new(),
            instrument_needed: None,
        })
        .expect_err("underpowered null must fail");
    if !error.contains("underpowered") {
        return Err(format!("unexpected rejection: {error}"));
    }
    Ok((
        "underpowered quantitative null rejected".into(),
        json!({"error": error}),
    ))
}

pub fn partial_requires_limit() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    let mut open = claim(&[], false, false);
    open.missing_instrument = Some("production trace".into());
    ledger.issue_assignment(assignment("mapper"))?;
    ledger.submit(envelope("mapper", Vec::new(), vec![open]))?;
    advance_to(&mut ledger, ScoutPhase::Synthesize);
    let error = ledger
        .seal(SealDisposition::Partial)
        .expect_err("partial seal without limits must fail");
    if !error.contains("limitation") {
        return Err(format!("unexpected rejection: {error}"));
    }
    Ok((
        "partial seal requires an explicit gap".into(),
        json!({"error": error}),
    ))
}

pub fn forged_actor_rejected() -> Result<(String, Value), String> {
    let mut ledger = ScoutLedger::new(charter())?;
    ledger.advance(ScoutPhase::Map)?;
    ledger.issue_assignment(assignment("mapper"))?;
    ledger.submit(envelope("mapper", Vec::new(), Vec::new()))?;
    let mut events = ledger.events().to_vec();
    let event = events
        .iter_mut()
        .find(|event| matches!(event.kind, ScoutEventKind::WorkerSubmitted { .. }))
        .ok_or_else(|| "worker event missing".to_string())?;
    event.actor = ScoutActor::Root;
    let error = ScoutLedger::replay(events).expect_err("forged actor must fail");
    if !error.contains("not authorized") {
        return Err(format!("unexpected rejection: {error}"));
    }
    Ok((
        "replay rejects actor forgery".into(),
        json!({"error": error}),
    ))
}

pub fn wilson_reference() -> Result<(String, Value), String> {
    let observations = (0..100).map(|index| json!(index < 60)).collect::<Vec<_>>();
    let result = compute_scout_measurement(
        ScoutMeasurementMethod::WilsonProportion,
        &observations,
        0.95,
        None,
        None,
    )?;
    if (result.estimate - 0.6).abs() > 1e-12
        || (result.lower - 0.502_002_586_791_061_8).abs() > 1e-12
        || (result.upper - 0.690_598_713_567_541_9).abs() > 1e-12
    {
        return Err("Wilson implementation drifted from reference".into());
    }
    Ok((
        "Wilson 95% interval matches reference".into(),
        json!({
            "estimate": result.estimate,
            "lower": result.lower,
            "upper": result.upper,
            "method_version": result.method_version
        }),
    ))
}

pub fn seeded_bootstrap_determinism() -> Result<(String, Value), String> {
    let observations = vec![json!(1.0), json!(2.0), Value::Null, json!(5.0)];
    let first = compute_scout_measurement(
        ScoutMeasurementMethod::BootstrapMedian,
        &observations,
        0.95,
        Some(1_000),
        Some(42),
    )?;
    let replay = compute_scout_measurement(
        ScoutMeasurementMethod::BootstrapMedian,
        &observations,
        0.95,
        Some(1_000),
        Some(42),
    )?;
    if first != replay
        || first.sample_size != 4
        || first.missing != 1
        || first.estimate != 2.0
        || first.lower > first.estimate
        || first.upper < first.estimate
    {
        return Err("seeded bootstrap replay drifted".into());
    }
    Ok((
        "seeded bootstrap replay is deterministic".into(),
        json!({
            "estimate": first.estimate,
            "lower": first.lower,
            "upper": first.upper,
            "missing": first.missing,
            "method_version": first.method_version,
            "seed": first.seed,
            "resamples": first.resamples
        }),
    ))
}
