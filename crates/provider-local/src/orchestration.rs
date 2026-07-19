use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_core::provider::{Provider, ProviderConfig};
use agent_orchestration::{
    HarnessError, HarnessKind, ProviderHarness, ProviderHarnessConfig, ReadOnlyEnforcement,
    WorkspaceGuard,
};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::exec::Executor;
use crate::LocalAgentProvider;

#[path = "orchestration_tool.rs"]
mod tool;

pub(crate) use tool::orchestration_tools;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DelegationMode {
    #[default]
    ExplicitRequestOnly,
    Proactive,
}

impl DelegationMode {
    fn parse(value: Option<&Value>) -> Self {
        match value.and_then(Value::as_str) {
            Some("proactive") => Self::Proactive,
            _ => Self::ExplicitRequestOnly,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OrchestrationConfig {
    pub enabled: bool,
    pub mode: DelegationMode,
    pub max_agents: usize,
    pub max_attempts: u32,
    pub token_budget: u64,
    pub minimum_context_tokens: u64,
    pub child_system_prompt_tokens: u64,
    pub max_projected_cost_ratio: f64,
    pub subagent_model: Option<String>,
    pub read_only_harness: String,
    pub root_model_rate: agent_orchestration::ModelRate,
    pub subagent_model_rate: Option<agent_orchestration::ModelRate>,
    pub acp_harnesses: Vec<AcpHarnessConfig>,
}

#[derive(Clone, Debug)]
pub(crate) struct AcpHarnessConfig {
    pub id: String,
    pub command: Vec<String>,
    pub model: String,
    pub enforcement: ReadOnlyEnforcement,
    pub model_rate: Option<agent_orchestration::ModelRate>,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: DelegationMode::ExplicitRequestOnly,
            max_agents: 3,
            max_attempts: 2,
            token_budget: 120_000,
            minimum_context_tokens: 40_000,
            child_system_prompt_tokens: 6_000,
            max_projected_cost_ratio: 1.25,
            subagent_model: None,
            read_only_harness: "local".to_string(),
            root_model_rate: agent_orchestration::ModelRate::default(),
            subagent_model_rate: None,
            acp_harnesses: Vec::new(),
        }
    }
}

impl OrchestrationConfig {
    pub(crate) fn from_extra(extra: &Value) -> Self {
        let Some(value) = extra.get("orchestration") else {
            return Self::default();
        };
        if let Some(enabled) = value.as_bool() {
            return Self {
                enabled,
                ..Self::default()
            };
        }
        let Some(object) = value.as_object() else {
            return Self::default();
        };
        let mut config = Self::default();
        config.enabled = object
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(config.enabled);
        config.mode = DelegationMode::parse(object.get("mode"));
        config.max_agents =
            integer(object, "max_agents", config.max_agents as u64).clamp(1, 4) as usize;
        config.max_attempts =
            integer(object, "max_attempts", config.max_attempts as u64).clamp(1, 3) as u32;
        config.token_budget = integer(object, "token_budget", config.token_budget).max(1);
        config.minimum_context_tokens = integer(
            object,
            "minimum_context_tokens",
            config.minimum_context_tokens,
        )
        .max(1);
        config.child_system_prompt_tokens = integer(
            object,
            "child_system_prompt_tokens",
            config.child_system_prompt_tokens,
        );
        config.max_projected_cost_ratio = object
            .get("max_projected_cost_ratio")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 1.0)
            .unwrap_or(config.max_projected_cost_ratio)
            .min(2.0);
        config.subagent_model = object
            .get("subagent_model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        config.root_model_rate = model_rate(object.get("root_model_rate")).unwrap_or_default();
        config.subagent_model_rate = model_rate(object.get("subagent_model_rate"));
        config.acp_harnesses = object
            .get("acp_harnesses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_acp_harness)
            .take(3)
            .collect();
        config.read_only_harness = object
            .get("read_only_harness")
            .and_then(Value::as_str)
            .filter(|id| {
                *id == "local" || config.acp_harnesses.iter().any(|harness| harness.id == *id)
            })
            .unwrap_or("local")
            .to_string();
        config
    }
}

#[derive(Clone)]
pub(crate) struct OrchestrationToolsConfig {
    pub policy: OrchestrationConfig,
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: std::collections::HashMap<String, String>,
    pub root_model: String,
    pub reasoning_effort: Option<String>,
}

impl OrchestrationToolsConfig {
    pub(crate) fn from_local(config: &crate::config::LocalConfig) -> Self {
        Self {
            policy: config.orchestration.clone(),
            base_url: config.base_url.clone(),
            api_key: config.api_key.clone(),
            headers: config.headers.clone(),
            root_model: config.model.clone(),
            reasoning_effort: config.reasoning_effort.clone(),
        }
    }
}

fn integer(object: &Map<String, Value>, key: &str, default: u64) -> u64 {
    object.get(key).and_then(Value::as_u64).unwrap_or(default)
}

fn model_rate(value: Option<&Value>) -> Option<agent_orchestration::ModelRate> {
    let object = value?.as_object()?;
    let input = object.get("input_per_million_usd")?.as_f64()?;
    let output = object.get("output_per_million_usd")?.as_f64()?;
    (input.is_finite() && output.is_finite() && input >= 0.0 && output >= 0.0).then_some(
        agent_orchestration::ModelRate {
            input_per_million_usd: input,
            output_per_million_usd: output,
        },
    )
}

fn parse_acp_harness(value: &Value) -> Option<AcpHarnessConfig> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?.trim().to_string();
    let model = object.get("model")?.as_str()?.trim().to_string();
    let command = object
        .get("command")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let enforcement = match object.get("enforcement")?.as_str()? {
        "os_sandbox" => ReadOnlyEnforcement::OsSandbox,
        _ => return None,
    };
    let model_rate = model_rate(object.get("model_rate"));
    (!id.is_empty() && !model.is_empty() && !command.is_empty()).then_some(AcpHarnessConfig {
        id,
        command,
        model,
        enforcement,
        model_rate,
    })
}

pub(crate) fn turn_policy_section(mode: DelegationMode) -> &'static str {
    match mode {
        DelegationMode::ExplicitRequestOnly => {
            "[runtime policy — bounded delegation]\n\
             Tool availability is not authorization. Do not delegate unless the current user request, applicable project instructions, or an active skill explicitly asks for subagents, delegation, or parallel agent work.\n\
             Even when authorized, stay single-agent unless there are at least two concrete, bounded, genuinely independent workstreams and parallelism would materially improve speed, context isolation, or correctness. Do not fan out for status, explanation, one small edit, sequential work, or merely to avoid doing local work.\n\
             While delegates run, continue useful root work that cannot conflict with them.\n\
             Read-only agents must return evidence before conclusions. Verify cited evidence yourself, then resolve every report.\n\
             Parallel writers require at least two exact, disjoint file leases. They work in disposable Git clones; the host replays hashed patches and verifies the integrated result before the primary checkout can change.\n\
             Do not recurse, widen permissions, or use coding delegates for external research."
        }
        DelegationMode::Proactive => {
            "[runtime policy — bounded delegation]\n\
             You may proactively delegate only when independent workstreams would materially improve speed, context isolation, or correctness. Keep ordinary, small, overlapping, and sequential work single-agent.\n\
             Delegate concrete, bounded work that can run independently alongside useful local work. Prefer read-heavy exploration, review, verification, and test analysis.\n\
             Read-only agents must return evidence before conclusions. Verify cited evidence yourself, then resolve every report.\n\
             Parallel writers require at least two exact, disjoint file leases. They work in disposable Git clones; the host replays hashed patches and verifies the integrated result before the primary checkout can change.\n\
             Do not recurse, widen permissions, or use coding delegates for external research."
        }
    }
}

/// Hashes the model-visible workspace before and after a delegated attempt.
/// The executor walk honors repository ignores and Clark's fixed build-cache
/// exclusions; source, tracked files, and ordinary untracked files are covered.
pub struct WorkspaceDigestGuard {
    root: PathBuf,
    executor: Arc<dyn Executor>,
}

impl WorkspaceDigestGuard {
    pub fn new(root: impl Into<PathBuf>, executor: Arc<dyn Executor>) -> Self {
        Self {
            root: root.into(),
            executor,
        }
    }
}

#[async_trait]
impl WorkspaceGuard for WorkspaceDigestGuard {
    async fn snapshot(&self) -> Result<String, HarnessError> {
        digest_workspace(self.executor.as_ref(), &self.root)
            .await
            .map_err(HarnessError::Failed)
    }
}

/// Build a nested Clark local-agent harness with fail-closed permissions.
///
/// Configuration is sanitized here rather than trusting caller-provided
/// permission maps: known file/shell mutators are denied, every other mutating
/// tool defaults to Clark's approval gate (which ProviderHarness always rejects),
/// remote/cloud/MCP/memory/browser paths are disabled, and recursion is off.
pub fn local_read_only_harness(
    mut config: ProviderHarnessConfig,
    workspace: Arc<dyn WorkspaceGuard>,
) -> Result<ProviderHarness, String> {
    if config.kind != HarnessKind::Local {
        return Err("local adapter requires harness kind=local".to_string());
    }
    if config.enforcement != ReadOnlyEnforcement::HostToolGate {
        return Err("local adapter requires Clark's host tool gate".to_string());
    }
    config.provider_config = read_only_provider_config(config.provider_config);
    ProviderHarness::new(
        config,
        Arc::new(|| Box::new(LocalAgentProvider::new()) as Box<dyn Provider>),
        workspace,
    )
}

fn read_only_provider_config(mut config: ProviderConfig) -> ProviderConfig {
    let mut extra = match config.extra {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    extra.insert(
        "permissions".to_string(),
        json!({
            "write_file": "deny",
            "edit_file": "deny",
            "apply_patch": "deny",
            "bash": "deny"
        }),
    );
    extra.insert("command_allowlist".to_string(), json!([]));
    extra.insert(
        "command_denylist".to_string(),
        json!(["git", "rm", "mv", "cp", "curl", "wget", "ssh"]),
    );
    extra.insert("mcp_servers".to_string(), json!([]));
    extra.insert("research".to_string(), Value::Bool(false));
    extra.insert("memories".to_string(), Value::Bool(false));
    extra.insert("project_knowledge".to_string(), Value::Bool(false));
    extra.insert("browser_enabled".to_string(), Value::Bool(false));
    extra.insert("orchestration".to_string(), Value::Bool(false));
    let max_iterations = extra
        .get("max_iterations")
        .and_then(Value::as_u64)
        .unwrap_or(32)
        .clamp(1, 48);
    extra.insert("max_iterations".to_string(), json!(max_iterations));
    config.extra = Value::Object(extra);
    config
}

async fn digest_workspace(executor: &dyn Executor, root: &Path) -> Result<String, String> {
    let mut entries = executor.walk(root).await?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let mut digest = Sha256::new();
    for entry in entries {
        let relative = entry.path.strip_prefix(root).unwrap_or(&entry.path);
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(entry.len.to_le_bytes());
        digest.update([0]);
        let contents = executor.read(&entry.path).await?;
        digest.update(Sha256::digest(contents));
        digest.update([0xff]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use agent_orchestration::WorkspaceGuard;
    use tempfile::tempdir;

    use crate::config::LocalConfig;
    use crate::exec::LocalExecutor;

    use super::*;

    #[tokio::test]
    async fn digest_changes_when_source_changes() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("file.rs"), "before").unwrap();
        let guard = WorkspaceDigestGuard::new(dir.path(), Arc::new(LocalExecutor));
        let before = guard.snapshot().await.unwrap();
        std::fs::write(dir.path().join("file.rs"), "after").unwrap();
        assert_ne!(before, guard.snapshot().await.unwrap());
    }

    #[test]
    fn nested_provider_config_is_fail_closed_and_cloud_free() {
        let sanitized = read_only_provider_config(ProviderConfig {
            extra: json!({
                "max_iterations": 500,
                "permissions": {"write_file": "allow"},
                "research": true,
                "browser_enabled": true,
                "memories": true
            }),
            ..Default::default()
        });
        let local = LocalConfig::from_provider_config(&sanitized);
        assert_eq!(local.max_iterations, 48);
        assert_eq!(
            local.mode_for("write_file"),
            crate::tools::PermissionMode::Deny
        );
        assert_eq!(local.mode_for("bash"), crate::tools::PermissionMode::Deny);
        assert!(local.clark.is_none());
        assert!(!local.browser_enabled);
        assert!(!local.memories_enabled);
        assert!(local.mcp_servers.is_empty());
        assert!(!local.orchestration.enabled);
    }

    #[test]
    fn orchestration_is_default_available_bounded_and_refuses_unproved_disposable_acp() {
        let defaults = OrchestrationConfig::from_extra(&json!({}));
        assert!(defaults.enabled);
        assert_eq!(defaults.mode, DelegationMode::ExplicitRequestOnly);

        let explicitly_disabled = OrchestrationConfig::from_extra(&json!({
            "orchestration": {"enabled": false}
        }));
        assert!(!explicitly_disabled.enabled);

        let config = OrchestrationConfig::from_extra(&json!({
            "orchestration": {
                "enabled": true,
                "mode": "proactive",
                "max_agents": 99,
                "max_attempts": 99,
                "token_budget": 75_000,
                "subagent_model": "cheap-model",
                "read_only_harness": "sandboxed",
                "acp_harnesses": [
                    {
                        "id": "unsafe",
                        "model": "codex",
                        "command": ["codex", "acp"],
                        "enforcement": "disposable_checkout"
                    },
                    {
                        "id": "sandboxed",
                        "model": "codex",
                        "command": ["codex", "acp"],
                        "enforcement": "os_sandbox",
                        "model_rate": {
                            "input_per_million_usd": 0.2,
                            "output_per_million_usd": 0.8
                        }
                    }
                ]
            }
        }));
        assert!(config.enabled);
        assert_eq!(config.mode, DelegationMode::Proactive);
        assert_eq!(config.max_agents, 4);
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.token_budget, 75_000);
        assert_eq!(config.subagent_model.as_deref(), Some("cheap-model"));
        assert_eq!(config.read_only_harness, "sandboxed");
        assert_eq!(config.acp_harnesses.len(), 1);
        assert_eq!(config.acp_harnesses[0].id, "sandboxed");
        assert_eq!(
            config.acp_harnesses[0].model_rate,
            Some(agent_orchestration::ModelRate {
                input_per_million_usd: 0.2,
                output_per_million_usd: 0.8,
            })
        );
    }

    #[test]
    fn explicit_mode_requires_a_real_delegation_trigger() {
        let prompt = turn_policy_section(DelegationMode::ExplicitRequestOnly);
        assert!(prompt.contains("Tool availability is not authorization"));
        assert!(prompt.contains("Do not delegate unless"));
        assert!(prompt.contains("at least two concrete, bounded, genuinely independent"));
        assert!(prompt.contains("Do not fan out for status"));
        assert!(prompt.contains("disposable Git clones"));
        assert!(prompt.contains("Do not recurse"));
    }

    #[test]
    fn proactive_mode_still_rejects_weak_parallelism() {
        let prompt = turn_policy_section(DelegationMode::Proactive);
        assert!(prompt.contains("proactively delegate"));
        assert!(prompt.contains("materially improve"));
        assert!(prompt.contains("small, overlapping, and sequential work single-agent"));
    }
}
