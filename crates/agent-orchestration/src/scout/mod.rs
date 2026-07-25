//! Evidence-first system cartography primitives.
//!
//! Scout workers are replaceable, read-only sensors. The ledger is the
//! canonical single-writer authority: workers may propose claims and attach
//! evidence, while only the root may adjudicate, retract, supersede, or seal.

mod contract;
mod ledger;
mod measurement;
mod report;
mod validation;

pub use contract::{
    Adjudication, AssignmentRecord, AssignmentStatus, ClaimId, ClaimProposal, ClaimRecord,
    ClaimStatus, ClaimUpdate, ConfidenceInterval, EvidenceArtifact, EvidenceCheck, EvidenceId,
    EvidenceKind, EvidenceProducer, EvidenceRecord, Measurement, OfflinePocControls, ProofTier,
    RunnerId, ScoutActor, ScoutAssignment, ScoutCapabilities, ScoutCharter, ScoutEvent,
    ScoutEventKind, ScoutLimits, ScoutPhase, ScoutRole, ScoutRunId, ScoutSnapshot, ScoutVerdict,
    SealDisposition, VerificationOutcome, WorkerAssignmentId, WorkerEnvelope,
};
pub use ledger::{CompletionCheck, ScoutLedger};
pub use measurement::{compute_measurement, MeasurementComputation, MeasurementMethod};

#[cfg(test)]
mod tests;
