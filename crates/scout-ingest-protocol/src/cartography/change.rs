use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

pub const DEFAULT_CHANGE_LIMIT: u16 = 100;
pub const MAX_CHANGE_LIMIT: u16 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CartographyChangeQuery {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    #[serde(default)]
    pub after_sequence: u64,
    #[serde(default = "default_change_limit")]
    pub limit: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CartographyChange {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub sequence: u64,
    pub event_type: String,
    pub occurred_at_ms: u64,
    pub payload: JsonValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CartographyChangePage {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub changes: Vec<CartographyChange>,
    pub next_after_sequence: u64,
}

const fn default_change_limit() -> u16 {
    DEFAULT_CHANGE_LIMIT
}
