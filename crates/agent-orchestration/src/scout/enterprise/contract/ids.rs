use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

macro_rules! string_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_identifier($label, &value)?;
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

string_id!(EnterpriseId, "enterprise");
string_id!(EnterpriseEntityId, "enterprise entity");
string_id!(EnterpriseEdgeId, "enterprise edge");
string_id!(EnterpriseEventId, "enterprise event");
string_id!(EnterpriseBatchId, "enterprise batch");
string_id!(CoverageCellId, "coverage cell");
string_id!(FrontierTaskId, "frontier task");

pub(crate) fn canonical_digest<T: Serialize>(value: &T) -> Result<String, String> {
    let encoded = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

pub(super) fn validate_text(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{label} exceeds the {max_bytes}-byte limit"));
    }
    if value.contains('\0') {
        return Err(format!("{label} contains a null byte"));
    }
    if looks_secret_bearing(value) {
        return Err(format!(
            "{label} appears to contain secret material; submit a reference or digest instead"
        ));
    }
    Ok(())
}

pub(super) fn validate_string_set(
    label: &str,
    values: &BTreeSet<String>,
    max_items: usize,
    max_bytes: usize,
) -> Result<(), String> {
    if values.len() > max_items {
        return Err(format!("{label} exceeds the {max_items}-item limit"));
    }
    for value in values {
        validate_text(label, value, max_bytes)?;
    }
    Ok(())
}

pub(super) fn validate_evidence(values: &BTreeSet<String>) -> Result<(), String> {
    if values.is_empty() {
        return Err("enterprise observations require at least one evidence digest".into());
    }
    if values.len() > 256 {
        return Err("enterprise observations exceed the 256-evidence limit".into());
    }
    for value in values {
        validate_digest("evidence digest", value)?;
    }
    Ok(())
}

pub(super) fn validate_digest(label: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} must be a 64-character hexadecimal digest"));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed != value {
        return Err(format!("{label} ids cannot contain surrounding whitespace"));
    }
    if trimmed.is_empty() || trimmed.len() > 256 {
        return Err(format!("{label} ids must contain 1 to 256 characters"));
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

fn looks_secret_bearing(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    if upper.contains("-----BEGIN")
        || upper.starts_with("BEARER ")
        || upper.starts_with("BASIC ")
        || ["GHP_", "GHO_", "GHS_", "GHR_", "GITHUB_PAT_"]
            .iter()
            .any(|prefix| upper.starts_with(prefix))
        || ((upper.starts_with("AKIA") || upper.starts_with("ASIA"))
            && value.len() >= 20
            && value
                .bytes()
                .take(20)
                .all(|byte| byte.is_ascii_alphanumeric()))
    {
        return true;
    }
    if [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "API_KEY",
        "COOKIE",
        "SESSION",
    ]
    .iter()
    .any(|marker| {
        upper.contains(&format!("{marker}=")) || upper.contains(&format!("\"{marker}\":"))
    }) {
        return true;
    }
    if let Some(scheme_end) = value.find("://") {
        let authority = &value[scheme_end + 3..];
        if let Some(at) = authority.find('@') {
            if authority[..at].contains(':') {
                return true;
            }
        }
    }
    let jwt_parts = value.split('.').collect::<Vec<_>>();
    jwt_parts.len() == 3
        && value.len() > 60
        && jwt_parts.iter().all(|part| {
            part.len() >= 8
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}
