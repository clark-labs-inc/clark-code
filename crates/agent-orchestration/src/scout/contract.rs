use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! scout_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_id(&value, $label)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

scout_id!(ScoutRunId, "run");
scout_id!(WorkerAssignmentId, "assignment");
scout_id!(RunnerId, "runner");
scout_id!(ClaimId, "claim");
scout_id!(EvidenceId, "evidence");

fn validate_id(value: &str, label: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return Err(format!("{label} ids must contain 1 to 128 characters"));
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!(
            "{label} ids may contain letters, digits, dash, underscore, dot, and colon"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutPhase {
    Charter,
    Map,
    Measure,
    Check,
    Prove,
    Adjudicate,
    Synthesize,
    Sealed,
}

impl ScoutPhase {
    pub fn next(self) -> Option<Self> {
        Some(match self {
            Self::Charter => Self::Map,
            Self::Map => Self::Measure,
            Self::Measure => Self::Check,
            Self::Check => Self::Prove,
            Self::Prove => Self::Adjudicate,
            Self::Adjudicate => Self::Synthesize,
            Self::Synthesize => Self::Sealed,
            Self::Sealed => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutRole {
    Mapper,
    Measurer,
    Prover,
    RedTeam,
    Reproducer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScoutActor {
    Root,
    Worker { assignment_id: WorkerAssignmentId },
    Runner { runner_id: RunnerId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutAssignment {
    pub id: WorkerAssignmentId,
    pub role: ScoutRole,
    pub objective: String,
    pub snapshot_id: String,
    pub scopes: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentStatus {
    Issued,
    Submitted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignmentRecord {
    pub assignment: ScoutAssignment,
    pub status: AssignmentStatus,
}

impl ScoutRole {
    pub fn allowed_in(self, phase: ScoutPhase) -> bool {
        matches!(
            (self, phase),
            (Self::Mapper, ScoutPhase::Map)
                | (Self::Measurer, ScoutPhase::Measure)
                | (Self::Reproducer, ScoutPhase::Check)
                | (
                    Self::Prover | Self::RedTeam | Self::Reproducer,
                    ScoutPhase::Prove
                )
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofTier {
    T1Source,
    T2LiveState,
    T3OfflinePoc,
    T4BenignReachability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    SourceTrace,
    LiveState,
    Census,
    Measurement,
    OfflinePoc,
    BenignReachability,
    Reproduction,
    Counterexample,
    Assumption,
}

impl EvidenceKind {
    pub fn maximum_tier(self) -> Option<ProofTier> {
        match self {
            Self::SourceTrace | Self::Census | Self::Measurement | Self::Assumption => {
                Some(ProofTier::T1Source)
            }
            Self::LiveState => Some(ProofTier::T2LiveState),
            Self::OfflinePoc | Self::Counterexample => Some(ProofTier::T3OfflinePoc),
            Self::BenignReachability => Some(ProofTier::T4BenignReachability),
            Self::Reproduction => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceInterval {
    pub lower: f64,
    pub upper: f64,
    pub confidence: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub sample_size: u64,
    #[serde(default)]
    pub missing: u64,
    pub estimate: f64,
    pub interval: ConfidenceInterval,
    pub method: String,
    pub method_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflinePocControls {
    pub positive_control_sha256: String,
    pub negative_control_sha256: String,
    pub positive_passed: bool,
    pub negative_passed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceArtifact {
    pub id: EvidenceId,
    pub kind: EvidenceKind,
    pub source: String,
    pub content_sha256: String,
    pub observed_at_ms: u64,
    pub snapshot_id: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_tier: Option<ProofTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement: Option<Measurement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_poc_controls: Option<OfflinePocControls>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reproduces: Option<EvidenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceProducer {
    Worker {
        assignment_id: WorkerAssignmentId,
        role: ScoutRole,
    },
    Runner {
        runner_id: RunnerId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Exact,
    Equivalent,
    Changed,
    Unavailable,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceCheck {
    pub evidence_id: EvidenceId,
    pub verifier: RunnerId,
    pub outcome: VerificationOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_sha256: Option<String>,
    pub checked_at_ms: u64,
    pub recipe: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub artifact: EvidenceArtifact,
    pub producer: EvidenceProducer,
    #[serde(default)]
    pub checks: Vec<EvidenceCheck>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimProposal {
    pub id: ClaimId,
    pub text: String,
    #[serde(default)]
    pub headline: bool,
    #[serde(default)]
    pub quantitative: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_tier: Option<ProofTier>,
    #[serde(default)]
    pub evidence: BTreeSet<EvidenceId>,
    #[serde(default)]
    pub counterevidence: BTreeSet<EvidenceId>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_instrument: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimUpdate {
    pub claim_id: ClaimId,
    #[serde(default)]
    pub evidence: BTreeSet<EvidenceId>,
    #[serde(default)]
    pub counterevidence: BTreeSet<EvidenceId>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Proposed,
    Checked,
    Supported,
    Unsupported,
    Unfalsifiable,
    Retracted,
    Superseded,
}

impl ClaimStatus {
    pub fn is_adjudicated(self) -> bool {
        matches!(
            self,
            Self::Supported | Self::Unsupported | Self::Unfalsifiable
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimRecord {
    pub proposal: ClaimProposal,
    pub status: ClaimStatus,
    pub originating_assignment: WorkerAssignmentId,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Adjudication>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<ClaimId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retraction_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersession_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerEnvelope {
    pub assignment_id: WorkerAssignmentId,
    pub role: ScoutRole,
    pub snapshot_id: String,
    #[serde(default)]
    pub artifacts: Vec<EvidenceArtifact>,
    #[serde(default)]
    pub claims: Vec<ClaimProposal>,
    #[serde(default)]
    pub claim_updates: Vec<ClaimUpdate>,
    pub coverage: String,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default)]
    pub requested_followups: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutVerdict {
    Supported,
    Unsupported,
    Unfalsifiable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Adjudication {
    pub claim_id: ClaimId,
    pub verdict: ScoutVerdict,
    pub test: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_tier: Option<ProofTier>,
    #[serde(default)]
    pub addressed_counterevidence: BTreeSet<EvidenceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument_needed: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutCapabilities {
    pub production_read_only: bool,
    pub network_allowed: bool,
    #[serde(default)]
    pub denied: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutLimits {
    pub max_parallel_agents: u16,
    pub max_worker_submissions: usize,
    pub max_claims: usize,
    pub max_artifacts: usize,
    pub max_events: usize,
}

impl Default for ScoutLimits {
    fn default() -> Self {
        Self {
            max_parallel_agents: 4,
            max_worker_submissions: 64,
            max_claims: 256,
            max_artifacts: 512,
            max_events: 2_048,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoutCharter {
    pub run_id: ScoutRunId,
    pub objective: String,
    pub snapshot_id: String,
    pub capability_census_id: String,
    pub capability_fingerprint: String,
    pub scopes: BTreeSet<String>,
    #[serde(default)]
    pub exclusions: BTreeSet<String>,
    pub capabilities: ScoutCapabilities,
    #[serde(default)]
    pub limits: ScoutLimits,
    #[serde(default = "default_minimum_power")]
    pub minimum_power: f64,
}

fn default_minimum_power() -> f64 {
    0.8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealDisposition {
    Complete,
    Partial,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScoutEventKind {
    Chartered {
        charter: ScoutCharter,
    },
    PhaseAdvanced {
        from: ScoutPhase,
        to: ScoutPhase,
    },
    AssignmentIssued {
        assignment: ScoutAssignment,
    },
    WorkerSubmitted {
        envelope: WorkerEnvelope,
    },
    EvidenceRecorded {
        artifact: EvidenceArtifact,
        runner_id: RunnerId,
    },
    EvidenceChecked {
        check: EvidenceCheck,
    },
    ClaimAdjudicated {
        decision: Adjudication,
    },
    ClaimRetracted {
        claim_id: ClaimId,
        reason: String,
    },
    ClaimSuperseded {
        claim_id: ClaimId,
        replacement: ClaimProposal,
        assignment_id: WorkerAssignmentId,
        reason: String,
    },
    Sealed {
        disposition: SealDisposition,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoutEvent {
    pub sequence: u64,
    pub run_id: ScoutRunId,
    pub actor: ScoutActor,
    #[serde(flatten)]
    pub kind: ScoutEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoutSnapshot {
    pub charter: ScoutCharter,
    pub phase: ScoutPhase,
    pub claims: BTreeMap<ClaimId, ClaimRecord>,
    pub evidence: BTreeMap<EvidenceId, EvidenceRecord>,
    pub assignments: BTreeMap<WorkerAssignmentId, AssignmentRecord>,
    pub coverage: Vec<String>,
    pub limitations: Vec<String>,
    pub requested_followups: Vec<String>,
    pub event_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<SealDisposition>,
}
