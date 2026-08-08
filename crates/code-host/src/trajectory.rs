use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryStatus {
    Started,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryRecord {
    pub schema_version: u32,
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub request_id: String,
    pub plugin: String,
    pub operation: String,
    pub status: TrajectoryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl TrajectoryRecord {
    pub(crate) fn path(root: &std::path::Path) -> PathBuf {
        root.join("trajectory.jsonl")
    }
}

pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
