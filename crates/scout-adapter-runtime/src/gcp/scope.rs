use crate::error::{RuntimeError, RuntimeResult};

pub(super) fn validate_scope(scope: &str) -> RuntimeResult<()> {
    if scope == "global"
        || numeric_scope(scope, "organizations/").is_some()
        || numeric_scope(scope, "folders/").is_some()
        || scope
            .strip_prefix("projects/")
            .is_some_and(portable_project)
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRequest)
    }
}

pub(super) fn hierarchy_parent(scope: &str) -> RuntimeResult<(&'static str, &str)> {
    if let Some(id) = numeric_scope(scope, "organizations/") {
        Ok(("organization", id))
    } else if let Some(id) = numeric_scope(scope, "folders/") {
        Ok(("folder", id))
    } else {
        Err(RuntimeError::InvalidRequest)
    }
}

fn numeric_scope<'a>(scope: &'a str, prefix: &str) -> Option<&'a str> {
    scope
        .strip_prefix(prefix)
        .filter(|id| !id.is_empty() && id.len() <= 32)
        .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn portable_project(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
