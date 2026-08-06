use std::collections::BTreeSet;

use super::super::*;

pub(super) fn evidence_ids(values: &[&str]) -> BTreeSet<EvidenceId> {
    values
        .iter()
        .map(|value| EvidenceId::new(*value).unwrap())
        .collect()
}

pub(super) fn charter() -> ScoutCharter {
    ScoutCharter {
        run_id: ScoutRunId::new("scout-test").unwrap(),
        objective: "Map the fixture with falsifiable claims".into(),
        snapshot_id: "snapshot-1".into(),
        capability_census_id: "census-test".into(),
        capability_fingerprint: "c".repeat(64),
        scopes: BTreeSet::from(["repo".into()]),
        exclusions: BTreeSet::from(["production-write".into()]),
        capabilities: ScoutCapabilities {
            production_read_only: true,
            network_allowed: false,
            denied: BTreeSet::from(["credentials".into()]),
        },
        limits: ScoutLimits::default(),
        minimum_power: 0.8,
    }
}

pub(super) fn artifact(id: &str, kind: EvidenceKind, tier: Option<ProofTier>) -> EvidenceArtifact {
    EvidenceArtifact {
        id: EvidenceId::new(id).unwrap(),
        kind,
        source: format!("fixture/{id}.json"),
        content_sha256: format!("{:0>64}", id.len()),
        observed_at_ms: 1,
        snapshot_id: "snapshot-1".into(),
        scope: "repo".into(),
        recipe: Some(format!("reproduce {id}")),
        proof_tier: tier,
        measurement: None,
        offline_poc_controls: None,
        reproduces: None,
    }
}

pub(super) fn measurement_artifact(id: &str, power: Option<f64>) -> EvidenceArtifact {
    let mut artifact = artifact(id, EvidenceKind::Measurement, Some(ProofTier::T1Source));
    artifact.measurement = Some(Measurement {
        sample_size: 100,
        missing: 0,
        estimate: 0.6,
        interval: ConfidenceInterval {
            lower: 0.502,
            upper: 0.691,
            confidence: 0.95,
        },
        method: "wilson".into(),
        method_version: "scout-wilson-v1".into(),
        seed: None,
        power,
    });
    artifact
}

pub(super) fn envelope(
    assignment: &str,
    role: ScoutRole,
    artifacts: Vec<EvidenceArtifact>,
    claims: Vec<ClaimProposal>,
    claim_updates: Vec<ClaimUpdate>,
) -> WorkerEnvelope {
    WorkerEnvelope {
        assignment_id: WorkerAssignmentId::new(assignment).unwrap(),
        role,
        snapshot_id: "snapshot-1".into(),
        artifacts,
        claims,
        claim_updates,
        coverage: format!("{assignment} covered its declared surface"),
        limitations: Vec::new(),
        requested_followups: Vec::new(),
    }
}

pub(super) fn assignment_for(envelope: &WorkerEnvelope) -> ScoutAssignment {
    ScoutAssignment {
        id: envelope.assignment_id.clone(),
        role: envelope.role,
        objective: format!("Run {} evidence pass", envelope.assignment_id),
        snapshot_id: envelope.snapshot_id.clone(),
        scopes: BTreeSet::from(["repo".into()]),
    }
}

pub(super) fn issue_submit(
    ledger: &mut ScoutLedger,
    envelope: WorkerEnvelope,
) -> Result<(), String> {
    ledger.issue_assignment(assignment_for(&envelope))?;
    ledger.submit(envelope)
}

pub(super) fn proposal(
    headline: bool,
    quantitative: bool,
    required_tier: ProofTier,
) -> ClaimProposal {
    ClaimProposal {
        id: ClaimId::new("claim-1").unwrap(),
        text: "The measured control is present".into(),
        headline,
        quantitative,
        required_tier: Some(required_tier),
        evidence: evidence_ids(&["source-1"]),
        counterevidence: BTreeSet::new(),
        assumptions: vec!["the pinned snapshot is representative".into()],
        missing_instrument: None,
    }
}

pub(super) fn check_exact(ledger: &mut ScoutLedger, evidence_id: &str, verifier: &str) {
    let id = EvidenceId::new(evidence_id).unwrap();
    let digest = ledger
        .snapshot()
        .evidence
        .get(&id)
        .unwrap()
        .artifact
        .content_sha256
        .clone();
    ledger
        .check_evidence(EvidenceCheck {
            evidence_id: id,
            verifier: RunnerId::new(verifier).unwrap(),
            outcome: VerificationOutcome::Exact,
            observed_sha256: Some(digest),
            checked_at_ms: 2,
            recipe: "host re-read the pinned fixture".into(),
            reason: "fresh hash matched".into(),
        })
        .unwrap();
}

pub(super) fn advance_to(ledger: &mut ScoutLedger, target: ScoutPhase) {
    while ledger.snapshot().phase != target {
        let next = ledger.snapshot().phase.next().unwrap();
        assert_ne!(next, ScoutPhase::Sealed);
        ledger.advance(next).unwrap();
    }
}
