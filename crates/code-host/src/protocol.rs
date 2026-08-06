use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PROTOCOL_VERSION;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub schema_version: u32,
    pub request_id: String,
    #[serde(flatten)]
    pub command: RequestCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum RequestCommand {
    Ping,
    Catalog,
    Invoke {
        plugin: String,
        operation: String,
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        input: Value,
    },
    Cancel {
        target_request_id: String,
    },
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    /// Ordered, non-terminal output for one still-running request. A terminal
    /// `result` or `error` with the same request id must follow exactly once.
    Progress {
        schema_version: u32,
        request_id: Option<String>,
        sequence: u64,
        kind: String,
        data: Value,
    },
    Result {
        schema_version: u32,
        request_id: Option<String>,
        kind: String,
        data: Value,
    },
    Error {
        schema_version: u32,
        request_id: Option<String>,
        code: String,
        message: String,
    },
}

impl Response {
    pub fn progress(
        request_id: impl Into<String>,
        sequence: u64,
        kind: impl Into<String>,
        data: Value,
    ) -> Self {
        Self::Progress {
            schema_version: PROTOCOL_VERSION,
            request_id: Some(request_id.into()),
            sequence,
            kind: kind.into(),
            data,
        }
    }

    pub fn result(request_id: Option<String>, kind: impl Into<String>, data: Value) -> Self {
        Self::Result {
            schema_version: PROTOCOL_VERSION,
            request_id,
            kind: kind.into(),
            data,
        }
    }

    pub fn error(
        request_id: Option<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Error {
            schema_version: PROTOCOL_VERSION,
            request_id,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn with_request_id(mut self, request_id: String) -> Self {
        match &mut self {
            Self::Progress { request_id: id, .. }
            | Self::Result { request_id: id, .. }
            | Self::Error { request_id: id, .. } => {
                *id = Some(request_id);
            }
        }
        self
    }

    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Progress { request_id, .. }
            | Self::Result { request_id, .. }
            | Self::Error { request_id, .. } => request_id.as_deref(),
        }
    }

    pub fn schema_version(&self) -> u32 {
        match self {
            Self::Progress { schema_version, .. }
            | Self::Result { schema_version, .. }
            | Self::Error { schema_version, .. } => *schema_version,
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Progress { .. })
    }
}

impl Request {
    /// Strictly decode one JSONL control message. `serde(deny_unknown_fields)`
    /// cannot cover a flattened tagged enum, so ignored paths are rejected
    /// explicitly rather than silently widening the protocol.
    pub fn from_json_str(input: &str) -> Result<Self, String> {
        let original: Value = serde_json::from_str(input).map_err(|error| error.to_string())?;
        let mut deserializer = serde_json::Deserializer::from_str(input);
        let mut ignored = Vec::new();
        let request: Self = serde_ignored::deserialize(&mut deserializer, |path| {
            ignored.push(path.to_string());
        })
        .map_err(|error| error.to_string())?;
        deserializer.end().map_err(|error| error.to_string())?;
        if let (Some(original), Ok(canonical)) =
            (original.as_object(), serde_json::to_value(&request))
        {
            if let Some(canonical) = canonical.as_object() {
                ignored.extend(
                    original
                        .keys()
                        .filter(|key| !canonical.contains_key(*key))
                        .cloned(),
                );
            }
        }
        if ignored.is_empty() {
            Ok(request)
        } else {
            ignored.sort();
            ignored.dedup();
            Err(format!("unknown request field(s): {}", ignored.join(", ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_keeps_the_wire_strict() {
        let request = Request::from_json_str(&format!(
            r#"{{"schema_version":{PROTOCOL_VERSION},"request_id":"ping-1","command":"ping"}}"#
        ))
        .unwrap();
        assert!(matches!(request.command, RequestCommand::Ping));

        let error = Request::from_json_str(&format!(
            r#"{{"schema_version":{PROTOCOL_VERSION},"request_id":"ping-1","command":"ping","surprise":true}}"#
        ))
        .unwrap_err();
        assert!(error.contains("surprise"));
    }

    #[test]
    fn response_correlation_is_preserved() {
        let response = Response::result(Some("request-1".into()), "ok", Value::Null);
        assert_eq!(response.request_id(), Some("request-1"));
        assert_eq!(
            response.with_request_id("request-2".into()).request_id(),
            Some("request-2")
        );
    }

    #[test]
    fn progress_is_ordered_and_non_terminal() {
        let response = Response::progress("request-1", 3, "agent_event", Value::Null);
        assert_eq!(response.request_id(), Some("request-1"));
        assert_eq!(response.schema_version(), PROTOCOL_VERSION);
        assert!(!response.is_terminal());
    }

    #[test]
    fn response_decoder_rejects_unknown_fields() {
        let value = format!(
            r#"{{"type":"progress","schema_version":{PROTOCOL_VERSION},"request_id":"request-1","sequence":0,"kind":"agent_event","data":null,"surprise":true}}"#
        );
        assert!(serde_json::from_str::<Response>(&value).is_err());
    }
}
