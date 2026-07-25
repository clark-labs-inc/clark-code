use super::super::validation::{
    evidence_verified, validate_adjudication, validate_artifact_for_ledger, validate_charter,
    validate_envelope, validate_evidence_check,
};
use super::super::{
    AssignmentRecord, AssignmentStatus, ClaimProposal, ClaimRecord, ClaimStatus, EvidenceKind,
    EvidenceProducer, EvidenceRecord, ScoutActor, ScoutAssignment, ScoutEventKind, ScoutPhase,
    ScoutSnapshot, ScoutVerdict, VerificationOutcome, WorkerEnvelope,
};
use super::ScoutLedger;

impl ScoutLedger {
    pub(super) fn validate_kind(&self, kind: &ScoutEventKind) -> Result<(), String> {
        if self.snapshot.phase == ScoutPhase::Sealed {
            return Err("sealed Scout runs are immutable".to_string());
        }
        match kind {
            ScoutEventKind::Chartered { charter } => {
                if !self.events.is_empty() {
                    return Err("Scout can only be chartered once".to_string());
                }
                if charter.run_id != self.snapshot.charter.run_id {
                    return Err("charter run id changed during initialization".to_string());
                }
                validate_charter(charter)
            }
            ScoutEventKind::PhaseAdvanced { from, to } => {
                if *from != self.snapshot.phase {
                    return Err("phase transition starts from stale state".to_string());
                }
                if from.next() != Some(*to) {
                    return Err(format!("invalid Scout phase transition {from:?} -> {to:?}"));
                }
                if *to == ScoutPhase::Sealed {
                    return Err("use seal to enter the sealed phase".to_string());
                }
                Ok(())
            }
            ScoutEventKind::AssignmentIssued { assignment } => {
                if !assignment.role.allowed_in(self.snapshot.phase) {
                    return Err(format!(
                        "role {:?} cannot be assigned during {:?}",
                        assignment.role, self.snapshot.phase
                    ));
                }
                if self.snapshot.assignments.contains_key(&assignment.id) {
                    return Err(format!("duplicate assignment id {}", assignment.id));
                }
                if self.snapshot.assignments.len()
                    >= self.snapshot.charter.limits.max_worker_submissions
                {
                    return Err("Scout worker submission limit reached".to_string());
                }
                let active = self
                    .snapshot
                    .assignments
                    .values()
                    .filter(|record| record.status == AssignmentStatus::Issued)
                    .count();
                if active >= usize::from(self.snapshot.charter.limits.max_parallel_agents) {
                    return Err("Scout parallel-agent limit reached".to_string());
                }
                validate_assignment(&self.snapshot, assignment)
            }
            ScoutEventKind::WorkerSubmitted { envelope } => {
                validate_envelope(&self.snapshot, envelope)
            }
            ScoutEventKind::EvidenceRecorded {
                artifact,
                runner_id: _,
            } => {
                if !matches!(
                    self.snapshot.phase,
                    ScoutPhase::Map | ScoutPhase::Measure | ScoutPhase::Check | ScoutPhase::Prove
                ) {
                    return Err(
                        "runner evidence may only be recorded during evidence phases".to_string(),
                    );
                }
                if self.snapshot.evidence.len() >= self.snapshot.charter.limits.max_artifacts {
                    return Err("Scout artifact limit reached".to_string());
                }
                if self.snapshot.evidence.contains_key(&artifact.id) {
                    return Err(format!("duplicate evidence id {}", artifact.id));
                }
                validate_artifact_for_ledger(&self.snapshot, artifact)?;
                if let Some(reproduced) = &artifact.reproduces {
                    if reproduced == &artifact.id
                        || !self.snapshot.evidence.contains_key(reproduced)
                    {
                        return Err(format!(
                            "reproduction evidence {} refers to unknown evidence {}",
                            artifact.id, reproduced
                        ));
                    }
                }
                Ok(())
            }
            ScoutEventKind::EvidenceChecked { check } => {
                validate_evidence_check(&self.snapshot, check)
            }
            ScoutEventKind::ClaimAdjudicated { decision } => {
                validate_adjudication(&self.snapshot, decision)
            }
            ScoutEventKind::ClaimRetracted { claim_id, reason } => {
                if self.snapshot.phase != ScoutPhase::Adjudicate {
                    return Err("claims may only be retracted during adjudication".to_string());
                }
                if reason.trim().is_empty() {
                    return Err("retractions require a reason".to_string());
                }
                let claim = self
                    .snapshot
                    .claims
                    .get(claim_id)
                    .ok_or_else(|| format!("unknown claim {claim_id}"))?;
                if matches!(
                    claim.status,
                    ClaimStatus::Retracted | ClaimStatus::Superseded
                ) {
                    return Err(format!("claim {claim_id} is already withdrawn"));
                }
                Ok(())
            }
            ScoutEventKind::ClaimSuperseded {
                claim_id,
                replacement,
                assignment_id: _,
                reason,
            } => {
                if self.snapshot.phase != ScoutPhase::Adjudicate {
                    return Err("claims may only be superseded during adjudication".to_string());
                }
                if reason.trim().is_empty() {
                    return Err("supersessions require a correction reason".to_string());
                }
                let original = self
                    .snapshot
                    .claims
                    .get(claim_id)
                    .ok_or_else(|| format!("unknown claim {claim_id}"))?;
                if matches!(
                    original.status,
                    ClaimStatus::Retracted | ClaimStatus::Superseded
                ) {
                    return Err(format!("claim {claim_id} is already withdrawn"));
                }
                if self.snapshot.claims.contains_key(&replacement.id) {
                    return Err(format!("duplicate claim id {}", replacement.id));
                }
                if self.snapshot.claims.len() >= self.snapshot.charter.limits.max_claims {
                    return Err("Scout claim limit reached".to_string());
                }
                let available = self.snapshot.evidence.keys().cloned().collect();
                super::super::validation::validate_claim_for_ledger(replacement, &available)
            }
            ScoutEventKind::Sealed { disposition } => {
                let check = self.completion_check(*disposition);
                if check.ready {
                    Ok(())
                } else {
                    Err(check.blockers.join("; "))
                }
            }
        }
    }

    pub(super) fn reduce(&mut self, kind: &ScoutEventKind) {
        match kind {
            ScoutEventKind::Chartered { .. } => {}
            ScoutEventKind::PhaseAdvanced { to, .. } => self.snapshot.phase = *to,
            ScoutEventKind::AssignmentIssued { assignment } => {
                self.snapshot.assignments.insert(
                    assignment.id.clone(),
                    AssignmentRecord {
                        assignment: assignment.clone(),
                        status: AssignmentStatus::Issued,
                    },
                );
            }
            ScoutEventKind::WorkerSubmitted { envelope } => {
                self.snapshot
                    .assignments
                    .get_mut(&envelope.assignment_id)
                    .expect("envelope validation checked assignment")
                    .status = AssignmentStatus::Submitted;
                self.reduce_envelope(envelope);
            }
            ScoutEventKind::EvidenceRecorded {
                artifact,
                runner_id,
            } => {
                self.snapshot.evidence.insert(
                    artifact.id.clone(),
                    EvidenceRecord {
                        artifact: artifact.clone(),
                        producer: EvidenceProducer::Runner {
                            runner_id: runner_id.clone(),
                        },
                        checks: Vec::new(),
                    },
                );
            }
            ScoutEventKind::EvidenceChecked { check } => {
                self.snapshot
                    .evidence
                    .get_mut(&check.evidence_id)
                    .expect("evidence check validation checked artifact")
                    .checks
                    .push(check.clone());
            }
            ScoutEventKind::ClaimAdjudicated { decision } => {
                let claim = self
                    .snapshot
                    .claims
                    .get_mut(&decision.claim_id)
                    .expect("adjudication validation checked claim");
                claim.status = match decision.verdict {
                    ScoutVerdict::Supported => ClaimStatus::Supported,
                    ScoutVerdict::Unsupported => ClaimStatus::Unsupported,
                    ScoutVerdict::Unfalsifiable => ClaimStatus::Unfalsifiable,
                };
                claim.adjudication = Some(decision.clone());
            }
            ScoutEventKind::ClaimRetracted { claim_id, reason } => {
                let claim = self
                    .snapshot
                    .claims
                    .get_mut(claim_id)
                    .expect("retraction validation checked claim");
                claim.status = ClaimStatus::Retracted;
                claim.retraction_reason = Some(reason.clone());
            }
            ScoutEventKind::ClaimSuperseded {
                claim_id,
                replacement,
                assignment_id,
                reason,
            } => {
                let original = self
                    .snapshot
                    .claims
                    .get_mut(claim_id)
                    .expect("supersession validation checked claim");
                original.status = ClaimStatus::Superseded;
                original.superseded_by = Some(replacement.id.clone());
                original.supersession_reason = Some(reason.clone());
                self.snapshot.claims.insert(
                    replacement.id.clone(),
                    ClaimRecord {
                        proposal: replacement.clone(),
                        status: initial_status(replacement),
                        originating_assignment: assignment_id.clone(),
                        limitations: Vec::new(),
                        adjudication: None,
                        superseded_by: None,
                        retraction_reason: None,
                        supersession_reason: None,
                    },
                );
            }
            ScoutEventKind::Sealed { disposition } => {
                self.snapshot.phase = ScoutPhase::Sealed;
                self.snapshot.disposition = Some(*disposition);
            }
        }
    }

    fn reduce_envelope(&mut self, envelope: &WorkerEnvelope) {
        for artifact in &envelope.artifacts {
            self.snapshot.evidence.insert(
                artifact.id.clone(),
                EvidenceRecord {
                    artifact: artifact.clone(),
                    producer: EvidenceProducer::Worker {
                        assignment_id: envelope.assignment_id.clone(),
                        role: envelope.role,
                    },
                    checks: Vec::new(),
                },
            );
        }
        for proposal in &envelope.claims {
            self.snapshot.claims.insert(
                proposal.id.clone(),
                ClaimRecord {
                    proposal: proposal.clone(),
                    status: initial_status(proposal),
                    originating_assignment: envelope.assignment_id.clone(),
                    limitations: envelope.limitations.clone(),
                    adjudication: None,
                    superseded_by: None,
                    retraction_reason: None,
                    supersession_reason: None,
                },
            );
        }
        for update in &envelope.claim_updates {
            let claim = self
                .snapshot
                .claims
                .get_mut(&update.claim_id)
                .expect("envelope validation checked claim");
            claim
                .proposal
                .evidence
                .extend(update.evidence.iter().cloned());
            claim
                .proposal
                .counterevidence
                .extend(update.counterevidence.iter().cloned());
            claim.limitations.extend(update.limitations.iter().cloned());
            if matches!(
                claim.status,
                ClaimStatus::Proposed | ClaimStatus::Unfalsifiable
            ) {
                claim.status = ClaimStatus::Checked;
            }
        }
        self.snapshot.coverage.push(envelope.coverage.clone());
        self.snapshot
            .limitations
            .extend(envelope.limitations.iter().cloned());
        self.snapshot
            .requested_followups
            .extend(envelope.requested_followups.iter().cloned());
    }

    pub(super) fn has_independent_reproduction(&self, claim: &ClaimRecord) -> bool {
        claim.proposal.evidence.iter().any(|id| {
            let Some(reproduction) = self.snapshot.evidence.get(id) else {
                return false;
            };
            if reproduction.artifact.kind != EvidenceKind::Reproduction {
                return false;
            }
            let Some(target_id) = &reproduction.artifact.reproduces else {
                return false;
            };
            let Some(target) = self.snapshot.evidence.get(target_id) else {
                return false;
            };
            let independently_checked = reproduction.checks.last().is_some_and(|check| {
                check.outcome == VerificationOutcome::Equivalent
                    || (check.outcome == VerificationOutcome::Exact
                        && check.observed_sha256.as_deref()
                            == Some(target.artifact.content_sha256.as_str()))
            });
            reproduction.producer != target.producer
                && evidence_verified(target)
                && evidence_verified(reproduction)
                && independently_checked
        })
    }
}

pub(super) fn validate_actor(actor: &ScoutActor, kind: &ScoutEventKind) -> Result<(), String> {
    match (actor, kind) {
        (
            ScoutActor::Root,
            ScoutEventKind::Chartered { .. }
            | ScoutEventKind::PhaseAdvanced { .. }
            | ScoutEventKind::AssignmentIssued { .. }
            | ScoutEventKind::ClaimAdjudicated { .. }
            | ScoutEventKind::ClaimRetracted { .. }
            | ScoutEventKind::ClaimSuperseded { .. }
            | ScoutEventKind::Sealed { .. },
        ) => Ok(()),
        (
            ScoutActor::Worker {
                assignment_id: actor_assignment,
            },
            ScoutEventKind::WorkerSubmitted { envelope },
        ) if actor_assignment == &envelope.assignment_id => Ok(()),
        (
            ScoutActor::Runner {
                runner_id: actor_runner,
            },
            ScoutEventKind::EvidenceRecorded { runner_id, .. },
        ) if actor_runner == runner_id => Ok(()),
        (
            ScoutActor::Runner {
                runner_id: actor_runner,
            },
            ScoutEventKind::EvidenceChecked { check },
        ) if actor_runner == &check.verifier => Ok(()),
        _ => Err("Scout event actor is not authorized for this event".to_string()),
    }
}

fn validate_assignment(
    snapshot: &ScoutSnapshot,
    assignment: &ScoutAssignment,
) -> Result<(), String> {
    if assignment.objective.trim().is_empty() {
        return Err("Scout assignments require an objective".to_string());
    }
    if assignment.snapshot_id != snapshot.charter.snapshot_id {
        return Err("Scout assignment targets a different snapshot".to_string());
    }
    if assignment.scopes.is_empty() {
        return Err("Scout assignments require at least one scope".to_string());
    }
    if let Some(scope) = assignment
        .scopes
        .iter()
        .find(|scope| !snapshot.charter.scopes.contains(*scope))
    {
        return Err(format!("Scout assignment uses undeclared scope {scope}"));
    }
    Ok(())
}

fn initial_status(proposal: &ClaimProposal) -> ClaimStatus {
    if proposal.evidence.is_empty() && proposal.counterevidence.is_empty() {
        ClaimStatus::Unfalsifiable
    } else {
        ClaimStatus::Proposed
    }
}
