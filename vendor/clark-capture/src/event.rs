use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub type JsonMap = Map<String, Value>;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Trace,
    Debug,
    #[default]
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Breadcrumb {
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Level>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub data: JsonMap,
}

impl Breadcrumb {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            category: None,
            kind: None,
            level: None,
            message: Some(message.into()),
            data: JsonMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RuntimeContext {
    pub language: String,
    pub version: String,
    pub pid: u32,
    pub executable: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CaptureEvent {
    pub schema_version: u8,
    pub event_id: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub level: Level,
    pub project: String,
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub runtime: RuntimeContext,
    pub tags: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<JsonMap>,
    pub contexts: JsonMap,
    pub extra: JsonMap,
    pub breadcrumbs: Vec<Breadcrumb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
    pub payload: JsonMap,
}

#[derive(Clone, Debug)]
pub struct EventInput {
    pub kind: String,
    pub level: Level,
    pub timestamp: Option<String>,
    pub tags: BTreeMap<String, String>,
    pub user: Option<JsonMap>,
    pub contexts: JsonMap,
    pub extra: JsonMap,
    pub breadcrumbs: Option<Vec<Breadcrumb>>,
    pub trace: Option<TraceContext>,
    pub payload: JsonMap,
}

impl EventInput {
    pub fn new(kind: impl Into<String>, level: Level, payload: JsonMap) -> Self {
        Self {
            kind: kind.into(),
            level,
            timestamp: None,
            tags: BTreeMap::new(),
            user: None,
            contexts: JsonMap::new(),
            extra: JsonMap::new(),
            breadcrumbs: None,
            trace: None,
            payload,
        }
    }
}
