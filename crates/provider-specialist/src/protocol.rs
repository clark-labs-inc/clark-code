use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
pub struct WorkerRequest {
    pub schema_version: u32,
    pub request_id: String,
    #[serde(flatten)]
    pub command: WorkerCommand,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WorkerCommand {
    Ping,
    SpecialistCatalog,
    SpecialistTurn {
        session_id: String,
        specialist: String,
        workflow: String,
        project_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        scout_context: Option<Value>,
        message: String,
        now_ms: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerResponse {
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

impl WorkerResponse {
    pub fn from_json_str(input: &str) -> Result<Self, String> {
        let original: Value = serde_json::from_str(input).map_err(|error| error.to_string())?;
        let mut deserializer = serde_json::Deserializer::from_str(input);
        let mut ignored = Vec::new();
        let response: Self = serde_ignored::deserialize(&mut deserializer, |path| {
            ignored.push(path.to_string());
        })
        .map_err(|error| error.to_string())?;
        deserializer.end().map_err(|error| error.to_string())?;
        let canonical = serde_json::to_value(&response).map_err(|error| error.to_string())?;
        if let (Some(original), Some(canonical)) = (original.as_object(), canonical.as_object()) {
            ignored.extend(
                original
                    .keys()
                    .filter(|key| !canonical.contains_key(*key))
                    .cloned(),
            );
        }
        ignored.sort();
        ignored.dedup();
        if ignored.is_empty() {
            Ok(response)
        } else {
            Err(format!(
                "unknown worker response field(s): {}",
                ignored.join(", ")
            ))
        }
    }

    pub fn into_result(
        self,
        expected_request_id: &str,
        expected_kind: &str,
    ) -> Result<Value, String> {
        match self {
            Self::Result {
                schema_version,
                request_id,
                kind,
                data,
            } => {
                validate_identity(schema_version, request_id.as_deref(), expected_request_id)?;
                if kind != expected_kind {
                    return Err(format!(
                        "worker returned kind {kind:?}; expected {expected_kind:?}"
                    ));
                }
                Ok(data)
            }
            Self::Error {
                schema_version,
                request_id,
                code,
                message,
            } => {
                validate_identity(schema_version, request_id.as_deref(), expected_request_id)?;
                Err(format!("worker {code}: {message}"))
            }
        }
    }
}

fn validate_identity(
    schema_version: u32,
    request_id: Option<&str>,
    expected_request_id: &str,
) -> Result<(), String> {
    if schema_version != 1 {
        return Err(format!(
            "worker returned unsupported schema version {schema_version}"
        ));
    }
    if request_id != Some(expected_request_id) {
        return Err("worker response request identity did not match".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_correlation_fails_closed() {
        let response = WorkerResponse::Result {
            schema_version: 1,
            request_id: Some("other".into()),
            kind: "pong".into(),
            data: Value::Null,
        };
        assert!(response.into_result("expected", "pong").is_err());
    }

    #[test]
    fn worker_errors_preserve_machine_code() {
        let response = WorkerResponse::Error {
            schema_version: 1,
            request_id: Some("request-1".into()),
            code: "specialist_turn_failed".into(),
            message: "invalid workflow".into(),
        };
        let error = response
            .into_result("request-1", "specialist_turn")
            .unwrap_err();
        assert!(error.contains("specialist_turn_failed"));
        assert!(error.contains("invalid workflow"));
    }

    #[test]
    fn response_parser_rejects_unknown_fields() {
        let error = WorkerResponse::from_json_str(
            r#"{"type":"result","schema_version":1,"request_id":"request-1","kind":"pong","data":{},"surprise":true}"#,
        )
        .unwrap_err();
        assert!(error.contains("surprise"));
    }
}
