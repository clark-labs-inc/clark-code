pub(super) fn census_scope_covers(census: &str, requested: &str) -> bool {
    let Some(census) = normalized_scope(census) else {
        return false;
    };
    let Some(requested) = normalized_scope(requested) else {
        return false;
    };
    if census.is_empty() {
        true
    } else if requested.is_empty() {
        false
    } else {
        requested == census || requested.starts_with(&format!("{census}/"))
    }
}

fn normalized_scope(scope: &str) -> Option<String> {
    if scope.trim() != scope || scope.contains(':') {
        return None;
    }
    if matches!(scope, "." | "repo") {
        return Some(String::new());
    }
    let scope = scope.replace('\\', "/");
    if scope.is_empty() || scope.starts_with('/') {
        return None;
    }
    let scope = scope.trim_end_matches('/');
    if scope.is_empty()
        || scope
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        None
    } else {
        Some(scope.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_census_covers_portable_relative_scopes() {
        assert!(census_scope_covers(".", "services/api"));
        assert!(census_scope_covers("repo", r"services\api"));
    }

    #[test]
    fn nested_census_does_not_escape_or_cover_siblings() {
        assert!(census_scope_covers("services/api", "services/api/src"));
        assert!(!census_scope_covers("services/api", "services/api/../web"));
        assert!(!census_scope_covers("services/api", "services/web"));
    }

    #[test]
    fn absolute_and_drive_qualified_scopes_are_rejected() {
        assert!(!census_scope_covers(".", "/etc"));
        assert!(!census_scope_covers(".", r"C:\Windows"));
        assert!(!census_scope_covers(".", r"\\server\share"));
    }
}
