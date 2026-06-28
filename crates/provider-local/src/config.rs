//! Connection + behavior config for the local agent, parsed from
//! [`agent_core::ProviderConfig`].
//!
//! Everything routes through the production Clark Platform API
//! (`https://api.clarkslabs.com/v1`) authenticated with a single `ck_live_…`
//! key. No URLs are user-configurable; the only required input is the key (and a
//! project folder). A `base_url` override in `extra` exists solely for tests.

use std::collections::HashMap;

use agent_core::provider::ProviderConfig;
use serde_json::Value;

use crate::tools::PermissionMode;

/// Production Clark Platform API (OpenAI-compatible) base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.clarkslabs.com/v1";
/// Default coding model: the passthrough tool-calling tier (native tool_calls,
/// no internal sandbox loop), which the local loop drives with its own tools.
pub const DEFAULT_MODEL: &str = "clark-code";
/// Agentic Clark model used for research / memory extraction (no client tools).
pub const DEFAULT_RESEARCH_MODEL: &str = "clark";
/// Safety cap on tool-call ping-pong within a single turn.
pub const DEFAULT_MAX_ITERATIONS: u32 = 50;

/// Resolved configuration for one local-agent session.
#[derive(Clone, Debug)]
pub struct LocalConfig {
    /// OpenAI-compatible base URL (no trailing `/chat/completions`).
    pub base_url: String,
    /// Model id sent in the request body.
    pub model: String,
    /// Bearer `ck_live_…` Platform API key.
    pub api_key: Option<String>,
    /// Extra HTTP headers, if any.
    pub headers: HashMap<String, String>,
    /// Sampling temperature, if pinned.
    pub temperature: Option<f32>,
    /// Hard cap on model<->tool iterations per turn.
    pub max_iterations: u32,
    /// Default permission mode per (mutating) tool name.
    pub permissions: HashMap<String, PermissionMode>,
    /// Shell-command prefixes the user has chosen to always allow (skip the
    /// gate). Only honored for commands that classify Safe/Caution, so a trusted
    /// prefix can't smuggle a destructive suffix past the gate.
    pub command_allowlist: Vec<String>,
    /// Shell-command prefixes that are always refused.
    pub command_denylist: Vec<String>,
    /// MCP servers to connect and expose as tools.
    pub mcp_servers: Vec<crate::mcp::McpServerConfig>,
    /// Clark research config (same Platform API + key), when research is enabled.
    pub clark: Option<ClarkResearchConfig>,
    /// Project root, when set at connect time. A session's `cwd` option wins.
    pub cwd: Option<String>,
}

/// How the `clark_research` tool reaches Clark: the same Platform API + key, with
/// an agentic model. Clark runs web search / planning / browsing server-side and
/// returns the final answer — no client tools involved, so the `ck_live_` key
/// suffices.
#[derive(Clone, Debug)]
pub struct ClarkResearchConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    /// Agentic model the research run uses (e.g. `clark`, `clark_max`).
    pub model: String,
}

fn str_field(extra: &Value, key: &str) -> Option<String> {
    extra
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn str_vec(extra: &Value, key: &str) -> Vec<String> {
    extra
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

impl LocalConfig {
    /// Parse from the generic [`ProviderConfig`]. Unknown keys are ignored.
    ///
    /// Recognized `extra` keys: `model`, `temperature`, `max_iterations`,
    /// `permissions` (map of tool→`allow|ask|deny`), `research` (bool, default
    /// true), `research_model`, and `base_url` (tests only). The key rides on
    /// `auth_token`.
    pub fn from_provider_config(config: &ProviderConfig) -> Self {
        let extra = &config.extra;

        let base_url = str_field(extra, "base_url").unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let model = str_field(extra, "model").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let api_key = config
            .auth_token
            .clone()
            .or_else(|| str_field(extra, "api_key"));

        let temperature = extra
            .get("temperature")
            .and_then(Value::as_f64)
            .map(|t| t as f32);
        let max_iterations = extra
            .get("max_iterations")
            .and_then(Value::as_u64)
            .map(|n| n as u32)
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MAX_ITERATIONS);

        let mut permissions = default_permissions();
        if let Some(map) = extra.get("permissions").and_then(Value::as_object) {
            for (tool, mode) in map {
                if let Some(mode) = mode.as_str().and_then(PermissionMode::parse) {
                    permissions.insert(tool.clone(), mode);
                }
            }
        }

        // Research is on by default and uses the same Platform API + key; it only
        // needs a key to function.
        let research_enabled = extra
            .get("research")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let clark = (research_enabled && api_key.is_some()).then(|| ClarkResearchConfig {
            base_url: base_url.clone(),
            api_key: api_key.clone(),
            model: str_field(extra, "research_model")
                .unwrap_or_else(|| DEFAULT_RESEARCH_MODEL.to_string()),
        });

        let cwd = config.cwd.clone().or_else(|| str_field(extra, "cwd"));

        Self {
            base_url,
            model,
            api_key,
            headers: config.headers.clone(),
            temperature,
            max_iterations,
            permissions,
            command_allowlist: str_vec(extra, "command_allowlist"),
            command_denylist: str_vec(extra, "command_denylist"),
            mcp_servers: extra
                .get("mcp_servers")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            clark,
            cwd,
        }
    }

    /// Resolve a tool's effective permission mode (read-only tools aren't listed
    /// here and are governed by [`crate::tools::ToolExecutor::mutating`]).
    pub fn mode_for(&self, tool: &str) -> PermissionMode {
        self.permissions
            .get(tool)
            .copied()
            .unwrap_or(PermissionMode::Ask)
    }
}

/// Mutating tools default to asking; read-only tools never reach the gate.
fn default_permissions() -> HashMap<String, PermissionMode> {
    [
        ("write_file", PermissionMode::Ask),
        ("edit_file", PermissionMode::Ask),
        ("bash", PermissionMode::Ask),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_to_production_clark_and_no_research_without_key() {
        let cfg = LocalConfig::from_provider_config(&ProviderConfig::default());
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.model, DEFAULT_MODEL);
        assert_eq!(cfg.max_iterations, DEFAULT_MAX_ITERATIONS);
        assert_eq!(cfg.mode_for("bash"), PermissionMode::Ask);
        // No key → research can't run, so it's disabled.
        assert!(cfg.clark.is_none());
    }

    #[test]
    fn a_key_enables_research_through_the_same_api() {
        let pc = ProviderConfig {
            auth_token: Some("ck_live_abc".into()),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.api_key.as_deref(), Some("ck_live_abc"));
        let clark = cfg.clark.expect("research enabled when a key is present");
        assert_eq!(clark.base_url, DEFAULT_BASE_URL);
        assert_eq!(clark.api_key.as_deref(), Some("ck_live_abc"));
        assert_eq!(clark.model, DEFAULT_RESEARCH_MODEL);
    }

    #[test]
    fn extra_overrides_model_permissions_and_can_disable_research() {
        let pc = ProviderConfig {
            auth_token: Some("ck_live_abc".into()),
            extra: json!({
                "base_url": "http://localhost:1234/v1",
                "model": "clark_max",
                "temperature": 0.2,
                "max_iterations": 8,
                "permissions": { "bash": "deny", "edit_file": "allow" },
                "research": false
            }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.base_url, "http://localhost:1234/v1");
        assert_eq!(cfg.model, "clark_max");
        assert_eq!(cfg.temperature, Some(0.2));
        assert_eq!(cfg.max_iterations, 8);
        assert_eq!(cfg.mode_for("bash"), PermissionMode::Deny);
        assert_eq!(cfg.mode_for("edit_file"), PermissionMode::Allow);
        assert_eq!(cfg.mode_for("write_file"), PermissionMode::Ask);
        assert!(
            cfg.clark.is_none(),
            "research:false disables it even with a key"
        );
    }
}
