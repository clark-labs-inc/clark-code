use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type CapsuleResult<T> = Result<T, CapsuleError>;

#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapsuleError {
    #[error("{resource} limit exceeded: observed {observed}, limit {limit}")]
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        observed: u64,
    },
    #[error("malformed JSON at line {line}, column {column}")]
    MalformedJson { line: usize, column: usize },
    #[error("invalid capsule request field {field}: {reason}")]
    InvalidRequest { field: &'static str, reason: String },
    #[error("duplicate {field}")]
    Duplicate { field: &'static str },
    #[error("normalized output serialization failed")]
    Serialization,
}

impl CapsuleError {
    pub(crate) fn limit(resource: &'static str, limit: usize, observed: usize) -> Self {
        Self::LimitExceeded {
            resource,
            limit: limit as u64,
            observed: observed as u64,
        }
    }

    pub(crate) fn invalid(field: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidRequest {
            field,
            reason: reason.into(),
        }
    }
}
