//! Portable wire contract for an authoritative system-cartography API.
//!
//! These types intentionally do not depend on a particular backend repository.
//! They reimplement the public JSON/signature contract so desktop, SSH, and VM
//! collectors remain portable untrusted sensors.

mod change;
mod crypto;
mod history;
mod model;
mod simulation;

pub use change::{
    CartographyChange, CartographyChangePage, CartographyChangeQuery, DEFAULT_CHANGE_LIMIT,
    MAX_CHANGE_LIMIT,
};
pub use crypto::CollectorSigningKey;
pub use history::{
    GraphChangeKind, GraphDeltaCursor, GraphDeltaEntry, GraphDeltaPage, GraphDeltaQuery,
    GraphObjectKind, GraphSnapshotCursor, GraphSnapshotEntry, GraphSnapshotPage,
    GraphSnapshotQuery, GraphSnapshotRef, DEFAULT_GRAPH_SNAPSHOT_LIMIT, MAX_GRAPH_SNAPSHOT_LIMIT,
};
pub use model::{
    BatchAcceptance, BatchEnvelope, BatchReceipt, ClaimIdentity, ClaimTarget, ClaimedTask,
    Classification, EdgeIdentity, EntityIdentity, EvidenceCommitOutcome, EvidenceCommitRequest,
    EvidenceObjectRef, EvidenceStatus, EvidenceUploadAuthorization, EvidenceUploadGrant,
    EvidenceUploadRequest, ObservationEvent, ObservationFact, ObservationSubject, ReceiptOutcome,
    TaskClaimRequest, TaskClaimResponse, TaskCompletion, TerminalDisposition, UploadHeader,
    MAX_BATCH_BYTES, MAX_EVENTS_PER_BATCH, SYSTEM_CARTOGRAPHY_SCHEMA_VERSION,
};
pub use simulation::{
    PublishSimulationOverlay, SimulationCoverageState, SimulationMembership, SimulationObjectRef,
    SimulationOverlayCursor, SimulationOverlayPage, SimulationOverlayQuery,
    SimulationOverlayRecord, SimulationOverlayStatus, SimulationResultState,
    MAX_SIMULATION_MEMBERSHIPS_PER_PUBLISH,
};
