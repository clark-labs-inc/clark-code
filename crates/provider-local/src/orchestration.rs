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

#[derive(Clone, Debug)]
pub(crate) struct OrchestrationConfig {
    pub max_agents: usize,
    pub token_budget: Option<u64>,
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
            max_agents: 3,
            token_budget: None,
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
        let Some(object) = value.as_object() else {
            return Self::default();
        };
        let mut config = Self::default();
        config.max_agents =
            integer(object, "max_agents", config.max_agents as u64).clamp(1, 4) as usize;
        config.token_budget = object
            .get("token_budget")
            .and_then(Value::as_u64)
            .filter(|budget| *budget > 0);
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
    pub scout_capsules: Option<ScoutCapsulePolicyConfig>,
    pub scout_cartography: Option<ScoutCartographyHostConfig>,
}

#[derive(Clone, Debug)]
pub(crate) struct ScoutCartographyHostConfig {
    pub organization_id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub identity_root: PathBuf,
    pub platform: String,
    pub architecture: String,
    pub route_prefix: String,
    pub human_run_request_id: Option<String>,
}

impl ScoutCartographyHostConfig {
    pub(crate) fn from_extra(extra: &Value) -> Option<Self> {
        let value = extra.get("scout_cartography")?.as_object()?;
        let organization_id = value
            .get("organization_id")?
            .as_str()?
            .parse::<uuid::Uuid>()
            .ok()
            .filter(|value| !value.is_nil())?;
        let workspace_id = value
            .get("workspace_id")?
            .as_str()?
            .parse::<uuid::Uuid>()
            .ok()
            .filter(|value| !value.is_nil())?;
        let identity_root = PathBuf::from(value.get("identity_root")?.as_str()?);
        if !identity_root.is_absolute() {
            return None;
        }
        let platform = portable_namespace(value.get("platform")?.as_str()?)?;
        let architecture = portable_namespace(value.get("architecture")?.as_str()?)?;
        let route_prefix = value.get("route_prefix")?.as_str()?.trim_end_matches('/');
        if !route_prefix.starts_with('/')
            || route_prefix.len() < 2
            || route_prefix.contains('?')
            || route_prefix.contains('#')
            || route_prefix.contains("..")
        {
            return None;
        }
        let human_run_request_id = value
            .get("human_run_request_id")
            .and_then(Value::as_str)
            .filter(|request_id| {
                request_id.strip_prefix("scout-run:").is_some_and(|digest| {
                    digest.len() == 64
                        && digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                })
            })
            .map(str::to_owned);
        Some(Self {
            organization_id,
            workspace_id,
            identity_root,
            platform,
            architecture,
            route_prefix: route_prefix.to_string(),
            human_run_request_id,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScoutCapsulePolicyConfig {
    pub authorized_tenant_id: String,
    pub trusted_admin_key_sha256: String,
    pub minimum_registry_generation: u64,
}

impl ScoutCapsulePolicyConfig {
    pub(crate) fn from_extra(extra: &Value) -> Option<Self> {
        let value = extra.get("scout_capsules")?.as_object()?;
        let authorized_tenant_id = value
            .get("authorized_tenant_id")?
            .as_str()?
            .trim()
            .to_owned();
        let trusted_admin_key_sha256 = value
            .get("trusted_admin_key_sha256")?
            .as_str()?
            .trim()
            .to_owned();
        let minimum_registry_generation = value.get("minimum_registry_generation")?.as_u64()?;
        if authorized_tenant_id.is_empty()
            || authorized_tenant_id.len() > 256
            || authorized_tenant_id.trim() != authorized_tenant_id
            || authorized_tenant_id.chars().any(char::is_control)
            || minimum_registry_generation == 0
            || trusted_admin_key_sha256.len() != 64
            || !trusted_admin_key_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return None;
        }
        Some(Self {
            authorized_tenant_id,
            trusted_admin_key_sha256,
            minimum_registry_generation,
        })
    }
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
            scout_capsules: config.scout_capsules.clone(),
            scout_cartography: config.scout_cartography.clone(),
        }
    }
}

fn portable_namespace(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 128
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        }))
    .then(|| value.to_owned())
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

pub(crate) fn turn_policy_section() -> &'static str {
    "[runtime policy — autonomous delegation]\n\
     Proactively delegate when independent workstreams would materially improve speed, context isolation, or correctness. Keep ordinary, small, overlapping, and sequential work single-agent.\n\
     Delegate concrete work that can run independently alongside useful local work. Prefer read-heavy exploration, review, verification, and test analysis.\n\
     Read-only agents must return evidence before conclusions. Verify cited evidence yourself, then resolve every report.\n\
     Parallel writers require exact, disjoint file leases. They work in disposable Git clones; the host replays hashed patches and verifies the integrated result before the primary checkout can change.\n\
     Do not recurse, widen permissions, or use coding delegates for external research."
}

/// Hashes the model-visible workspace before and after a delegated attempt.
/// The executor walk honors repository ignores and Clark Code's fixed build-cache
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

/// Build a nested Clark Code local-agent harness with fail-closed permissions.
///
/// Configuration is sanitized here rather than trusting caller-provided
/// permission maps: known file/shell mutators are denied, every other mutating
/// tool defaults to Clark Code's approval gate (which ProviderHarness always rejects),
/// remote/cloud/MCP/memory/browser paths are disabled, and recursion is off.
pub fn local_read_only_harness(
    mut config: ProviderHarnessConfig,
    workspace: Arc<dyn WorkspaceGuard>,
) -> Result<ProviderHarness, String> {
    if config.kind != HarnessKind::Local {
        return Err("local adapter requires harness kind=local".to_string());
    }
    if config.enforcement != ReadOnlyEnforcement::HostToolGate {
        return Err("local adapter requires Clark Code's host tool gate".to_string());
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
    extra.insert("memories".to_string(), Value::Bool(false));
    extra.insert("project_knowledge".to_string(), Value::Bool(false));
    extra.insert("browser_enabled".to_string(), Value::Bool(false));
    extra.insert("orchestration".to_string(), Value::Bool(false));
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
                "permissions": {"write_file": "allow"},
                "browser_enabled": true,
                "memories": true
            }),
            ..Default::default()
        });
        let local = LocalConfig::from_provider_config(&sanitized);
        assert_eq!(
            local.mode_for("write_file"),
            crate::tools::PermissionMode::Deny
        );
        assert_eq!(local.mode_for("bash"), crate::tools::PermissionMode::Deny);
        assert!(!local.browser_enabled);
        assert!(!local.memories_enabled);
        assert!(local.mcp_servers.is_empty());
    }

    #[test]
    fn orchestration_is_always_available_and_refuses_unproved_disposable_acp() {
        let defaults = OrchestrationConfig::from_extra(&json!({}));
        assert_eq!(defaults.max_agents, 3);
        assert_eq!(defaults.token_budget, None);

        let config = OrchestrationConfig::from_extra(&json!({
            "orchestration": {
                "max_agents": 99,
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
        assert_eq!(config.max_agents, 4);
        assert_eq!(config.token_budget, Some(75_000));
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
    fn autonomous_policy_rejects_weak_parallelism() {
        let prompt = turn_policy_section();
        assert!(prompt.contains("Proactively delegate"));
        assert!(prompt.contains("materially improve"));
        assert!(prompt.contains("small, overlapping, and sequential work single-agent"));
    }
}
