//! Project-level Clark config: `<root>/.clark/settings.json`. Read once at
//! `new_session` through the session [`Executor`] (so it works for remote/SSH
//! projects too), and layered under the global, UI-driven config
//! ([`crate::config::LocalConfig`]) rather than replacing it: permission
//! arrays union, while `check_command`, `hooks`, and commit attribution are
//! project-scoped values.
//!
//! This is intentionally a single flat file, not a directory of fragments —
//! see `.claude/settings.json` for the convention this mirrors.

use std::path::Path;

use serde::Deserialize;

use crate::exec::Executor;
use crate::markdown_frontmatter::read_json;

pub const DEFAULT_COMMIT_ATTRIBUTION: &str = "Co-Authored-By: Clark Code <noreply@clarkchat.com>";

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
    // Exercised by the settings-parsing tests; kept as a public helper.
    #[allow(dead_code)]
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
pub struct AttributionConfig {
    /// Full commit attribution text, including any trailers. An empty string
    /// disables commit attribution, matching Claude Code's settings contract.
    #[serde(default)]
    pub commit: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ProjectSettings {
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub check_command: Option<String>,
    #[serde(default)]
    pub attribution: Option<AttributionConfig>,
    #[serde(
        default,
        rename = "includeCoAuthoredBy",
        alias = "include_co_authored_by"
    )]
    pub include_co_authored_by: Option<bool>,
    #[serde(
        default,
        rename = "includeGitInstructions",
        alias = "include_git_instructions"
    )]
    pub include_git_instructions: Option<bool>,
}

impl ProjectSettings {
    pub fn commit_attribution(&self) -> &str {
        if let Some(attribution) = &self.attribution {
            return attribution
                .commit
                .as_deref()
                .unwrap_or(DEFAULT_COMMIT_ATTRIBUTION);
        }
        if self.include_co_authored_by == Some(false) {
            return "";
        }
        DEFAULT_COMMIT_ATTRIBUTION
    }

    pub fn include_git_instructions(&self) -> bool {
        self.include_git_instructions.unwrap_or(true)
    }
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
    fn parses_hooks_permissions_check_command_and_attribution() {
        let json = serde_json::json!({
            "hooks": {
                "PreToolUse": [{"matcher": "bash", "command": "echo pre"}],
                "PostToolUse": [{"matcher": "*", "command": "echo post"}]
            },
            "permissions": { "allow": ["cargo test"], "deny": ["rm -rf /"] },
            "check_command": "cargo check",
            "attribution": {
                "commit": "Co-Authored-By: Custom Agent <agent@example.com>"
            },
            "includeGitInstructions": false
        });
        let settings: ProjectSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.hooks.pre_tool_use.len(), 1);
        assert_eq!(settings.hooks.pre_tool_use[0].matcher, "bash");
        assert_eq!(settings.hooks.post_tool_use[0].command, "echo post");
        assert_eq!(settings.permissions.allow, vec!["cargo test"]);
        assert_eq!(settings.permissions.deny, vec!["rm -rf /"]);
        assert_eq!(settings.check_command.as_deref(), Some("cargo check"));
        assert_eq!(
            settings.commit_attribution(),
            "Co-Authored-By: Custom Agent <agent@example.com>"
        );
        assert!(!settings.include_git_instructions());
    }

    #[test]
    fn attribution_matches_claude_defaults_customization_and_opt_out() {
        let defaults = ProjectSettings::default();
        assert_eq!(defaults.commit_attribution(), DEFAULT_COMMIT_ATTRIBUTION);
        assert!(defaults.include_git_instructions());

        let disabled: ProjectSettings = serde_json::from_value(serde_json::json!({
            "attribution": { "commit": "" }
        }))
        .unwrap();
        assert_eq!(disabled.commit_attribution(), "");

        let legacy_disabled: ProjectSettings = serde_json::from_value(serde_json::json!({
            "includeCoAuthoredBy": false
        }))
        .unwrap();
        assert_eq!(legacy_disabled.commit_attribution(), "");

        let object_without_commit: ProjectSettings =
            serde_json::from_value(serde_json::json!({ "attribution": {} })).unwrap();
        assert_eq!(
            object_without_commit.commit_attribution(),
            DEFAULT_COMMIT_ATTRIBUTION
        );
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
