//! Connection + behavior config for the local agent, parsed from
//! [`agent_core::ProviderConfig`].
//!
//! The host supplies an OpenAI-compatible endpoint, model catalog, and optional
//! auxiliary-model policy through [`ProviderConfig`].

use std::collections::HashMap;
use std::path::PathBuf;

use agent_core::provider::{ModelCapability, ProviderConfig};
use serde::Deserialize;
use serde_json::Value;

use crate::compaction::CompactionConfig;
use crate::tools::PermissionMode;

/// Neutral local endpoint used when a host does not provide product policy.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";
pub const DEFAULT_MODEL: &str = "local-model";
/// Neutral auxiliary model used for research when a host does not override it.
pub const DEFAULT_RESEARCH_MODEL: &str = "research-model";
/// Neutral auxiliary model used for image understanding.
pub const DEFAULT_VISION_MODEL: &str = "vision-model";
/// Approximate transcript-token threshold where the local loop checkpoints old
/// context before the next model request.
pub const DEFAULT_AUTO_COMPACT_TOKEN_LIMIT: usize = 300_000;

/// Unknown models use the flat default plus the engine's overflow recovery.
fn model_context_window(model: &str) -> Option<usize> {
    let _ = model;
    None
}

/// Provider-safe output ceiling. Hosts currently leave this backend-owned.
pub(crate) fn model_max_output_tokens(_model: &str) -> Option<u32> {
    None
}

/// Image understanding is routed through the host-provided auxiliary model,
/// keeping image behavior independent of the coding tier.
pub(crate) fn model_supports_images(_model: &str) -> bool {
    false
}

/// Effective auto-compaction threshold for `model`: the flat default, lowered
/// (never raised — 300k on the default 400K window is deliberate headroom) to
/// 80% of the model's known context window when that is smaller. Applied
/// defensively: a model whose whole window is under the flat default must
/// compact before it overflows, not after.
pub(crate) fn default_auto_compact_limit(model: &str) -> usize {
    match model_context_window(model) {
        Some(window) => DEFAULT_AUTO_COMPACT_TOKEN_LIMIT.min(window.saturating_mul(4) / 5),
        None => DEFAULT_AUTO_COMPACT_TOKEN_LIMIT,
    }
}

/// Host-owned retry contract for a managed model alias. The generic transport
/// does not know provider error wording or which model is safe to fall back to.
#[derive(Clone, Debug, Deserialize)]
pub struct ModelFallbackPolicy {
    pub model: String,
    pub reason: String,
    pub error_type: String,
    pub error_param: String,
    pub error_message: String,
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
    /// Host-advertised selectable model catalog.
    pub models: Vec<ModelCapability>,
    pub model_fallback: Option<ModelFallbackPolicy>,
    pub memory_extraction_model: Option<String>,
    /// Product-owned execution policies for named built-in workflows such as
    /// `scout` and `security`. Missing entries inherit the conversation model.
    pub skill_model_overrides: HashMap<String, ModelPolicyConfig>,
    /// Bearer credential for the configured model endpoint.
    pub api_key: Option<String>,
    /// Extra HTTP headers, if any.
    pub headers: HashMap<String, String>,
    /// Sampling temperature, if pinned.
    pub temperature: Option<f32>,
    /// Host-owned ceiling for one model response, including provider-native
    /// reasoning tokens. `None` leaves the endpoint default unchanged.
    pub max_output_tokens: Option<u32>,
    /// Reasoning-effort override sent with each coding request ("low" …
    /// "xhigh"). `None` → the model's server-side default.
    pub reasoning_effort: Option<String>,
    /// Provider-native structured response contract for the whole session.
    /// This is set only by trusted hosts that own the output schema.
    pub response_format: Option<Value>,
    /// Provider routing preferences such as OpenRouter's
    /// `require_parameters`. Model prompts cannot set this value.
    pub provider_preferences: Option<Value>,
    /// Stable host-owned routing identity for provider-side prefix caching.
    /// It is honored only for strict toolless structured sessions.
    pub cache_session_id: Option<String>,
    /// Whether the session advertises or executes any tools. Structured
    /// scientist turns disable the registry entirely.
    pub tools_enabled: bool,
    /// Optional complete system prompt supplied by a trusted headless host.
    /// Product sessions leave this unset and use local agent's normal prompt.
    pub system_prompt_override: Option<String>,
    /// Host-owned Git attribution defaults. Project settings may override or
    /// disable either value without learning product policy from this crate.
    pub default_commit_attribution: String,
    pub default_pr_body_attribution: String,
    /// Optional hard cap on model<->tool iterations per turn. Production leaves
    /// this unbounded; tests and evals may set an explicit cap.
    pub max_iterations: Option<u32>,
    /// Hidden A/B switch used by the paid planning benchmark. Production and
    /// normal tests always use the decision-complete profile.
    pub(crate) planning_prompt_profile: crate::planning::PlanningPromptProfile,
    /// Use Clark Code's hidden `<proposed_plan>` framing in Plan Mode instead
    /// of exposing the legacy JSON plan tools. The legacy switch exists for
    /// protocol-compatibility fixtures and old clients.
    pub(crate) hidden_plan_protocol: bool,
    /// Hidden planning-eval seam: deferred tool schemas that should be visible
    /// on the first model request. Production omits this and starts empty.
    pub(crate) planning_eval_preactivated_tools: Vec<String>,
    /// Expose registered read-only memory, organization, and Scout schemas on
    /// the first Plan Mode call so the model can explore unknown unknowns
    /// without guessing a `tool_search` query. Evals can disable this to keep
    /// legacy context-delivery treatments isolated.
    pub(crate) planning_research_autoactivate: bool,
    /// Keep the approved typed plan at the provider request's recency edge and
    /// reopen execution when the model stops with unresolved plan steps.
    ///
    /// This is on by default. The hidden switch exists so planning evaluations
    /// can compare the same runtime with and without plan enforcement.
    pub(crate) plan_execution_reminders: bool,
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
    /// Optional brokered research config, when research is enabled.
    pub research: Option<AuxiliaryModelConfig>,
    /// Vision-fallback config for coding models without native image support.
    /// Independent of the `research` toggle — gated only on a key being
    /// present.
    pub vision: Option<AuxiliaryModelConfig>,
    /// Product-owned model ids that must not receive image-generation tools.
    pub image_generation_excluded_models: Vec<String>,
    /// Project root, when set at connect time. A session's `cwd` option wins.
    pub cwd: Option<String>,
    /// Trusted host attestation that this provider process itself is running
    /// inside a durable remote worker. It never contains a host, credential,
    /// transport, or alternate executor target.
    pub remote_worker: bool,
    /// Local OS containment policy. `Auto` and `Required` fail session creation
    /// when containment cannot be established; `Disabled` is an explicit
    /// host-capability mode.
    pub sandbox_mode: LocalSandboxMode,
    /// Host-approved directories that model-facing file reads may access in
    /// addition to the project and Clark Code document workspace. They never widen
    /// write access and are ignored for remote sessions.
    pub sandbox_read_roots: Vec<PathBuf>,
    /// Whether durable memory is enabled — exposes the `memory` tool and injects
    /// the project + global memory into the system prompt. On by default; the
    /// user turns it off from the profile menu (`extra.memories = false`).
    pub memories_enabled: bool,
    /// Stable authenticated account binding used only to partition local
    /// global-memory files. Missing scope disables that local global scope.
    pub memory_scope: Option<String>,
    /// Whether this opted-in session may retrieve private repository evidence
    /// previously synced to the user's Clark Code account.
    pub project_knowledge_enabled: bool,
    /// Checkpoint compaction for the model-visible transcript.
    pub compaction: CompactionConfig,
    /// Experimental: register the host-configured browser tool (lazily
    /// downloaded on first use). Off by default — the user opts in from
    /// Settings (`extra.browser_enabled = true`).
    pub browser_enabled: bool,
    pub browser_binary: Option<crate::browser_binary::BrowserBinaryConfig>,
    /// Opt-in control of ordinary apps on the local Mac. The tool layer still
    /// enforces per-app permission scopes and macOS TCC independently.
    pub computer_use_enabled: bool,
    /// Native in production; the deterministic simulator is accepted only by
    /// debug builds and test harnesses.
    pub computer_use_backend: ComputerUseBackend,
    /// Bounded local multi-agent orchestration. Available by default, while its
    /// model-facing policy remains explicit-request-only.
    pub(crate) orchestration: crate::orchestration::OrchestrationConfig,
    pub(crate) scout_capsules: Option<crate::orchestration::ScoutCapsulePolicyConfig>,
    pub(crate) scout_cartography: Option<crate::orchestration::ScoutCartographyHostConfig>,
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
            Some("disabled" | "danger-full-access") => Self::Disabled,
            _ => Self::Auto,
        }
    }
}

/// Config for calling a host-advertised model tier (for example
/// `research-model`) as an auxiliary, non-coding call over the same model API and
/// key as the coding model — used by optional brokered research, the `web_fetch`
/// long-page condenser, and the image-description vision fallback. Clark Code runs
/// web search / planning / browsing / vision server-side and returns the
/// final answer with no client tools involved.
#[derive(Clone, Debug)]
pub struct AuxiliaryModelConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    /// Agentic model tier this call uses (e.g. `research-model`).
    pub model: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ModelPolicyConfig {
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
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
    /// Recognized `extra` keys: `model`, `temperature`, `max_output_tokens`, `max_iterations`,
    /// `permissions` (map of tool→`allow|ask|deny`), `research` (bool, default
    /// true), `research_model`, `auto_compact` (bool),
    /// `auto_compact_token_limit`, `compact_request_token_limit`,
    /// `compact_recent_user_token_budget`, and `base_url` (tests only). The key
    /// rides on `auth_token`.
    pub fn from_provider_config(config: &ProviderConfig) -> Self {
        let extra = &config.extra;

        let base_url = str_field(extra, "base_url").unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let model = str_field(extra, "model").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let models = extra
            .get("models")
            .and_then(|value| serde_json::from_value::<Vec<ModelCapability>>(value.clone()).ok())
            .filter(|models| !models.is_empty())
            .unwrap_or_else(|| {
                vec![ModelCapability {
                    id: DEFAULT_MODEL.into(),
                    label: "Local model".into(),
                    description: "OpenAI-compatible local coding model".into(),
                    reasoning_effort: None,
                }]
            });
        let skill_model_overrides = extra
            .get("skill_model_overrides")
            .and_then(|value| {
                serde_json::from_value::<HashMap<String, ModelPolicyConfig>>(value.clone()).ok()
            })
            .unwrap_or_default();
        let model_fallback = extra
            .get("model_fallback")
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        let memory_extraction_model = str_field(extra, "memory_extraction_model");
        let api_key = config
            .auth_token
            .clone()
            .or_else(|| str_field(extra, "api_key"));

        let temperature = extra
            .get("temperature")
            .and_then(Value::as_f64)
            .map(|t| t as f32);
        let max_output_tokens = extra
            .get("max_output_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0);
        let reasoning_effort = str_field(extra, "reasoning_effort");
        let response_format = extra
            .get("response_format")
            .filter(|value| value.is_object())
            .cloned();
        let provider_preferences = extra
            .get("provider")
            .filter(|value| value.is_object())
            .cloned();
        let cache_session_id = str_field(extra, "cache_session_id").filter(|value| {
            value.len() <= 128
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                })
        });
        let tools_enabled = extra
            .get("tools_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let system_prompt_override =
            str_field(extra, "system_prompt_override").filter(|prompt| prompt.len() <= 64 * 1024);
        let default_commit_attribution = str_field(extra, "default_commit_attribution")
            .unwrap_or_else(|| crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION.to_string());
        let default_pr_body_attribution = str_field(extra, "default_pr_body_attribution")
            .unwrap_or_else(|| crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION.to_string());
        let max_iterations = extra
            .get("max_iterations")
            .and_then(Value::as_u64)
            .map(|n| n as u32)
            .filter(|n| *n > 0);
        let planning_prompt_profile = crate::planning::PlanningPromptProfile::from_extra(
            extra.get("planning_prompt_profile").and_then(Value::as_str),
        );
        let hidden_plan_protocol = extra
            .get("hidden_plan_protocol")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let planning_eval_preactivated_tools = str_vec(extra, "planning_eval_preactivated_tools");
        let planning_research_autoactivate = extra
            .get("planning_research_autoactivate")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let plan_execution_reminders = extra
            .get("plan_execution_reminders")
            .and_then(Value::as_bool)
            .unwrap_or(true);

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
        let research = (research_enabled && api_key.is_some()).then(|| AuxiliaryModelConfig {
            base_url: base_url.clone(),
            api_key: api_key.clone(),
            model: str_field(extra, "research_model")
                .unwrap_or_else(|| DEFAULT_RESEARCH_MODEL.to_string()),
        });

        // Vision fallback is core functionality for models without native
        // image support, not the opt-out-able research feature — gated only on
        // a key.
        let vision = api_key.is_some().then(|| AuxiliaryModelConfig {
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
        let memory_scope = str_field(extra, "memory_scope").filter(|scope| scope.len() <= 512);
        let project_knowledge_enabled = extra
            .get("project_knowledge")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Off by default — the user opts in from Settings.
        let browser_enabled = extra
            .get("browser_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let browser_binary = extra
            .get("browser_binary")
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        let computer_use_enabled = extra
            .get("computer_use_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let computer_use_backend = ComputerUseBackend::from_extra(extra);
        let orchestration = crate::orchestration::OrchestrationConfig::from_extra(extra);
        let scout_capsules = crate::orchestration::ScoutCapsulePolicyConfig::from_extra(extra);
        let scout_cartography = crate::orchestration::ScoutCartographyHostConfig::from_extra(extra);
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

        let sandbox_mode = LocalSandboxMode::from_extra(extra);
        let sandbox_read_roots = str_vec(extra, "sandbox_read_roots")
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let remote_worker = extra
            .get("worker_execution_residency")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "remote_worker");
        Self {
            base_url,
            model,
            models,
            model_fallback,
            memory_extraction_model,
            skill_model_overrides,
            api_key,
            headers: config.headers.clone(),
            temperature,
            max_output_tokens,
            reasoning_effort,
            response_format,
            provider_preferences,
            cache_session_id,
            tools_enabled,
            system_prompt_override,
            default_commit_attribution,
            default_pr_body_attribution,
            max_iterations,
            planning_prompt_profile,
            hidden_plan_protocol,
            planning_eval_preactivated_tools,
            planning_research_autoactivate,
            plan_execution_reminders,
            permissions,
            command_allowlist: str_vec(extra, "command_allowlist"),
            command_denylist: str_vec(extra, "command_denylist"),
            mcp_servers: extra
                .get("mcp_servers")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            research,
            vision,
            image_generation_excluded_models: str_vec(extra, "image_generation_excluded_models"),
            cwd,
            remote_worker,
            sandbox_mode,
            sandbox_read_roots,
            memories_enabled,
            memory_scope,
            project_knowledge_enabled,
            compaction,
            browser_enabled,
            browser_binary,
            computer_use_enabled,
            computer_use_backend,
            orchestration,
            scout_capsules,
            scout_cartography,
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
    fn defaults_to_neutral_local_endpoint_and_no_research_without_key() {
        let cfg = LocalConfig::from_provider_config(&ProviderConfig::default());
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.model, DEFAULT_MODEL);
        assert_eq!(cfg.max_iterations, None);
        assert_eq!(cfg.max_output_tokens, None);
        assert!(cfg.response_format.is_none());
        assert!(cfg.provider_preferences.is_none());
        assert!(cfg.tools_enabled);
        assert!(cfg.system_prompt_override.is_none());
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
        assert!(cfg.research.is_none());
        // No key → vision fallback can't run either.
        assert!(cfg.vision.is_none());
    }

    #[test]
    fn unknown_host_models_use_conservative_generic_defaults() {
        assert!(!model_supports_images(DEFAULT_MODEL));
        assert_eq!(model_context_window(DEFAULT_MODEL), None);
    }

    #[test]
    fn host_model_context_windows_do_not_change_the_neutral_default() {
        assert_eq!(model_context_window(DEFAULT_MODEL), None);
        assert_eq!(model_context_window("vendor/host-managed-model"), None,);
        assert_eq!(
            default_auto_compact_limit(DEFAULT_MODEL),
            DEFAULT_AUTO_COMPACT_TOKEN_LIMIT
        );
    }

    #[test]
    fn default_auto_sandbox_enables_auxiliary_research_with_a_key() {
        let pc = ProviderConfig {
            auth_token: Some("product_test_token".into()),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.sandbox_mode, LocalSandboxMode::Auto);
        assert_eq!(cfg.api_key.as_deref(), Some("product_test_token"));
        let research = cfg
            .research
            .expect("research enabled when a key is present");
        assert_eq!(research.base_url, DEFAULT_BASE_URL);
        assert_eq!(research.api_key.as_deref(), Some("product_test_token"));
        assert_eq!(research.model, DEFAULT_RESEARCH_MODEL);
    }

    #[test]
    fn danger_full_access_alias_disables_host_sandbox_for_unattended_workers() {
        let pc = ProviderConfig {
            extra: serde_json::json!({"sandbox_mode": "danger-full-access"}),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.sandbox_mode, LocalSandboxMode::Disabled);
    }

    #[test]
    fn host_approved_sandbox_read_roots_do_not_widen_tool_permissions() {
        let pc = ProviderConfig {
            extra: serde_json::json!({
                "sandbox_read_roots": ["/evidence/one", "/evidence/two"],
                "permissions": {"bash": "deny"}
            }),
            ..ProviderConfig::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(
            cfg.sandbox_read_roots,
            vec![
                PathBuf::from("/evidence/one"),
                PathBuf::from("/evidence/two")
            ]
        );
        assert_eq!(cfg.mode_for("bash"), PermissionMode::Deny);
    }

    #[test]
    fn a_key_enables_the_vision_fallback_through_the_same_api() {
        let pc = ProviderConfig {
            auth_token: Some("product_test_token".into()),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        let vision = cfg
            .vision
            .expect("vision fallback enabled when a key is present");
        assert_eq!(vision.base_url, DEFAULT_BASE_URL);
        assert_eq!(vision.api_key.as_deref(), Some("product_test_token"));
        assert_eq!(vision.model, DEFAULT_VISION_MODEL);
        assert_eq!(vision.model, "vision-model");
    }

    #[test]
    fn vision_stays_enabled_when_research_is_disabled() {
        let pc = ProviderConfig {
            auth_token: Some("product_test_token".into()),
            extra: json!({ "research": false }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert!(
            cfg.research.is_none(),
            "research:false disables the research config"
        );
        assert!(
            cfg.vision.is_some(),
            "vision fallback is core functionality, not gated by the research toggle"
        );
    }

    #[test]
    fn parses_account_memory_scope_and_rejects_oversized_values() {
        let scoped = ProviderConfig {
            extra: json!({ "memory_scope": "id:account-one" }),
            ..Default::default()
        };
        assert_eq!(
            LocalConfig::from_provider_config(&scoped)
                .memory_scope
                .as_deref(),
            Some("id:account-one")
        );

        let oversized = ProviderConfig {
            extra: json!({ "memory_scope": "x".repeat(513) }),
            ..Default::default()
        };
        assert!(LocalConfig::from_provider_config(&oversized)
            .memory_scope
            .is_none());
    }

    #[test]
    fn scout_cartography_binding_is_exact_and_host_owned() {
        let organization_id = uuid::Uuid::new_v4();
        let workspace_id = uuid::Uuid::new_v4();
        let pc = ProviderConfig {
            auth_token: Some("product_test_token".into()),
            extra: json!({
                "scout_cartography": {
                    "organization_id": organization_id,
                    "workspace_id": workspace_id,
                    "identity_root": "/host-private/agent/scout",
                    "platform": "linux",
                    "architecture": "x86_64",
                    "route_prefix": "/v1/cartography"
                }
            }),
            ..Default::default()
        };
        let binding = LocalConfig::from_provider_config(&pc)
            .scout_cartography
            .expect("complete host binding");
        assert_eq!(binding.organization_id, organization_id);
        assert_eq!(binding.workspace_id, workspace_id);
        assert_eq!(
            binding.identity_root,
            std::path::PathBuf::from("/host-private/agent/scout")
        );

        let relative = ProviderConfig {
            extra: json!({
                "scout_cartography": {
                    "organization_id": organization_id,
                    "workspace_id": workspace_id,
                    "identity_root": ".agent/scout",
                    "platform": "linux",
                    "architecture": "x86_64",
                    "route_prefix": "/v1/cartography"
                }
            }),
            ..Default::default()
        };
        assert!(LocalConfig::from_provider_config(&relative)
            .scout_cartography
            .is_none());
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
            auth_token: Some("product_test_token".into()),
            extra: json!({
                "base_url": "http://localhost:1234/v1",
                "model": "research-model",
                "temperature": 0.2,
                "max_output_tokens": 16384,
                "max_iterations": 8,
                "permissions": { "bash": "deny", "edit_file": "allow" },
                "research": false
            }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.base_url, "http://localhost:1234/v1");
        assert_eq!(cfg.model, "research-model");
        assert_eq!(cfg.temperature, Some(0.2));
        assert_eq!(cfg.max_output_tokens, Some(16_384));
        assert_eq!(cfg.max_iterations, Some(8));
        assert_eq!(cfg.mode_for("bash"), PermissionMode::Deny);
        assert_eq!(cfg.mode_for("edit_file"), PermissionMode::Allow);
        assert_eq!(cfg.mode_for("write_file"), PermissionMode::Ask);
        assert!(
            cfg.research.is_none(),
            "research:false disables it even with a key"
        );
    }

    #[test]
    fn planning_eval_preactivation_is_empty_by_default_and_parses_exact_names() {
        let defaults = LocalConfig::from_provider_config(&ProviderConfig::default());
        assert!(defaults.planning_eval_preactivated_tools.is_empty());
        assert!(defaults.planning_research_autoactivate);
        let pc = ProviderConfig {
            extra: json!({
                "planning_research_autoactivate": false,
                "planning_eval_preactivated_tools": [
                    "memory",
                    "organization_knowledge",
                    "scout_enterprise_query"
                ]
            }),
            ..Default::default()
        };
        assert_eq!(
            LocalConfig::from_provider_config(&pc).planning_eval_preactivated_tools,
            ["memory", "organization_knowledge", "scout_enterprise_query"]
        );
        assert!(!LocalConfig::from_provider_config(&pc).planning_research_autoactivate);
    }

    #[test]
    fn plan_execution_reminders_default_on_and_can_be_disabled_for_control_runs() {
        assert!(
            LocalConfig::from_provider_config(&ProviderConfig::default()).plan_execution_reminders
        );
        let control = ProviderConfig {
            extra: json!({"plan_execution_reminders": false}),
            ..Default::default()
        };
        assert!(!LocalConfig::from_provider_config(&control).plan_execution_reminders);
    }

    #[test]
    fn parses_host_owned_structured_output_and_toolless_policy() {
        let schema = json!({
            "type": "json_schema",
            "json_schema": {
                "name": "hypothesis",
                "strict": true,
                "schema": {"type": "object"}
            }
        });
        let provider = json!({"require_parameters": true});
        let pc = ProviderConfig {
            extra: json!({
                "response_format": schema,
                "provider": provider,
                "tools_enabled": false,
                "cache_session_id": "example-specialist-cache-1",
                "system_prompt_override": "You are a bounded specialist."
            }),
            ..Default::default()
        };
        let cfg = LocalConfig::from_provider_config(&pc);
        assert_eq!(cfg.response_format, Some(schema));
        assert_eq!(cfg.provider_preferences, Some(provider));
        assert_eq!(
            cfg.cache_session_id.as_deref(),
            Some("example-specialist-cache-1")
        );
        assert!(!cfg.tools_enabled);
        assert_eq!(
            cfg.system_prompt_override.as_deref(),
            Some("You are a bounded specialist.")
        );
    }
}
