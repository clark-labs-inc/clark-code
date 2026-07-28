use std::collections::{BTreeMap, BTreeSet};

use crate::{ProtocolError, ProtocolResult, SafeFieldValue};

pub(crate) const MAX_SAFE_TEXT_BYTES: usize = 4_096;

pub(crate) fn validate_digest(field: &'static str, value: &str) -> ProtocolResult<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProtocolError::invalid(
            field,
            "must be a lowercase 64-character SHA-256 digest",
        ));
    }
    Ok(())
}

pub(crate) fn validate_identifier(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> ProtocolResult<()> {
    if value.trim() != value || value.is_empty() || value.len() > max_bytes {
        return Err(ProtocolError::invalid(
            field,
            format!("must contain 1 to {max_bytes} characters without surrounding whitespace"),
        ));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(ProtocolError::invalid(
            field,
            "contains characters outside the portable identifier alphabet",
        ));
    }
    Ok(())
}

pub(crate) fn validate_safe_text(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> ProtocolResult<()> {
    if value.trim() != value || value.is_empty() || value.len() > max_bytes {
        return Err(ProtocolError::invalid(
            field,
            format!("must contain 1 to {max_bytes} bytes without surrounding whitespace"),
        ));
    }
    if value
        .bytes()
        .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(ProtocolError::invalid(field, "contains a control byte"));
    }
    if looks_protected(value) {
        return Err(ProtocolError::invalid(
            field,
            "appears to contain credential or protected cursor material",
        ));
    }
    Ok(())
}

pub(crate) fn validate_field_name(value: &str) -> ProtocolResult<()> {
    validate_identifier("field_name", value, 128)?;
    let normalized = value.to_ascii_lowercase().replace('-', "_");
    if [
        "authorization",
        "cookie",
        "credential",
        "credentials",
        "access_key",
        "secret_access_key",
        "api_key",
        "password",
        "private_key",
        "refresh_token",
        "access_token",
        "id_token",
        "session_token",
        "next_token",
        "next_page_token",
        "page_token",
        "cursor",
    ]
    .contains(&normalized.as_str())
    {
        return Err(ProtocolError::invalid(
            "field_name",
            "is reserved for protected material",
        ));
    }
    Ok(())
}

pub(crate) fn validate_string_set(
    field: &'static str,
    values: &BTreeSet<String>,
    max_items: usize,
    max_bytes: usize,
) -> ProtocolResult<()> {
    if values.len() > max_items {
        return Err(ProtocolError::invalid(
            field,
            format!("exceeds the {max_items}-item limit"),
        ));
    }
    for value in values {
        validate_safe_text(field, value, max_bytes)?;
    }
    Ok(())
}

pub(crate) fn validate_fields(
    fields: &BTreeMap<String, SafeFieldValue>,
    max_items: usize,
) -> ProtocolResult<()> {
    if fields.len() > max_items {
        return Err(ProtocolError::invalid(
            "fields",
            format!("exceeds the {max_items}-item limit"),
        ));
    }
    for (name, value) in fields {
        validate_field_name(name)?;
        value.validate()?;
    }
    Ok(())
}

fn looks_protected(value: &str) -> bool {
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
        "CREDENTIAL",
    ]
    .iter()
    .any(|marker| {
        upper.contains(&format!("{marker}=")) || upper.contains(&format!("\"{marker}\":"))
    }) {
        return true;
    }
    if let Some(scheme_end) = value.find("://") {
        let authority = &value[scheme_end + 3..];
        if authority
            .split_once('@')
            .is_some_and(|(userinfo, _)| userinfo.contains(':'))
        {
            return true;
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
