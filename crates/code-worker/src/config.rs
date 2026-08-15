use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use agent_core::ProviderConfig;
use code_host::ProjectRegistration;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub schema_version: u32,
    #[serde(default = "default_worker_name")]
    pub worker_name: String,
    pub projects: Vec<ProjectRegistration>,
    pub trajectory_root: PathBuf,
    #[serde(default)]
    pub provider: ProviderProfile,
    /// Built-in plugins are opt-in by id. The registry remains compile-time
    /// and typed; a config file cannot load arbitrary code into the worker.
    #[serde(default = "default_plugins")]
    pub enabled_plugins: BTreeSet<String>,
    #[serde(default = "default_request_limit")]
    pub max_request_bytes: usize,
    #[serde(default = "default_response_limit")]
    pub max_response_bytes: usize,
    #[serde(default = "default_concurrent_requests")]
    pub max_concurrent_requests: usize,
    /// Where this process owns execution. A remote launcher sets this to
    /// `remote_worker`, so a ping receipt proves model/tool residency rather
    /// than merely proving that an SSH control channel exists.
    #[serde(default)]
    pub execution_residency: ExecutionResidency,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionResidency {
    #[default]
    Local,
    RemoteWorker,
}

impl WorkerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != 1 {
            return Err(ConfigError::UnsupportedSchema(self.schema_version));
        }
        let unsafe_root = |path: &PathBuf| {
            path == &PathBuf::from("/")
                || path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
        };
        if !portable_name(&self.worker_name)
            || self.projects.is_empty()
            || !self.trajectory_root.is_absolute()
            || self
                .projects
                .iter()
                .any(|project| !project.root.is_absolute())
            || unsafe_root(&self.trajectory_root)
            || self
                .projects
                .iter()
                .any(|project| unsafe_root(&project.root))
        {
            return Err(ConfigError::InvalidIdentity);
        }
        if self.enabled_plugins.is_empty() {
            return Err(ConfigError::NoPlugins);
        }
        if self
            .enabled_plugins
            .iter()
            .any(|plugin| !portable_name(plugin))
        {
            return Err(ConfigError::InvalidIdentity);
        }
        if self.max_request_bytes == 0
            || self.max_request_bytes > 8 * 1024 * 1024
            || self.max_response_bytes == 0
            || self.max_response_bytes > 8 * 1024 * 1024
            || self.max_concurrent_requests == 0
            || self.max_concurrent_requests > 256
        {
            return Err(ConfigError::InvalidLimits);
        }
        self.provider.validate()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    /// The credential name is configuration, not a credential value. The
    /// worker reads it once from its environment and never returns it.
    #[serde(default = "default_key_env")]
    pub api_key_env: String,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub allowed_tools: BTreeSet<String>,
    /// Bash remains permission-gated even when the tool is enabled. Only an
    /// exact configured prefix may be auto-approved by the worker.
    #[serde(default)]
    pub allowed_command_prefixes: Vec<String>,
}

impl ProviderProfile {
    fn validate(&self) -> Result<(), ConfigError> {
        if !self.base_url.starts_with("https://") && !self.base_url.starts_with("http://127.0.0.1")
        {
            return Err(ConfigError::InvalidProvider(
                "provider base_url must use HTTPS (or loopback HTTP for tests)".into(),
            ));
        }
        if self.model.trim().is_empty() || self.model.len() > 256 {
            return Err(ConfigError::InvalidProvider(
                "provider model is invalid".into(),
            ));
        }
        if !portable_env_name(&self.api_key_env) {
            return Err(ConfigError::InvalidProvider(
                "api_key_env must be a portable environment variable name".into(),
            ));
        }
        if self.allowed_tools.iter().any(|tool| !portable_name(tool)) {
            return Err(ConfigError::InvalidProvider(
                "allowed tool names must be portable identifiers".into(),
            ));
        }
        if self.allowed_command_prefixes.iter().any(|prefix| {
            prefix.trim().is_empty() || prefix.len() > 256 || prefix.contains(['\n', '\r'])
        }) {
            return Err(ConfigError::InvalidProvider(
                "allowed command prefixes must be bounded and non-empty".into(),
            ));
        }
        Ok(())
    }

    pub fn provider_config(&self, execution_residency: ExecutionResidency) -> ProviderConfig {
        let auth_token = std::env::var(&self.api_key_env)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let mut permissions = serde_json::Map::new();
        for tool in ["bash", "write_file", "edit_file"] {
            let mode = if self.allowed_tools.contains(tool) {
                "ask"
            } else {
                "deny"
            };
            permissions.insert(tool.into(), serde_json::Value::String(mode.into()));
        }
        let extra = serde_json::json!({
            "base_url": self.base_url,
            "model": self.model,
            "reasoning_effort": self.reasoning_effort,
            "permissions": permissions,
            "command_allowlist": self.allowed_command_prefixes,
            "memories": false,
            "orchestration": false,
            "browser_enabled": false,
            "computer_use_enabled": false,
            "project_knowledge": false,
            "worker_execution_residency": execution_residency,
            // The durable worker is already a trusted native host boundary.
            // The canonical project-root sandbox still contains file tools,
            // while shell execution remains permission-gated with the exact
            // command allowlist below. Do not require optional host packages
            // such as Bubblewrap merely to open a remote session.
            "sandbox_mode": "disabled",
            "system_prompt_override": "You are running as a headless coding-agent worker. Work only inside the registered project root. Use the available tools deliberately, keep actions bounded, and finish with a concise machine-readable summary."
        });
        ProviderConfig {
            endpoint: None,
            command: None,
            cwd: None,
            headers: HashMap::new(),
            auth_token,
            extra,
        }
    }

    pub fn command_allowed(&self, command: &str) -> bool {
        self.allowed_command_prefixes
            .iter()
            .any(|prefix| prefix == "*" || command.trim_start().starts_with(prefix.trim()))
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            worker_name: default_worker_name(),
            projects: Vec::new(),
            trajectory_root: PathBuf::new(),
            provider: ProviderProfile::default(),
            enabled_plugins: default_plugins(),
            max_request_bytes: default_request_limit(),
            max_response_bytes: default_response_limit(),
            max_concurrent_requests: default_concurrent_requests(),
            execution_residency: ExecutionResidency::Local,
        }
    }
}

impl Default for ProviderProfile {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            model: default_model(),
            api_key_env: default_key_env(),
            reasoning_effort: None,
            allowed_tools: BTreeSet::new(),
            allowed_command_prefixes: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unsupported worker schema version {0}")]
    UnsupportedSchema(u32),
    #[error("worker name, project list, project roots, and absolute trajectory_root are required")]
    InvalidIdentity,
    #[error("at least one plugin must be enabled")]
    NoPlugins,
    #[error("unknown worker plugin: {0}")]
    UnknownPlugin(String),
    #[error("worker request/response limits are invalid")]
    InvalidLimits,
    #[error("invalid provider configuration: {0}")]
    InvalidProvider(String),
}

fn default_worker_name() -> String {
    "agent-code-worker".into()
}

fn default_plugins() -> BTreeSet<String> {
    BTreeSet::from(["coding".into(), "project".into()])
}

fn default_request_limit() -> usize {
    1024 * 1024
}

fn default_response_limit() -> usize {
    8 * 1024 * 1024
}

fn default_concurrent_requests() -> usize {
    32
}

fn default_base_url() -> String {
    "http://127.0.0.1:11434/v1".into()
}

fn default_model() -> String {
    "local-model".into()
}

fn default_key_env() -> String {
    "DESKTOP_MODEL_API_KEY".into()
}

fn portable_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn portable_env_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && (byte.is_ascii_alphabetic() || byte == b'_'))
                || (index > 0 && (byte.is_ascii_alphanumeric() || byte == b'_'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_turns_do_not_inject_hidden_lifetime_limits() {
        let profile = ProviderProfile::default();

        let extra = profile
            .provider_config(ExecutionResidency::RemoteWorker)
            .extra;
        assert!(extra.get("turn_timeout_seconds").is_none());
    }

    #[test]
    fn credentials_stay_out_of_serialized_config_and_routes_are_bounded() {
        let profile = ProviderProfile {
            allowed_tools: BTreeSet::from(["edit_file".into()]),
            allowed_command_prefixes: vec!["git ".into()],
            ..ProviderProfile::default()
        };
        let config = profile.provider_config(ExecutionResidency::RemoteWorker);
        assert!(serde_json::to_string(&config)
            .unwrap()
            .contains("edit_file"));
        assert!(!serde_json::to_string(&config)
            .unwrap()
            .contains("DESKTOP_MODEL_API_KEY"));
        assert_eq!(config.extra["worker_execution_residency"], "remote_worker");
        assert_eq!(config.extra["sandbox_mode"], "disabled");
        assert_eq!(config.extra["permissions"]["edit_file"], "ask");
        assert_eq!(config.extra["permissions"]["bash"], "deny");
        assert_eq!(
            config.extra["command_allowlist"],
            serde_json::json!(["git "])
        );
        assert!(profile.command_allowed("git status"));
        assert!(!profile.command_allowed("rm -rf /"));
    }

    #[test]
    fn rejects_invalid_command_prefixes() {
        let prefix = ProviderProfile {
            allowed_command_prefixes: vec![" ".into()],
            ..ProviderProfile::default()
        };
        assert!(prefix.validate().is_err());
    }

    #[test]
    fn rejects_filesystem_root_and_parent_escape_roots() {
        let root = WorkerConfig {
            projects: vec![ProjectRegistration {
                id: "fixture".into(),
                root: PathBuf::from("/"),
            }],
            trajectory_root: PathBuf::from("/workspace/trajectory"),
            ..WorkerConfig::default()
        };
        assert!(root.validate().is_err());

        let parent = WorkerConfig {
            projects: vec![ProjectRegistration {
                id: "fixture".into(),
                root: PathBuf::from("/workspace/../outside"),
            }],
            trajectory_root: PathBuf::from("/workspace/trajectory"),
            ..WorkerConfig::default()
        };
        assert!(parent.validate().is_err());
    }
}
