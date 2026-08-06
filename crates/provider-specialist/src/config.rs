use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use agent_core::{Error, ProviderConfig, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const CLARK_BASE_URL: &str = "https://api.clarkslabs.com/v1";
const CLARK_FREE_MODEL: &str = "clark-code:free";
const CLARK_GLM_MODEL: &str = "clark-code:glm52";
const CLARK_DEEPSEEK_MODEL: &str = "clark-code:deepseek_v4_flash_latest";
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const PAID_QWEN_MODEL: &str = "qwen/qwen3.7-flash";
const CHILD_KEY_ENV: &str = "CLARK_SPECIALIST_MODEL_KEY";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScoutContextSnapshot {
    schema_version: u32,
    workspace_id: String,
    entries: Vec<ScoutContextEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScoutContextEntry {
    object_kind: String,
    object_id: String,
    classification: String,
    attributes: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecialistKind {
    Scientist,
    Rsi,
}

impl SpecialistKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scientist => "scientist",
            Self::Rsi => "rsi",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecialistConnectConfig {
    pub specialist: SpecialistKind,
    pub workflow: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scout_context: Option<Value>,
    pub runtime_root: PathBuf,
    pub worker_sha256: String,
    #[serde(default)]
    pub model_route: ModelRoute,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default)]
    pub advisor_training_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<SpecialistRemoteTarget>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub remote_worker_binaries: BTreeMap<String, PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecialistRemoteTarget {
    pub host: String,
    pub remote_root: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRoute {
    #[default]
    ClarkDeepseekV4Latest,
    ClarkGlm52,
    ClarkFree,
    PaidQwen37Flash,
}

#[derive(Clone, Debug)]
pub struct ConnectedConfig {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub specialist: SpecialistKind,
    pub workflow: String,
    pub organization_id: Option<String>,
    pub workspace_id: Option<String>,
    pub scout_context: Option<Value>,
    pub runtime_root: PathBuf,
    pub worker_sha256: String,
    pub project_id: String,
    pub model_route: ModelRoute,
    pub max_iterations: u32,
    pub advisor_training_enabled: bool,
    pub model_key: Option<String>,
    pub remote: Option<SpecialistRemoteTarget>,
    pub remote_worker_binaries: BTreeMap<String, PathBuf>,
}

impl SpecialistConnectConfig {
    pub fn validate(&self) -> Result<()> {
        let workflow_valid = matches!(
            (self.specialist, self.workflow.as_str()),
            (SpecialistKind::Scientist, "scientist:discover")
                | (SpecialistKind::Scientist, "scientist:replicate")
                | (SpecialistKind::Rsi, "rsi:research")
                | (SpecialistKind::Rsi, "rsi:create-evals")
                | (SpecialistKind::Rsi, "rsi:build-world")
                | (SpecialistKind::Rsi, "rsi:stress-test")
                | (SpecialistKind::Rsi, "rsi:regression")
        );
        if !workflow_valid {
            return Err(Error::Protocol(
                "specialist and workflow do not name a supported runtime lane".into(),
            ));
        }
        if !self.runtime_root.is_absolute() {
            return Err(Error::Protocol(
                "specialist runtime_root must be absolute".into(),
            ));
        }
        if self.worker_sha256.len() != 64
            || !self
                .worker_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Error::Protocol(
                "specialist workerSha256 must be one exact SHA-256 digest".into(),
            ));
        }
        if self.max_iterations == 0 || self.max_iterations > 12 {
            return Err(Error::Protocol(
                "specialist maxIterations must be between 1 and 12".into(),
            ));
        }
        if let Some(remote) = &self.remote {
            if remote.host.trim().is_empty()
                || remote.host.len() > 256
                || remote.host.starts_with('-')
                || !remote.host.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(byte, b'.' | b'_' | b'@' | b':' | b'-' | b'[' | b']')
                })
                || !remote.remote_root.is_absolute()
                || remote.remote_root == Path::new("/")
                || remote
                    .remote_root
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(Error::Protocol(
                    "specialist remote target is invalid".into(),
                ));
            }
        }
        for (arch, binary) in &self.remote_worker_binaries {
            if !matches!(
                arch.as_str(),
                "linux-x86_64" | "linux-aarch64" | "darwin-aarch64" | "darwin-x86_64"
            ) || !binary.is_absolute()
                || !binary.is_file()
            {
                return Err(Error::Protocol(
                    "specialist remote worker binary map is invalid".into(),
                ));
            }
        }
        if self
            .organization_id
            .as_deref()
            .is_some_and(|value| !portable_identifier(value))
        {
            return Err(Error::Protocol(
                "specialist organizationId is invalid".into(),
            ));
        }
        if self
            .workspace_id
            .as_deref()
            .is_some_and(|value| !portable_identifier(value))
        {
            return Err(Error::Protocol("specialist workspaceId is invalid".into()));
        }
        if let Some(context) = &self.scout_context {
            let snapshot: ScoutContextSnapshot =
                serde_json::from_value(context.clone()).map_err(|error| {
                    Error::Protocol(format!("specialist scoutContext is invalid: {error}"))
                })?;
            let size_valid =
                serde_json::to_vec(context).is_ok_and(|encoded| encoded.len() <= 16 * 1024);
            let entries_valid = snapshot.entries.len() <= 64
                && snapshot.entries.iter().all(|entry| {
                    !entry.object_kind.is_empty()
                        && entry.object_kind.len() <= 64
                        && !entry.object_id.is_empty()
                        && entry.object_id.len() <= 256
                        && !entry.classification.is_empty()
                        && entry.classification.len() <= 64
                        && entry.attributes.len() <= 64
                });
            if self.specialist != SpecialistKind::Rsi
                || snapshot.schema_version != 1
                || self.workspace_id.as_deref() != Some(snapshot.workspace_id.as_str())
                || !portable_identifier(&snapshot.workspace_id)
                || !size_valid
                || !entries_valid
            {
                return Err(Error::Protocol(
                    "specialist scoutContext does not match the bounded RSI source contract".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn worker_config(
        &self,
        session_id: &str,
        cwd: &Path,
        project_id: &str,
        cloud_api_base_url: &str,
        execution_residency: &str,
        trajectory_root: &Path,
    ) -> Value {
        let (base_url, model, structured_output_mode, allow_paid_models) = match self.model_route {
            ModelRoute::ClarkDeepseekV4Latest => {
                (CLARK_BASE_URL, CLARK_DEEPSEEK_MODEL, "json_object", true)
            }
            ModelRoute::ClarkGlm52 => (CLARK_BASE_URL, CLARK_GLM_MODEL, "json_schema", true),
            ModelRoute::ClarkFree => (CLARK_BASE_URL, CLARK_FREE_MODEL, "json_schema", false),
            ModelRoute::PaidQwen37Flash => {
                (OPENROUTER_BASE_URL, PAID_QWEN_MODEL, "json_object", true)
            }
        };
        let mut worker = serde_json::json!({
            "schema_version": 1,
            "projects": [{
                "id": project_id,
                "root": cwd,
            }],
            "trajectory_root": trajectory_root,
            "execution_residency": execution_residency,
            "allowed_evaluator_commands": [],
            "allow_paid_models": allow_paid_models,
            "advisor_training_enabled": self.advisor_training_enabled,
            "provider": {
                "base_url": base_url,
                "model": model,
                "api_key_env": CHILD_KEY_ENV,
                "reasoning_effort": match self.model_route {
                    ModelRoute::ClarkDeepseekV4Latest => Value::String("max".into()),
                    ModelRoute::ClarkGlm52 => Value::String("xhigh".into()),
                    ModelRoute::ClarkFree => Value::Null,
                    ModelRoute::PaidQwen37Flash => Value::String("max".into()),
                },
                "structured_output_mode": structured_output_mode,
                "max_iterations": self.max_iterations,
            },
        });
        worker["cloud_sync"] = serde_json::json!({
            "api_base_url": cloud_api_base_url,
            "organization_id": self.organization_id,
            "scope_id": session_id,
            "api_key_env": CHILD_KEY_ENV,
        });
        if self.organization_id.is_none() {
            worker["cloud_sync"]
                .as_object_mut()
                .expect("cloud sync config is an object")
                .remove("organization_id");
        }
        worker
    }
}

impl ConnectedConfig {
    pub fn parse(config: ProviderConfig) -> Result<Self> {
        let command = config
            .command
            .filter(|command| !command.is_empty())
            .ok_or_else(|| {
                Error::Unsupported("specialist provider requires a native worker command".into())
            })?;
        let cwd = config
            .cwd
            .map(PathBuf::from)
            .ok_or_else(|| Error::Protocol("specialist provider requires a project cwd".into()))?;
        let extra: SpecialistConnectConfig =
            serde_json::from_value(config.extra).map_err(|error| {
                Error::Protocol(format!("invalid specialist provider config: {error}"))
            })?;
        extra.validate()?;
        let cwd = if let Some(remote) = &extra.remote {
            if !cwd.is_absolute() || cwd != remote.remote_root {
                return Err(Error::Protocol(
                    "specialist remote cwd must match the registered SSH project root".into(),
                ));
            }
            cwd
        } else {
            cwd.canonicalize().map_err(|error| {
                Error::Io(format!(
                    "specialist project root could not be resolved: {error}"
                ))
            })?
        };
        let project_id = project_id(&cwd);
        Ok(Self {
            command,
            cwd,
            specialist: extra.specialist,
            workflow: extra.workflow,
            organization_id: extra.organization_id,
            workspace_id: extra.workspace_id,
            scout_context: extra.scout_context,
            runtime_root: extra.runtime_root,
            worker_sha256: extra.worker_sha256.to_ascii_lowercase(),
            project_id,
            model_route: extra.model_route,
            max_iterations: extra.max_iterations,
            advisor_training_enabled: extra.advisor_training_enabled,
            model_key: config.auth_token.filter(|value| !value.trim().is_empty()),
            remote: extra.remote,
            remote_worker_binaries: extra.remote_worker_binaries,
        })
    }

    pub fn child_key_env(&self) -> &'static str {
        CHILD_KEY_ENV
    }
}

/// Add native-only runtime and worker locations to a WebView-authored
/// specialist config. Existing values are rejected instead of trusted.
pub fn prepare_native_config(
    mut config: ProviderConfig,
    app_data_dir: &Path,
    worker_executable: &Path,
    remote_worker_binaries: BTreeMap<String, PathBuf>,
) -> Result<ProviderConfig> {
    if !app_data_dir.is_absolute() || !worker_executable.is_absolute() {
        return Err(Error::Protocol(
            "native specialist paths must be absolute".into(),
        ));
    }
    let mut extra = match config.extra {
        Value::Object(value) => value,
        Value::Null => Map::new(),
        _ => {
            return Err(Error::Protocol(
                "specialist provider extra must be an object".into(),
            ))
        }
    };
    extra.insert(
        "runtimeRoot".into(),
        Value::String(
            app_data_dir
                .join("scientist-runtime")
                .to_string_lossy()
                .into_owned(),
        ),
    );
    extra.insert(
        "workerSha256".into(),
        Value::String(sha256_file(worker_executable)?),
    );
    extra.insert(
        "remoteWorkerBinaries".into(),
        serde_json::to_value(remote_worker_binaries).map_err(|error| {
            Error::Protocol(format!(
                "could not encode native remote worker paths: {error}"
            ))
        })?,
    );
    // The installed Clark Code lane uses the Clark-managed DeepSeek V4 Flash
    // 0731 route. Direct OpenRouter Qwen remains available only to explicit
    // evaluation harnesses that bypass this WebView-to-native preparation
    // boundary.
    extra.insert(
        "modelRoute".into(),
        Value::String("clark_deepseek_v4_latest".into()),
    );
    config.extra = Value::Object(extra);
    config.command = Some(vec![worker_executable.to_string_lossy().into_owned()]);
    Ok(config)
}

fn project_id(cwd: &Path) -> String {
    let digest = Sha256::digest(cwd.to_string_lossy().as_bytes());
    let suffix = digest[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("desktop-{suffix}")
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|error| {
        Error::Io(format!(
            "could not open specialist worker {}: {error}",
            path.display()
        ))
    })?;
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut chunk).map_err(|error| {
            Error::Io(format!(
                "could not hash specialist worker {}: {error}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn portable_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn default_max_iterations() -> u32 {
    3
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn native_preparation_overrides_untrusted_paths() {
        let temp = tempfile::TempDir::new().unwrap();
        let worker = temp.path().join("clark-code-headless");
        std::fs::write(&worker, b"worker").unwrap();
        let config = ProviderConfig {
            endpoint: None,
            command: Some(vec!["untrusted".into()]),
            cwd: Some("/tmp".into()),
            headers: HashMap::new(),
            auth_token: None,
            extra: serde_json::json!({
                "specialist": "scientist",
                "workflow": "scientist:discover",
                "runtimeRoot": "/untrusted"
            }),
        };
        let remote_worker = temp.path().join("clark-code-headless-linux-x86_64");
        std::fs::write(&remote_worker, b"remote-worker").unwrap();
        let prepared = prepare_native_config(
            config,
            Path::new("/native/app-data"),
            &worker,
            [("linux-x86_64".to_string(), remote_worker.clone())]
                .into_iter()
                .collect(),
        )
        .unwrap();
        assert_eq!(
            prepared.command,
            Some(vec![worker.to_string_lossy().into_owned()])
        );
        assert_eq!(
            prepared.extra["runtimeRoot"],
            "/native/app-data/scientist-runtime"
        );
        assert_eq!(
            prepared.extra["remoteWorkerBinaries"]["linux-x86_64"],
            remote_worker.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn mismatched_workflow_fails_closed() {
        let config = SpecialistConnectConfig {
            specialist: SpecialistKind::Scientist,
            workflow: "rsi:stress-test".into(),
            organization_id: None,
            workspace_id: None,
            scout_context: None,
            runtime_root: PathBuf::from("/tmp/runtime"),
            worker_sha256: "a".repeat(64),
            model_route: ModelRoute::ClarkDeepseekV4Latest,
            max_iterations: 3,
            advisor_training_enabled: false,
            remote: None,
            remote_worker_binaries: BTreeMap::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn paid_qwen_worker_profile_is_exact_and_explicit() {
        let config = SpecialistConnectConfig {
            specialist: SpecialistKind::Rsi,
            workflow: "rsi:stress-test".into(),
            organization_id: Some("org-1".into()),
            workspace_id: Some("workspace-1".into()),
            scout_context: None,
            runtime_root: PathBuf::from("/tmp/runtime"),
            worker_sha256: "b".repeat(64),
            model_route: ModelRoute::PaidQwen37Flash,
            max_iterations: 3,
            advisor_training_enabled: false,
            remote: None,
            remote_worker_binaries: BTreeMap::new(),
        };
        let worker = config.worker_config(
            "session-1",
            Path::new("/tmp"),
            "project-1",
            CLARK_BASE_URL,
            "local_only",
            Path::new("/tmp/runtime/sessions/session-1"),
        );
        assert_eq!(worker["provider"]["model"], PAID_QWEN_MODEL);
        assert_eq!(worker["provider"]["reasoning_effort"], "max");
        assert_eq!(worker["provider"]["structured_output_mode"], "json_object");
        assert_eq!(worker["allow_paid_models"], true);
        assert_eq!(worker["cloud_sync"]["organization_id"], "org-1");
        assert_eq!(worker["cloud_sync"]["scope_id"], "session-1");
        assert_eq!(worker["cloud_sync"]["api_key_env"], CHILD_KEY_ENV);
    }

    #[test]
    fn default_deepseek_worker_profile_is_exact_and_explicit() {
        let config = SpecialistConnectConfig {
            specialist: SpecialistKind::Scientist,
            workflow: "scientist:discover".into(),
            organization_id: Some("org-1".into()),
            workspace_id: Some("workspace-1".into()),
            scout_context: None,
            runtime_root: PathBuf::from("/tmp/runtime"),
            worker_sha256: "d".repeat(64),
            model_route: ModelRoute::default(),
            max_iterations: 3,
            advisor_training_enabled: false,
            remote: None,
            remote_worker_binaries: BTreeMap::new(),
        };
        let worker = config.worker_config(
            "session-1",
            Path::new("/tmp"),
            "project-1",
            CLARK_BASE_URL,
            "local_only",
            Path::new("/tmp/runtime/sessions/session-1"),
        );
        assert_eq!(worker["provider"]["model"], CLARK_DEEPSEEK_MODEL);
        assert_eq!(worker["provider"]["reasoning_effort"], "max");
        assert_eq!(worker["provider"]["structured_output_mode"], "json_object");
        assert_eq!(worker["allow_paid_models"], true);
    }

    #[test]
    fn rsi_scout_context_is_typed_bounded_and_workspace_pinned() {
        let mut config = SpecialistConnectConfig {
            specialist: SpecialistKind::Rsi,
            workflow: "rsi:create-evals".into(),
            organization_id: Some("org-1".into()),
            workspace_id: Some("workspace-1".into()),
            scout_context: Some(serde_json::json!({
                "schemaVersion": 1,
                "workspaceId": "workspace-1",
                "entries": [{
                    "objectKind": "entity",
                    "objectId": "checkout",
                    "classification": "internal",
                    "attributes": {"kind": "service"}
                }]
            })),
            runtime_root: PathBuf::from("/tmp/runtime"),
            worker_sha256: "c".repeat(64),
            model_route: ModelRoute::ClarkDeepseekV4Latest,
            max_iterations: 3,
            advisor_training_enabled: false,
            remote: None,
            remote_worker_binaries: BTreeMap::new(),
        };
        config.validate().unwrap();
        config.workspace_id = Some("workspace-2".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn native_product_route_pins_clark_deepseek_v4_flash_latest() {
        let temp = tempfile::TempDir::new().unwrap();
        let worker = temp.path().join("clark-code-headless");
        std::fs::write(&worker, b"worker").unwrap();
        let config = ProviderConfig {
            endpoint: None,
            command: None,
            cwd: Some("/tmp".into()),
            headers: HashMap::new(),
            auth_token: Some("not-written".into()),
            extra: serde_json::json!({
                "specialist": "scientist",
                "workflow": "scientist:discover",
                "modelRoute": "paid_qwen37_flash"
            }),
        };
        let prepared = prepare_native_config(
            config,
            Path::new("/native/app-data"),
            &worker,
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(prepared.extra["modelRoute"], "clark_deepseek_v4_latest");
    }
}
