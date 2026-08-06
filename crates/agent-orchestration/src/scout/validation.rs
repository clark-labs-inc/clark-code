use std::collections::BTreeSet;

mod artifact;

use self::artifact::validate_artifact;
use super::{
    Adjudication, AssignmentStatus, ClaimProposal, ClaimRecord, ClaimStatus, EvidenceArtifact,
    EvidenceCheck, EvidenceId, EvidenceKind, EvidenceProducer, EvidenceRecord, ProofTier,
    ScoutCharter, ScoutPhase, ScoutSnapshot, ScoutVerdict, VerificationOutcome, WorkerEnvelope,
};

pub(super) fn validate_charter(charter: &ScoutCharter) -> Result<(), String> {
    nonempty("objective", &charter.objective)?;
    nonempty("snapshot_id", &charter.snapshot_id)?;
    nonempty("capability_census_id", &charter.capability_census_id)?;
    validate_digest(
        "capability census fingerprint",
        &charter.capability_fingerprint,
    )?;
    if charter.scopes.is_empty() {
        return Err("Scout charter requires at least one scope".to_string());
    }
    for scope in &charter.scopes {
        nonempty("scope", scope)?;
    }
    for exclusion in &charter.exclusions {
        nonempty("exclusion", exclusion)?;
    }
    if charter.limits.max_parallel_agents == 0 || charter.limits.max_parallel_agents > 32 {
        return Err("Scout max_parallel_agents must be between 1 and 32".to_string());
    }
    if charter.limits.max_worker_submissions == 0
        || charter.limits.max_claims == 0
        || charter.limits.max_artifacts == 0
        || charter.limits.max_events < 8
    {
        return Err(
            "Scout submission/claim/artifact limits must be positive and max_events at least 8"
                .into(),
        );
    }
    if !charter.minimum_power.is_finite()
        || charter.minimum_power <= 0.0
        || charter.minimum_power > 1.0
    {
        return Err("Scout minimum_power must be finite and in (0, 1]".to_string());
    }
    Ok(())
}

pub(super) fn validate_envelope(
    snapshot: &ScoutSnapshot,
    envelope: &WorkerEnvelope,
) -> Result<(), String> {
    let assignment = snapshot
        .assignments
        .get(&envelope.assignment_id)
        .ok_or_else(|| format!("unknown or unissued assignment {}", envelope.assignment_id))?;
    if assignment.status != AssignmentStatus::Issued {
        return Err(format!(
            "assignment {} already submitted",
            envelope.assignment_id
        ));
    }
    if assignment.assignment.role != envelope.role {
        return Err("worker role does not match its host-issued assignment".to_string());
    }
    if assignment.assignment.snapshot_id != envelope.snapshot_id {
        return Err("worker snapshot does not match its host-issued assignment".to_string());
    }
    if !envelope.role.allowed_in(snapshot.phase) {
        return Err(format!(
            "role {:?} cannot submit during {:?}",
            envelope.role, snapshot.phase
        ));
    }
    if envelope.snapshot_id != snapshot.charter.snapshot_id {
        return Err("worker evidence was collected against a different snapshot".to_string());
    }
    nonempty("coverage", &envelope.coverage)?;
    if snapshot.evidence.len() + envelope.artifacts.len() > snapshot.charter.limits.max_artifacts {
        return Err("Scout artifact limit reached".to_string());
    }
    if snapshot.claims.len() + envelope.claims.len() > snapshot.charter.limits.max_claims {
        return Err("Scout claim limit reached".to_string());
    }

    let mut evidence_ids = snapshot.evidence.keys().cloned().collect::<BTreeSet<_>>();
    for artifact in &envelope.artifacts {
        if !evidence_ids.insert(artifact.id.clone()) {
            return Err(format!("duplicate evidence id {}", artifact.id));
        }
        validate_artifact(snapshot, artifact)?;
        if !assignment.assignment.scopes.contains(&artifact.scope) {
            return Err(format!(
                "evidence {} is outside assignment {} scopes",
                artifact.id, envelope.assignment_id
            ));
        }
    }
    for artifact in &envelope.artifacts {
        if let Some(reproduced) = &artifact.reproduces {
            if reproduced == &artifact.id || !evidence_ids.contains(reproduced) {
                return Err(format!(
                    "reproduction evidence {} refers to unknown evidence {}",
                    artifact.id, reproduced
                ));
            }
        }
    }

    let mut claim_ids = snapshot.claims.keys().cloned().collect::<BTreeSet<_>>();
    for claim in &envelope.claims {
        if !claim_ids.insert(claim.id.clone()) {
            return Err(format!("duplicate claim id {}", claim.id));
        }
        validate_claim(claim, &evidence_ids)?;
    }
    let mut updated = BTreeSet::new();
    for update in &envelope.claim_updates {
        if !snapshot.claims.contains_key(&update.claim_id) {
            return Err(format!("cannot update unknown claim {}", update.claim_id));
        }
        if !updated.insert(update.claim_id.clone()) {
            return Err(format!("duplicate update for claim {}", update.claim_id));
        }
        require_evidence_exists(&update.evidence, &evidence_ids)?;
        require_evidence_exists(&update.counterevidence, &evidence_ids)?;
        for limitation in &update.limitations {
            nonempty("claim limitation", limitation)?;
        }
    }
    for limitation in &envelope.limitations {
        nonempty("worker limitation", limitation)?;
    }
    for followup in &envelope.requested_followups {
        nonempty("requested followup", followup)?;
    }
    Ok(())
}

pub(super) fn validate_artifact_for_ledger(
    snapshot: &ScoutSnapshot,
    artifact: &EvidenceArtifact,
) -> Result<(), String> {
    validate_artifact(snapshot, artifact)
}

pub(super) fn validate_evidence_check(
    snapshot: &ScoutSnapshot,
    check: &EvidenceCheck,
) -> Result<(), String> {
    let record = snapshot
        .evidence
        .get(&check.evidence_id)
        .ok_or_else(|| format!("cannot verify unknown evidence {}", check.evidence_id))?;
    nonempty("verification recipe", &check.recipe)?;
    nonempty("verification reason", &check.reason)?;
    if check.checked_at_ms == 0 {
        return Err("evidence checks require an observation timestamp".to_string());
    }
    if let Some(digest) = &check.observed_sha256 {
        validate_digest("observed evidence", digest)?;
    }
    match check.outcome {
        VerificationOutcome::Exact => {
            let observed = check
                .observed_sha256
                .as_deref()
                .ok_or_else(|| "exact verification requires an observed hash".to_string())?;
            if observed != record.artifact.content_sha256 {
                return Err("exact verification hash does not match the recorded artifact".into());
            }
        }
        VerificationOutcome::Changed => {
            let observed = check
                .observed_sha256
                .as_deref()
                .ok_or_else(|| "changed verification requires an observed hash".to_string())?;
            if observed == record.artifact.content_sha256 {
                return Err("changed verification must report a different hash".to_string());
            }
        }
        VerificationOutcome::Equivalent => {
            if check.observed_sha256.is_none() {
                return Err("equivalent verification requires an observed hash".to_string());
            }
        }
        VerificationOutcome::Unavailable | VerificationOutcome::Failed => {
            if check.observed_sha256.is_some() {
                return Err(
                    "unavailable or failed verification must not claim an observed hash".into(),
                );
            }
        }
    }
    Ok(())
}

fn validate_claim(
    claim: &ClaimProposal,
    evidence_ids: &BTreeSet<EvidenceId>,
) -> Result<(), String> {
    nonempty("claim text", &claim.text)?;
    require_evidence_exists(&claim.evidence, evidence_ids)?;
    require_evidence_exists(&claim.counterevidence, evidence_ids)?;
    for assumption in &claim.assumptions {
        nonempty("claim assumption", assumption)?;
    }
    if let Some(instrument) = &claim.missing_instrument {
        nonempty("missing instrument", instrument)?;
    }
    Ok(())
}

pub(super) fn validate_claim_for_ledger(
    claim: &ClaimProposal,
    evidence_ids: &BTreeSet<EvidenceId>,
) -> Result<(), String> {
    validate_claim(claim, evidence_ids)
}

pub(super) fn validate_adjudication(
    snapshot: &ScoutSnapshot,
    decision: &Adjudication,
) -> Result<(), String> {
    if snapshot.phase != ScoutPhase::Adjudicate {
        return Err("claims may only be adjudicated during the adjudicate phase".to_string());
    }
    let claim = snapshot
        .claims
        .get(&decision.claim_id)
        .ok_or_else(|| format!("unknown claim {}", decision.claim_id))?;
    if claim.status.is_adjudicated()
        || matches!(
            claim.status,
            ClaimStatus::Retracted | ClaimStatus::Superseded
        )
    {
        return Err(format!("claim {} is already final", decision.claim_id));
    }
    nonempty("adjudication test", &decision.test)?;
    nonempty("adjudication reason", &decision.reason)?;
    if decision.addressed_counterevidence != claim.proposal.counterevidence {
        return Err("adjudication must explicitly address every counterevidence artifact".into());
    }
    match decision.verdict {
        ScoutVerdict::Supported => validate_supported(snapshot, claim, decision),
        ScoutVerdict::Unsupported => validate_unsupported(snapshot, claim, decision),
        ScoutVerdict::Unfalsifiable => validate_unfalsifiable(claim, decision),
    }
}

fn validate_supported(
    snapshot: &ScoutSnapshot,
    claim: &ClaimRecord,
    decision: &Adjudication,
) -> Result<(), String> {
    if claim.proposal.evidence.is_empty() {
        return Err("supported claims require evidence".to_string());
    }
    let proof_tier = decision
        .proof_tier
        .ok_or_else(|| "supported claims require a proof tier".to_string())?;
    let maximum = maximum_proof_tier(snapshot, &claim.proposal.evidence)
        .ok_or_else(|| "claim evidence does not establish a proof tier".to_string())?;
    if proof_tier > maximum {
        return Err(format!(
            "adjudication claims {:?} but evidence reaches only {:?}",
            proof_tier, maximum
        ));
    }
    if claim
        .proposal
        .required_tier
        .is_some_and(|required| proof_tier < required)
    {
        return Err("claim wording requires a higher proof tier than the evidence reached".into());
    }
    if claim.proposal.quantitative && !has_valid_measurement(snapshot, claim) {
        return Err(
            "supported quantitative claims require n, method, and an uncertainty interval".into(),
        );
    }
    Ok(())
}

fn validate_unsupported(
    snapshot: &ScoutSnapshot,
    claim: &ClaimRecord,
    decision: &Adjudication,
) -> Result<(), String> {
    if decision.proof_tier.is_some() {
        return Err("unsupported claims do not receive a proof tier".to_string());
    }
    if claim.proposal.evidence.is_empty() && claim.proposal.counterevidence.is_empty() {
        return Err("unsupported claims require a named failed test artifact".to_string());
    }
    let has_verified_test = claim
        .proposal
        .evidence
        .iter()
        .chain(&claim.proposal.counterevidence)
        .filter_map(|id| snapshot.evidence.get(id))
        .any(evidence_verified);
    if !has_verified_test {
        return Err("unsupported claims require a host-verified failed test artifact".to_string());
    }
    if claim.proposal.quantitative {
        let sufficiently_powered = claim
            .proposal
            .evidence
            .iter()
            .chain(&claim.proposal.counterevidence)
            .filter_map(|id| snapshot.evidence.get(id))
            .filter(|record| evidence_verified(record))
            .filter_map(|record| record.artifact.measurement.as_ref())
            .any(|measurement| {
                measurement
                    .power
                    .is_some_and(|power| power >= snapshot.charter.minimum_power)
            });
        if !sufficiently_powered {
            return Err(
                "an underpowered quantitative test cannot support an unsupported verdict".into(),
            );
        }
    }
    Ok(())
}

fn validate_unfalsifiable(claim: &ClaimRecord, decision: &Adjudication) -> Result<(), String> {
    if decision.proof_tier.is_some() {
        return Err("unfalsifiable claims do not receive a proof tier".to_string());
    }
    let instrument = decision
        .instrument_needed
        .as_deref()
        .or(claim.proposal.missing_instrument.as_deref())
        .ok_or_else(|| "unfalsifiable verdicts must name the missing instrument".to_string())?;
    nonempty("missing instrument", instrument)
}

fn has_valid_measurement(snapshot: &ScoutSnapshot, claim: &ClaimRecord) -> bool {
    claim
        .proposal
        .evidence
        .iter()
        .filter_map(|id| snapshot.evidence.get(id))
        .any(|record| evidence_verified(record) && record.artifact.measurement.is_some())
}

fn maximum_proof_tier(
    snapshot: &ScoutSnapshot,
    evidence: &BTreeSet<EvidenceId>,
) -> Option<ProofTier> {
    evidence
        .iter()
        .filter_map(|id| snapshot.evidence.get(id))
        .filter(|record| evidence_verified(record))
        .filter_map(|record| resolved_tier(snapshot, record.artifact.proof_tier, &record.artifact))
        .max()
}

pub(super) fn evidence_verified(record: &EvidenceRecord) -> bool {
    match record.checks.last() {
        Some(check) => matches!(
            check.outcome,
            VerificationOutcome::Exact | VerificationOutcome::Equivalent
        ),
        None => matches!(record.producer, EvidenceProducer::Runner { .. }),
    }
}

fn resolved_tier(
    snapshot: &ScoutSnapshot,
    declared: Option<ProofTier>,
    artifact: &EvidenceArtifact,
) -> Option<ProofTier> {
    if artifact.kind != EvidenceKind::Reproduction {
        return declared;
    }
    let reproduced = artifact.reproduces.as_ref()?;
    snapshot
        .evidence
        .get(reproduced)
        .and_then(|record| record.artifact.proof_tier)
}

fn require_evidence_exists(
    requested: &BTreeSet<EvidenceId>,
    available: &BTreeSet<EvidenceId>,
) -> Result<(), String> {
    if let Some(missing) = requested.iter().find(|id| !available.contains(*id)) {
        return Err(format!("unknown evidence id {missing}"));
    }
    Ok(())
}

fn nonempty(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_digest(label: &str, digest: &str) -> Result<(), String> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Err(format!("{label} requires a 64-character SHA-256 digest"))
    } else {
        Ok(())
    }
}
