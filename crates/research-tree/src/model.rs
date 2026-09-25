use serde::{Deserialize, Serialize};

pub use clark_autoresearch::{DispatchHint, SurfaceKind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub scope_id: String,
    pub source_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_nodes: usize,
    pub max_steps: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub surface: SurfaceKind,
    pub in_scope: bool,
    pub requires_validation: bool,
    pub priority: f64,
    pub novelty: f64,
    pub confidence: f64,
    pub impact: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Active,
    Completed,
    Refuted,
    Blocked,
    Pruned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Supports,
    Refutes,
    Expands,
    Inconclusive,
    Blocked,
}

/// A caller assertion, never an independent verification of evidence or scope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub key: String,
    pub parent: Option<String>,
    pub links: Vec<String>,
    pub dependencies: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub rationale: String,
    pub assessment: Assessment,
    pub status: Status,
    pub outcome: Option<Outcome>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Proposed {
        key: String,
        parent: Option<String>,
        links: Vec<String>,
        dependencies: Vec<String>,
        evidence_refs: Vec<String>,
        rationale: String,
        assessment: Assessment,
    },
    Reassessed {
        key: String,
        evidence_refs: Vec<String>,
        rationale: String,
        assessment: Assessment,
    },
    Started {
        key: String,
    },
    Recorded {
        key: String,
        outcome: Outcome,
        evidence_refs: Vec<String>,
        rationale: String,
    },
    Reopened {
        key: String,
        evidence_refs: Vec<String>,
        rationale: String,
    },
    Pruned {
        key: String,
        rationale: String,
    },
}

/// Persist this journal, then replay it. Derived node snapshots are not authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub identity: Identity,
    pub limits: Limits,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventResult {
    Created(String),
    Merged(String),
    Updated(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    #[error("invalid research input: {0}")]
    Invalid(String),
    #[error("unknown research node: {0}")]
    UnknownNode(String),
    #[error("invalid research transition: {0}")]
    Transition(String),
    #[error("research budget exhausted: {0}")]
    Budget(String),
}
