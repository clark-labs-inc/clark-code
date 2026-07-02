//! Project-level Clark config: `<root>/.clark/settings.json`. Read once at
//! `new_session` through the session [`Executor`] (so it works for remote/SSH
//! projects too), and layered under the global, UI-driven config
//! ([`crate::config::LocalConfig`]) rather than replacing it: permission
//! arrays union, `check_command`/`hooks` are simple project-scoped values.
//!
//! This is intentionally a single flat file, not a directory of fragments —
//! see `.claude/settings.json` for the convention this mirrors.

use std::path::Path;

use serde::Deserialize;

use crate::exec::Executor;
use crate::markdown_frontmatter::read_json;

/// One `PreToolUse`/`PostToolUse` hook entry.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct HookEntry {
    /// A tool name, or `*` to match every tool.
    pub matcher: String,
    /// The shell command to run.
    pub command: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct HooksConfig {
    #[serde(default, rename = "PreToolUse")]
    pub pre_tool_use: Vec<HookEntry>,
    #[serde(default, rename = "PostToolUse")]
    pub post_tool_use: Vec<HookEntry>,
}

impl HooksConfig {
    pub fn is_empty(&self) -> bool {
        self.pre_tool_use.is_empty() && self.post_tool_use.is_empty()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ProjectSettings {
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub check_command: Option<String>,
}

/// Read `<root>/.clark/settings.json` through `exec`. Missing file, unreadable,
/// or malformed JSON all degrade silently to defaults — project settings are
/// optional, never required for a session to start.
pub async fn load(exec: &dyn Executor, root: &Path) -> ProjectSettings {
    read_json(exec, &root.join(".clark/settings.json"))
        .await
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Union two command lists, de-duplicated, order-preserving (`a` first).
pub fn union_unique(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    a.into_iter()
        .chain(b)
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    #[test]
    fn union_unique_dedupes_preserving_first_occurrence_order() {
        let a = vec!["cargo test".to_string(), "npm run build".to_string()];
        let b = vec!["npm run build".to_string(), "rm -rf".to_string()];
        assert_eq!(
            union_unique(a, b),
            vec!["cargo test", "npm run build", "rm -rf"]
        );
    }

    #[test]
    fn parses_hooks_permissions_and_check_command() {
        let json = serde_json::json!({
            "hooks": {
                "PreToolUse": [{"matcher": "bash", "command": "echo pre"}],
                "PostToolUse": [{"matcher": "*", "command": "echo post"}]
            },
            "permissions": { "allow": ["cargo test"], "deny": ["rm -rf /"] },
            "check_command": "cargo check"
        });
        let settings: ProjectSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.hooks.pre_tool_use.len(), 1);
        assert_eq!(settings.hooks.pre_tool_use[0].matcher, "bash");
        assert_eq!(settings.hooks.post_tool_use[0].command, "echo post");
        assert_eq!(settings.permissions.allow, vec!["cargo test"]);
        assert_eq!(settings.permissions.deny, vec!["rm -rf /"]);
        assert_eq!(settings.check_command.as_deref(), Some("cargo check"));
    }

    #[tokio::test]
    async fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load(&LocalExecutor, dir.path()).await;
        assert!(settings.hooks.is_empty());
        assert!(settings.permissions.allow.is_empty());
        assert!(settings.check_command.is_none());
    }

    #[tokio::test]
    async fn reads_settings_file_through_executor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".clark")).unwrap();
        std::fs::write(
            dir.path().join(".clark/settings.json"),
            r#"{"check_command": "tsc --noEmit"}"#,
        )
        .unwrap();
        let settings = load(&LocalExecutor, dir.path()).await;
        assert_eq!(settings.check_command.as_deref(), Some("tsc --noEmit"));
    }
}
