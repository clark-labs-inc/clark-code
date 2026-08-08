use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use super::{GraphObjectKind, GraphSnapshotRef};

pub const MAX_SIMULATION_MEMBERSHIPS_PER_PUBLISH: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationOverlayStatus {
    Draft,
    Ready,
    Running,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationCoverageState {
    Covered,
    Partial,
    OutsideContract,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationResultState {
    NotRun,
    Passed,
    Failed,
    Diverged,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationObjectRef {
    pub object_kind: GraphObjectKind,
    pub object_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationMembership {
    pub object: SimulationObjectRef,
    pub coverage: SimulationCoverageState,
    pub result: SimulationResultState,
    pub confidence_basis_points: u16,
    pub rationale: String,
    #[serde(default)]
    pub evidence_event_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishSimulationOverlay {
    pub stable_key: String,
    pub name: String,
    pub status: SimulationOverlayStatus,
    pub snapshot: GraphSnapshotRef,
    pub memberships: Vec<SimulationMembership>,
    pub summary: JsonValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationOverlayRecord {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub stable_key: String,
    pub version: u64,
    pub name: String,
    pub status: SimulationOverlayStatus,
    pub snapshot: GraphSnapshotRef,
    pub content_sha256: String,
    pub summary: JsonValue,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationOverlayCursor {
    pub simulation_id: Uuid,
    pub content_sha256: String,
    pub object_kind: GraphObjectKind,
    pub object_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationOverlayQuery {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub stable_key: String,
    pub version: Option<u64>,
    pub limit: u16,
    pub cursor: Option<SimulationOverlayCursor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationOverlayPage {
    pub overlay: SimulationOverlayRecord,
    pub memberships: Vec<SimulationMembership>,
    pub next_cursor: Option<SimulationOverlayCursor>,
}
