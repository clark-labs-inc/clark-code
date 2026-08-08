use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

mod events;

use self::events::validate_actor;
use super::validation::validate_charter;
use super::{
    Adjudication, ClaimId, ClaimProposal, ClaimStatus, EvidenceArtifact, EvidenceCheck, RunnerId,
    ScoutActor, ScoutAssignment, ScoutCharter, ScoutEvent, ScoutEventKind, ScoutPhase,
    ScoutSnapshot, SealDisposition, WorkerAssignmentId, WorkerEnvelope,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionCheck {
    pub ready: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug)]
pub struct ScoutLedger {
    snapshot: ScoutSnapshot,
    events: Vec<ScoutEvent>,
}

impl ScoutLedger {
    pub fn new(charter: ScoutCharter) -> Result<Self, String> {
        validate_charter(&charter)?;
        let mut ledger = Self::empty(charter.clone());
        ledger.append(ScoutActor::Root, ScoutEventKind::Chartered { charter })?;
        Ok(ledger)
    }

    pub fn replay(events: Vec<ScoutEvent>) -> Result<Self, String> {
        let Some(first) = events.first() else {
            return Err("Scout replay requires at least one event".to_string());
        };
        let ScoutEventKind::Chartered { charter } = &first.kind else {
            return Err("first Scout event must be chartered".to_string());
        };
        if first.sequence != 1 || first.run_id != charter.run_id {
            return Err("first Scout event has invalid identity or sequence".to_string());
        }

        let mut ledger = Self::empty(charter.clone());
        for event in events {
            let expected = ledger.events.len() as u64 + 1;
            if event.sequence != expected {
                return Err(format!(
                    "Scout event sequence mismatch: expected {expected}, got {}",
                    event.sequence
                ));
            }
            if event.run_id != ledger.snapshot.charter.run_id {
                return Err("Scout event belongs to a different run".to_string());
            }
            validate_actor(&event.actor, &event.kind)?;
            ledger.validate_kind(&event.kind)?;
            ledger.reduce(&event.kind);
            ledger.events.push(event);
            ledger.snapshot.event_count = ledger.events.len();
        }
        Ok(ledger)
    }

    pub fn snapshot(&self) -> &ScoutSnapshot {
        &self.snapshot
    }

    pub fn events(&self) -> &[ScoutEvent] {
        &self.events
    }

    pub fn advance(&mut self, to: ScoutPhase) -> Result<(), String> {
        self.append(
            ScoutActor::Root,
            ScoutEventKind::PhaseAdvanced {
                from: self.snapshot.phase,
                to,
            },
        )
    }

    pub fn issue_assignment(&mut self, assignment: ScoutAssignment) -> Result<(), String> {
        self.append(
            ScoutActor::Root,
            ScoutEventKind::AssignmentIssued { assignment },
        )
    }

    pub fn submit(&mut self, envelope: WorkerEnvelope) -> Result<(), String> {
        let actor = ScoutActor::Worker {
            assignment_id: envelope.assignment_id.clone(),
        };
        self.append(actor, ScoutEventKind::WorkerSubmitted { envelope })
    }

    pub fn record_evidence(
        &mut self,
        artifact: EvidenceArtifact,
        runner_id: RunnerId,
    ) -> Result<(), String> {
        let actor = ScoutActor::Runner {
            runner_id: runner_id.clone(),
        };
        self.append(
            actor,
            ScoutEventKind::EvidenceRecorded {
                artifact,
                runner_id,
            },
        )
    }

    pub fn check_evidence(&mut self, check: EvidenceCheck) -> Result<(), String> {
        let actor = ScoutActor::Runner {
            runner_id: check.verifier.clone(),
        };
        self.append(actor, ScoutEventKind::EvidenceChecked { check })
    }

    pub fn adjudicate(&mut self, decision: Adjudication) -> Result<(), String> {
        self.append(
            ScoutActor::Root,
            ScoutEventKind::ClaimAdjudicated { decision },
        )
    }

    pub fn retract(&mut self, claim_id: ClaimId, reason: String) -> Result<(), String> {
        self.append(
            ScoutActor::Root,
            ScoutEventKind::ClaimRetracted { claim_id, reason },
        )
    }

    pub fn supersede(
        &mut self,
        claim_id: ClaimId,
        replacement: ClaimProposal,
        assignment_id: WorkerAssignmentId,
        reason: String,
    ) -> Result<(), String> {
        self.append(
            ScoutActor::Root,
            ScoutEventKind::ClaimSuperseded {
                claim_id,
                replacement,
                assignment_id,
                reason,
            },
        )
    }

    pub fn completion_check(&self, disposition: SealDisposition) -> CompletionCheck {
        let mut blockers = Vec::new();
        if self.snapshot.phase != ScoutPhase::Synthesize {
            blockers.push("Scout may only seal from the synthesize phase".to_string());
        }
        if self.snapshot.claims.is_empty() {
            blockers.push("Scout cannot seal without any claims".to_string());
        }
        match disposition {
            SealDisposition::Complete => {
                for claim in self.snapshot.claims.values() {
                    if claim.proposal.headline && !claim.status.is_adjudicated() {
                        blockers.push(format!(
                            "headline claim {} is not adjudicated",
                            claim.proposal.id
                        ));
                    }
                    if claim.proposal.headline
                        && claim.status == ClaimStatus::Supported
                        && !self.has_independent_reproduction(claim)
                    {
                        blockers.push(format!(
                            "supported headline claim {} lacks independently checked reproduction",
                            claim.proposal.id
                        ));
                    }
                }
            }
            SealDisposition::Partial => {
                if self.snapshot.limitations.is_empty()
                    && self.snapshot.requested_followups.is_empty()
                {
                    blockers.push(
                        "partial Scout runs must name a limitation or requested follow-up"
                            .to_string(),
                    );
                }
            }
        }
        CompletionCheck {
            ready: blockers.is_empty(),
            blockers,
        }
    }

    pub fn seal(&mut self, disposition: SealDisposition) -> Result<(), String> {
        let check = self.completion_check(disposition);
        if !check.ready {
            return Err(check.blockers.join("; "));
        }
        self.append(ScoutActor::Root, ScoutEventKind::Sealed { disposition })
    }

    pub fn fingerprint(&self) -> Result<String, String> {
        let encoded = serde_json::to_vec(&self.events).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }

    pub fn report_markdown(&self) -> String {
        super::report::render(&self.snapshot, self.fingerprint().ok().as_deref())
    }

    fn empty(charter: ScoutCharter) -> Self {
        Self {
            snapshot: ScoutSnapshot {
                charter,
                phase: ScoutPhase::Charter,
                claims: BTreeMap::new(),
                evidence: BTreeMap::new(),
                assignments: BTreeMap::new(),
                coverage: Vec::new(),
                limitations: Vec::new(),
                requested_followups: Vec::new(),
                event_count: 0,
                disposition: None,
            },
            events: Vec::new(),
        }
    }

    fn append(&mut self, actor: ScoutActor, kind: ScoutEventKind) -> Result<(), String> {
        validate_actor(&actor, &kind)?;
        self.validate_kind(&kind)?;
        if self.events.len() >= self.snapshot.charter.limits.max_events {
            return Err("Scout event limit reached".to_string());
        }
        let event = ScoutEvent {
            sequence: self.events.len() as u64 + 1,
            run_id: self.snapshot.charter.run_id.clone(),
            actor,
            kind,
        };
        self.reduce(&event.kind);
        self.events.push(event);
        self.snapshot.event_count = self.events.len();
        Ok(())
    }
}
