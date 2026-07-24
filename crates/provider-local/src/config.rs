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

use crate::compaction::CompactionConfig;
use crate::tools::PermissionMode;

/// Production Clark Platform API (OpenAI-compatible) base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.clarkslabs.com/v1";
/// Default coding model: the passthrough tool-calling tier (native tool_calls,
/// no internal sandbox loop), which the local loop drives with its own tools.
pub const DEFAULT_MODEL: &str = "clark-code";
/// Agentic Clark model used for research / memory extraction (no client tools).
pub const DEFAULT_RESEARCH_MODEL: &str = "clark";
/// Stateless multimodal model used to describe image attachments that the
/// active coding model cannot see. Keep this on the provider-qualified
/// passthrough path: agentic Clark tiers own a sandbox/session and may answer
/// the user's task instead of returning a grounded pixel description.
pub const DEFAULT_VISION_MODEL: &str = "google/gemini-3.1-flash-lite";
/// Approximate transcript-token threshold where the local loop checkpoints old
/// context before the next model request. GLM 5.2 gives us a 1M-token window, so
/// leave plenty of room for the next turn instead of compacting early.
pub const DEFAULT_AUTO_COMPACT_TOKEN_LIMIT: usize = 300_000;

/// Known context windows for the clark-code tier models (mirrors the stated
/// windows in `app/src/lib/localAgent.ts`). Only models with a product-stated
/// window are listed; unknown models fall back to the flat default plus the
/// engine's overflow-recovery path.
fn model_context_window(model: &str) -> Option<usize> {
    match model {
        "clark-code" => Some(1_000_000),                     // GLM 5.2
        "clark-code:minimax_m3" => Some(1_000_000),          // "1M context"
        "clark-code:kimi_k3" => Some(1_000_000),             // "1M context"
        "clark-code:kimi_k27_code" => Some(262_144),         // observed provider contract
        "clark-code:grok45" => Some(500_000),                // "500K context"
        "clark-code:deepseek_v4_pro" => Some(1_000_000),     // "1M context"
        "clark-code:gemini35_flash_lite" => Some(1_000_000), // "1M context"
        _ => None,
    }
}

/// Provider-safe output ceilings for models whose upstream default would
/// otherwise consume the entire context window. Kimi K2.7's provider defaults
/// to 262,144 output tokens when omitted, which makes every non-empty prompt
/// invalid against its 262,144-token total context.
pub(crate) fn model_max_output_tokens(model: &str) -> Option<u32> {
    match model {
        "clark-code:kimi_k27_code" => Some(32_768),
        _ => None,
    }
}

/// Whether the selected coding tier accepts image content parts directly.
/// Models not listed here keep using the separate vision-description fallback.
pub(crate) fn model_supports_images(model: &str) -> bool {
    matches!(
        model,
        "clark-code:minimax_m3" | "clark-code:kimi_k3" | "clark-code:gemini35_flash_lite"
    )
}

/// Effective auto-compaction threshold for `model`: the flat default, lowered
/// (never raised — 300k on a 1M window is a deliberate cost choice) to 80% of
/// the model's known context window when that is smaller. Applied
/// defensively: a model whose whole window is under the flat default must
/// compact before it overflows, not after.
pub(crate) fn default_auto_compact_limit(model: &str) -> usize {
    match model_context_window(model) {
        Some(window) => DEFAULT_AUTO_COMPACT_TOKEN_LIMIT.min(window.saturating_mul(4) / 5),
        None => DEFAULT_AUTO_COMPACT_TOKEN_LIMIT,
    }
}
/// Approximate source budget for the summarization request itself.
pub const DEFAULT_COMPACT_REQUEST_TOKEN_LIMIT: usize = 250_000;
/// Approximate budget for preserving recent real user messages after compaction.
pub const DEFAULT_COMPACT_RECENT_USER_TOKEN_BUDGET: usize = 20_000;

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
    /// Reasoning-effort override sent with each coding request ("low" …
    /// "xhigh"). `None` → the model's server-side default.
    pub reasoning_effort: Option<String>,
    /// Optional hard cap on model<->tool iterations per turn. Production leaves
    /// this unbounded; tests and evals may set an explicit cap.
    pub max_iterations: Option<u32>,
    /// Hidden A/B switch used by the paid planning benchmark. Production and
    /// normal tests always use the decision-complete profile.
    pub(crate) planning_prompt_profile: crate::planning::PlanningPromptProfile,
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
    pub clark: Option<AgenticClarkConfig>,
    /// Vision-fallback config for coding models without native image support.
    /// Independent of the `research` toggle — gated only on a key being
    /// present.
    pub vision: Option<AgenticClarkConfig>,
    /// Project root, when set at connect time. A session's `cwd` option wins.
    pub cwd: Option<String>,
    /// When set, this session's tool I/O runs on a **remote** host (over the
    /// exec-server) rather than locally. The host fills this in after it brings
    /// up the SSH tunnel + server.
    pub remote: Option<RemoteTarget>,
    /// Local OS containment policy. `Auto` and `Required` fail session creation
    /// when containment cannot be established; `Disabled` is an explicit
    /// host-capability mode.
    pub sandbox_mode: LocalSandboxMode,
    /// Whether durable memory is enabled — exposes the `memory` tool and injects
    /// the project + global memory into the system prompt. On by default; the
    /// user turns it off from the profile menu (`extra.memories = false`).
    pub memories_enabled: bool,
    /// Whether this opted-in session may retrieve private repository evidence
    /// previously synced to the user's Clark account.
    pub project_knowledge_enabled: bool,
    /// Checkpoint compaction for the model-visible transcript.
    pub compaction: CompactionConfig,
    /// Experimental: register the `browser` tool (clark-browser, lazily
    /// downloaded on first use). Off by default — the user opts in from
    /// Settings (`extra.browser_enabled = true`).
    pub browser_enabled: bool,
    /// Opt-in control of ordinary apps on the local Mac. The tool layer still
    /// enforces per-app permission scopes and macOS TCC independently.
    pub computer_use_enabled: bool,
    /// Native in production; the deterministic simulator is accepted only by
    /// debug builds and test harnesses.
    pub computer_use_backend: ComputerUseBackend,
    /// Bounded local multi-agent orchestration. Available by default, while its
    /// model-facing policy remains explicit-request-only.
    pub(crate) orchestration: crate::orchestration::OrchestrationConfig,
    /// Universal root execution lifecycle. This is always present; its limits
    /// control bounded recovery and accounting rather than tool permissions.
    pub(crate) execution: crate::root_execution::RootExecutionConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalSandboxMode {
    Auto,
    Required,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputerUseBackend {
    Native,
    Simulated,
}

impl ComputerUseBackend {
    fn from_extra(extra: &Value) -> Self {
        if cfg!(debug_assertions)
            && extra.get("computer_use_backend").and_then(Value::as_str) == Some("simulated")
        {
            Self::Simulated
        } else {
            Self::Native
        }
    }
}

impl LocalSandboxMode {
    fn from_extra(extra: &Value) -> Self {
        match extra.get("sandbox_mode").and_then(Value::as_str) {
            Some("required") => Self::Required,
            Some("disabled") => Self::Disabled,
            _ => Self::Auto,
        }
    }
}

/// A remote project target. The agent's file/shell tools run against `cwd` on a
/// remote host, reached through a `clark-exec-server` at `ws_url` (a local port
/// the host forwarded over SSH) and authenticated with `token`.
#[derive(Clone, Debug)]
pub struct RemoteTarget {
    /// `ws://127.0.0.1:<forwarded-port>` — the local end of the SSH tunnel.
    pub ws_url: String,
    /// Per-session capability token the exec-server checks on `auth`.
    pub token: String,
    /// Absolute project root **on the remote host**.
    pub cwd: String,
}

/// Config for calling one of Clark's agentic model tiers (e.g. `clark`,
/// `clark_max`) as an auxiliary, non-coding call over the same Platform API +
/// key as the coding model — used by `clark_research`, the `web_fetch`
/// long-page condenser, and the image-description vision fallback. Clark runs
/// web search / planning / browsing / vision server-side and returns the
/// final answer — no client tools involved, so the `ck_live_` key suffices.
#[derive(Clone, Debug)]
pub struct AgenticClarkConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    /// Agentic model tier this call uses (e.g. `clark`, `clark_max`).
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

fn usize_field(extra: &Value, key: &str) -> Option<usize> {
    extra
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}

impl LocalConfig {
    /// Parse from the generic [`ProviderConfig`]. Unknown keys are ignored.
    ///
    /// Recognized `extra` keys: `model`, `temperature`, `max_iterations`,
    /// `permissions` (map of tool→`allow|ask|deny`), `research` (bool, default
    /// true), `research_model`, `vision_model`, `auto_compact` (bool),
    /// `auto_compact_token_limit`, `compact_request_token_limit`,
    /// `compact_recent_user_token_budget`, and `base_url` (tests only). The key
    /// rides on `auth_token`.
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
        let reasoning_effort = str_field(extra, "reasoning_effort");
        let max_iterations = extra
            .get("max_iterations")
            .and_then(Value::as_u64)
            .map(|n| n as u32)
            .filter(|n| *n > 0);
        let planning_prompt_profile = crate::planning::PlanningPromptProfile::from_extra(
            extra.get("planning_prompt_profile").and_then(Value::as_str),
        );

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
        let clark = (research_enabled && api_key.is_some()).then(|| AgenticClarkConfig {
            base_url: base_url.clone(),
            api_key: api_key.clone(),
            model: str_field(extra, "research_model")
                .unwrap_or_else(|| DEFAULT_RESEARCH_MODEL.to_string()),
        });

        // Vision fallback is core functionality for models without native
        // image support, not the opt-out-able research feature — gated only on
        // a key.
        let vision = api_key.is_some().then(|| AgenticClarkConfig {
            base_url: base_url.clone(),
            api_key: api_key.clone(),
            model: str_field(extra, "vision_model")
                .unwrap_or_else(|| DEFAULT_VISION_MODEL.to_string()),
        });

        let cwd = config.cwd.clone().or_else(|| str_field(extra, "cwd"));

        // Memory is on by default; the profile toggle sends `memories = false`.
        let memories_enabled = extra
            .get("memories")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let project_knowledge_enabled = extra
            .get("project_knowledge")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Off by default — the user opts in from Settings.
        let browser_enabled = extra
            .get("browser_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let computer_use_enabled = extra
            .get("computer_use_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let computer_use_backend = ComputerUseBackend::from_extra(extra);
        let mut orchestration = crate::orchestration::OrchestrationConfig::from_extra(extra);
        let execution = crate::root_execution::RootExecutionConfig::from_extra(extra);

        let compaction = if extra
            .get("auto_compact")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            let auto_compact_token_limit = usize_field(extra, "auto_compact_token_limit")
                .filter(|n| *n > 0)
                .unwrap_or_else(|| default_auto_compact_limit(&model));
            let compact_request_token_limit = usize_field(extra, "compact_request_token_limit")
                .filter(|n| *n > 0)
                .unwrap_or_else(|| {
                    DEFAULT_COMPACT_REQUEST_TOKEN_LIMIT.min(auto_compact_token_limit)
                });
            let recent_user_token_budget = usize_field(extra, "compact_recent_user_token_budget")
                .filter(|n| *n > 0)
                .unwrap_or(DEFAULT_COMPACT_RECENT_USER_TOKEN_BUDGET);
            CompactionConfig {
                auto_compact_token_limit,
                compact_request_token_limit,
                recent_user_token_budget,
                ..CompactionConfig::default()
            }
        } else {
            CompactionConfig::disabled()
        };

        // A remote project rides in on `extra.remote = { ws_url, token, cwd }`,
        // populated by the host once the SSH tunnel + exec-server are up. All
        // three fields are required; a partial object is treated as "local".
        let remote = extra.get("remote").and_then(|r| {
            Some(RemoteTarget {
                ws_url: str_field(r, "ws_url")?,
                token: str_field(r, "token")?,
                cwd: str_field(r, "cwd")?,
            })
        });
        let sandbox_mode = LocalSandboxMode::from_extra(extra);
        // The first orchestration boundary launches nested local/ACP processes.
        // A remote project needs a remote-aware child transport and read-only
        // enforcement on that host, neither of which this adapter can prove yet.
        if remote.is_some() {
            orchestration.enabled = false;
        }

        Self {
            base_url,
            model,
            api_key,
            headers: config.headers.clone(),
            temperature,
            reasoning_effort,
            max_iterations,
            planning_prompt_profile,
            permissions,
            command_allowlist: str_vec(extra, "command_allowlist"),
            command_denylist: str_vec(extra, "command_denylist"),
            mcp_servers: extra
                .get("mcp_servers")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            clark,
            vision,
            cwd,
            remote,
            sandbox_mode,
            memories_enabled,
            project_knowledge_enabled,
            compaction,
            browser_enabled,
            computer_use_enabled,
            computer_use_backend,
            orchestration,
            execution,
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
        assert_eq!(cfg.max_iterations, None);
        assert_eq!(cfg.sandbox_mode, LocalSandboxMode::Auto);
        assert_eq!(
            cfg.compaction.auto_compact_token_limit,
            DEFAULT_AUTO_COMPACT_TOKEN_LIMIT
        );
        assert_eq!(
            cfg.compaction.compact_request_token_limit,
            DEFAULT_COMPACT_REQUEST_TOKEN_LIMIT
        );
        assert_eq!(cfg.mode_for("bash"), PermissionMode::Ask);
        assert!(!cfg.computer_use_enabled);
        assert_eq!(cfg.computer_use_backend, ComputerUseBackend::Native);
        assert!(cfg.orchestration.enabled);
        assert_eq!(
            cfg.orchestration.mode,
            crate::orchestration::DelegationMode::ExplicitRequestOnly
        );
        // No key → research can't run, so it's disabled.
        assert!(cfg.clark.is_none());
        // No key → vision fallback can't run either.
        assert!(cfg.vision.is_none());
    }

    #[test]
    fn current_multimodal_model_options_support_native_images() {
        assert!(model_supports_images("clark-code:minimax_m3"));
        assert!(model_supports_images("clark-code:kimi_k3"));
        assert!(model_supports_images("clark-code:gemini35_flash_lite"));
        assert!(!model_supports_images("clark-code"));
        assert!(!model_supports_images("clark-code:kimi_k27_code"));
    }

    #[test]
    fn new_efficient_options_use_their_product_stated_context_windows() {
        for model in ["clark-code:minimax_m3", "clark-code:gemini35_flash_lite"] {
            assert_eq!(model_context_window(model), Some(1_000_000));
            assert_eq!(
                default_auto_compact_limit(model),
                DEFAULT_AUTO_COMPACT_TOKEN_LIMIT
            );
        }
    }

    #[test]
    fn deepseek_v4_pro_uses_its_product_stated_context_window() {
        assert_eq!(
            model_context_window("clark-code:deepseek_v4_pro"),
            Some(1_000_000)
        );
        assert_eq!(
            default_auto_compact_limit("clark-code:deepseek_v4_pro"),
            DEFAULT_AUTO_COMPACT_TOKEN_LIMIT
        );
    }

    #[test]
    fn kimi_k27_reserves_context_for_a_bounded_completion() {
        assert_eq!(
            model_context_window("clark-code:kimi_k27_code"),
            Some(262_144)
        );
        assert_eq!(
            model_max_output_tokens("clark-code:kimi_k27_code"),
            Some(32_768)
        );
        assert_eq!(
            default_auto_compact_limit("clark-code:kimi_k27_code"),
            209_715
        );
    }

    #[test]
    fn default_auto_sandbox_keeps_clark_cloud_enabled_through_the_same_api() {
        let pc = ProviderConfig {
            auth_token: Some("ck_live_abc".into()),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.sandbox_mode, LocalSandboxMode::Auto);
        assert_eq!(cfg.api_key.as_deref(), Some("ck_live_abc"));
        let clark = cfg.clark.expect("research enabled when a key is present");
        assert_eq!(clark.base_url, DEFAULT_BASE_URL);
        assert_eq!(clark.api_key.as_deref(), Some("ck_live_abc"));
        assert_eq!(clark.model, DEFAULT_RESEARCH_MODEL);
    }

    #[test]
    fn a_key_enables_the_vision_fallback_through_the_same_api() {
        let pc = ProviderConfig {
            auth_token: Some("ck_live_abc".into()),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        let vision = cfg
            .vision
            .expect("vision fallback enabled when a key is present");
        assert_eq!(vision.base_url, DEFAULT_BASE_URL);
        assert_eq!(vision.api_key.as_deref(), Some("ck_live_abc"));
        assert_eq!(vision.model, DEFAULT_VISION_MODEL);
        assert_eq!(vision.model, "google/gemini-3.1-flash-lite");
    }

    #[test]
    fn vision_stays_enabled_when_research_is_disabled() {
        let pc = ProviderConfig {
            auth_token: Some("ck_live_abc".into()),
            extra: json!({ "research": false }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert!(
            cfg.clark.is_none(),
            "research:false disables the research config"
        );
        assert!(
            cfg.vision.is_some(),
            "vision fallback is core functionality, not gated by the research toggle"
        );
    }

    #[test]
    fn vision_model_override_uses_its_own_extra_key() {
        let pc = ProviderConfig {
            auth_token: Some("ck_live_abc".into()),
            extra: json!({ "vision_model": "clark_max", "research_model": "clark" }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.vision.expect("vision enabled").model, "clark_max");
        assert_eq!(cfg.clark.expect("research enabled").model, "clark");
    }

    #[test]
    fn parses_remote_target_when_fully_specified() {
        let pc = ProviderConfig {
            auth_token: Some("ck_live_abc".into()),
            extra: json!({
                "remote": {
                    "ws_url": "ws://127.0.0.1:54321",
                    "token": "cap-token",
                    "cwd": "/home/me/project"
                },
                "orchestration": {"enabled": true}
            }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert!(!cfg.orchestration.enabled);
        let remote = cfg.remote.expect("remote target parsed");
        assert_eq!(remote.ws_url, "ws://127.0.0.1:54321");
        assert_eq!(remote.token, "cap-token");
        assert_eq!(remote.cwd, "/home/me/project");
    }

    #[test]
    fn partial_remote_target_is_ignored() {
        let pc = ProviderConfig {
            // Missing `token` and `cwd` → not a usable remote target.
            extra: json!({ "remote": { "ws_url": "ws://127.0.0.1:1" } }),
            ..Default::default()
        };
        assert!(LocalConfig::from_provider_config(&pc).remote.is_none());
    }

    #[test]
    fn parses_compaction_overrides() {
        let pc = ProviderConfig {
            extra: json!({
                "auto_compact_token_limit": 1234,
                "compact_request_token_limit": 1000,
                "compact_recent_user_token_budget": 99
            }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.compaction.auto_compact_token_limit, 1234);
        assert_eq!(cfg.compaction.compact_request_token_limit, 1000);
        assert_eq!(cfg.compaction.recent_user_token_budget, 99);
    }

    #[test]
    fn can_disable_compaction() {
        let pc = ProviderConfig {
            extra: json!({ "auto_compact": false }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert!(!cfg.compaction.enabled());
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
        assert_eq!(cfg.max_iterations, Some(8));
        assert_eq!(cfg.mode_for("bash"), PermissionMode::Deny);
        assert_eq!(cfg.mode_for("edit_file"), PermissionMode::Allow);
        assert_eq!(cfg.mode_for("write_file"), PermissionMode::Ask);
        assert!(
            cfg.clark.is_none(),
            "research:false disables it even with a key"
        );
    }
}
