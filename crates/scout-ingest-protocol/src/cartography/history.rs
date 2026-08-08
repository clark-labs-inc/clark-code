use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ObservationEvent;

pub const DEFAULT_GRAPH_SNAPSHOT_LIMIT: u16 = 100;
pub const MAX_GRAPH_SNAPSHOT_LIMIT: u16 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphObjectKind {
    Entity,
    Edge,
    Claim,
    Coverage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshotCursor {
    pub effective_at_ms: u64,
    pub known_at_ms: u64,
    pub object_kind: GraphObjectKind,
    pub object_id: String,
    pub filter_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshotQuery {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub effective_at_ms: Option<u64>,
    pub known_at_ms: Option<u64>,
    #[serde(default)]
    pub object_kinds: BTreeSet<GraphObjectKind>,
    #[serde(default = "default_graph_snapshot_limit")]
    pub limit: u16,
    pub cursor: Option<GraphSnapshotCursor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshotEntry {
    pub object_kind: GraphObjectKind,
    pub object_id: String,
    pub run_id: Uuid,
    pub machine_id: Uuid,
    pub accepted_at_ms: u64,
    pub event: ObservationEvent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshotPage {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub effective_at_ms: u64,
    pub known_at_ms: u64,
    pub entries: Vec<GraphSnapshotEntry>,
    pub next_cursor: Option<GraphSnapshotCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshotRef {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub effective_at_ms: u64,
    pub known_at_ms: u64,
    pub filter_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphChangeKind {
    Added,
    Changed,
    Removed,
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDeltaCursor {
    pub from_effective_at_ms: u64,
    pub from_known_at_ms: u64,
    pub to_effective_at_ms: u64,
    pub to_known_at_ms: u64,
    pub object_kind: GraphObjectKind,
    pub object_id: String,
    pub filter_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDeltaQuery {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub from_effective_at_ms: u64,
    pub from_known_at_ms: Option<u64>,
    pub to_effective_at_ms: Option<u64>,
    pub to_known_at_ms: Option<u64>,
    #[serde(default)]
    pub object_kinds: BTreeSet<GraphObjectKind>,
    #[serde(default)]
    pub include_unchanged: bool,
    #[serde(default = "default_graph_snapshot_limit")]
    pub limit: u16,
    pub cursor: Option<GraphDeltaCursor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDeltaEntry {
    pub object_kind: GraphObjectKind,
    pub object_id: String,
    pub change: GraphChangeKind,
    pub before: Option<GraphSnapshotEntry>,
    pub after: Option<GraphSnapshotEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDeltaPage {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub from_snapshot: GraphSnapshotRef,
    pub to_snapshot: GraphSnapshotRef,
    pub entries: Vec<GraphDeltaEntry>,
    pub next_cursor: Option<GraphDeltaCursor>,
}

const fn default_graph_snapshot_limit() -> u16 {
    DEFAULT_GRAPH_SNAPSHOT_LIMIT
}
