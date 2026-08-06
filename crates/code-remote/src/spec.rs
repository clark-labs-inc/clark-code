use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use thiserror::Error;

/// Trusted launcher input for a remote worker. `worker_config` is the exact
/// JSON written to the remote host; it must not contain credentials.
#[derive(Clone, Debug)]
pub struct RemoteWorkerSpec {
    pub host: String,
    pub project_id: String,
    pub remote_root: PathBuf,
    pub trajectory_root: PathBuf,
    pub worker_config: Value,
    /// Optional local worker binary to upload. If absent, the versioned remote
    /// installation must already exist at `remote_binary`.
    pub local_binary: Option<PathBuf>,
    /// Architecture-keyed local workers shipped by the native host. The
    /// transport selects only after SSH proves the remote platform. An exact
    /// `local_binary` override takes precedence for test and harness callers.
    pub local_binaries: BTreeMap<String, PathBuf>,
    pub remote_binary: Option<String>,
    /// Names of local environment variables whose values are sent as bounded
    /// SSH bootstrap lines and exported under the same names remotely. This
    /// supports separate model and Clark Cloud credentials without putting
    /// either secret in worker configuration.
    pub credential_envs: Vec<String>,
}

impl RemoteWorkerSpec {
    pub fn validate(&self) -> Result<(), SpecError> {
        if !portable_host(&self.host) {
            return Err(SpecError::InvalidHost(self.host.clone()));
        }
        if !portable_id(&self.project_id) {
            return Err(SpecError::InvalidProject(self.project_id.clone()));
        }
        if !self.remote_root.is_absolute() || !self.trajectory_root.is_absolute() {
            return Err(SpecError::AbsoluteRootsRequired);
        }
        if self.remote_root == std::path::Path::new("/")
            || self.trajectory_root == std::path::Path::new("/")
        {
            return Err(SpecError::RootTooBroad);
        }
        if self
            .remote_root
            .components()
            .chain(self.trajectory_root.components())
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(SpecError::UnsafeRoot);
        }
        if self.remote_root.to_string_lossy().contains('\0')
            || self.trajectory_root.to_string_lossy().contains('\0')
        {
            return Err(SpecError::NulPath);
        }
        if let Some(binary) = &self.remote_binary {
            if !binary.starts_with('/') || binary.contains('\0') {
                return Err(SpecError::InvalidRemoteBinary(binary.clone()));
            }
        }
        if let Some(binary) = &self.local_binary {
            if !binary.is_absolute() || binary.to_str().is_none() {
                return Err(SpecError::InvalidLocalBinary(binary.clone()));
            }
        }
        for (arch, binary) in &self.local_binaries {
            if !matches!(
                arch.as_str(),
                "linux-x86_64" | "linux-aarch64" | "darwin-aarch64" | "darwin-x86_64"
            ) || !binary.is_absolute()
                || binary.to_str().is_none()
            {
                return Err(SpecError::InvalidLocalBinary(binary.clone()));
            }
        }
        for env in &self.credential_envs {
            if !portable_env(env) {
                return Err(SpecError::InvalidCredentialEnv(env.clone()));
            }
        }
        if self
            .credential_envs
            .iter()
            .enumerate()
            .any(|(index, env)| self.credential_envs[..index].contains(env))
        {
            return Err(SpecError::InvalidConfig(
                "credential_envs may not contain duplicates".into(),
            ));
        }
        validate_config(
            &self.worker_config,
            &self.project_id,
            &self.remote_root,
            &self.trajectory_root,
            &self.credential_envs,
        )
    }

    pub fn config_bytes(&self) -> Result<Vec<u8>, SpecError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(&self.worker_config)
            .map_err(|error| SpecError::InvalidConfig(error.to_string()))?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

fn validate_config(
    config: &Value,
    project_id: &str,
    remote_root: &std::path::Path,
    trajectory_root: &std::path::Path,
    credential_envs: &[String],
) -> Result<(), SpecError> {
    let object = config
        .as_object()
        .ok_or_else(|| SpecError::InvalidConfig("worker_config must be an object".into()))?;
    if object.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(SpecError::InvalidConfig(
            "worker_config schema_version must be 1".into(),
        ));
    }
    if object.get("execution_residency").and_then(Value::as_str) != Some("remote_worker") {
        return Err(SpecError::InvalidConfig(
            "remote worker config execution_residency must be remote_worker".into(),
        ));
    }
    let projects = object
        .get("projects")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SpecError::InvalidConfig("worker_config projects must be an array".into())
        })?;
    let project = projects
        .iter()
        .find(|project| project.get("id").and_then(Value::as_str) == Some(project_id))
        .ok_or_else(|| {
            SpecError::InvalidConfig("worker_config does not register project_id".into())
        })?;
    let remote_root = remote_root.to_string_lossy();
    if project.get("root").and_then(Value::as_str) != Some(remote_root.as_ref()) {
        return Err(SpecError::InvalidConfig(
            "worker_config project root does not match remote_root".into(),
        ));
    }
    let trajectory_root = trajectory_root.to_string_lossy();
    if object.get("trajectory_root").and_then(Value::as_str) != Some(trajectory_root.as_ref()) {
        return Err(SpecError::InvalidConfig(
            "worker_config trajectory_root does not match trajectory_root".into(),
        ));
    }
    if contains_secret_key(config) {
        return Err(SpecError::CredentialInConfig);
    }
    let config_env = config
        .get("provider")
        .and_then(Value::as_object)
        .and_then(|provider| provider.get("api_key_env"))
        .and_then(Value::as_str);
    if let Some(env) = config_env {
        if !portable_env(env) {
            return Err(SpecError::InvalidCredentialEnv(env.into()));
        }
    }
    if !credential_envs.is_empty() {
        let Some(config_env) = config_env else {
            return Err(SpecError::InvalidConfig(
                "credential_envs require provider.api_key_env".into(),
            ));
        };
        if !credential_envs.iter().any(|env| env == config_env) {
            return Err(SpecError::InvalidConfig(
                "credential_envs must include provider.api_key_env".into(),
            ));
        }
        let cloud_env = config
            .get("cloud_sync")
            .and_then(Value::as_object)
            .and_then(|cloud| cloud.get("api_key_env"))
            .and_then(Value::as_str);
        if let Some(cloud_env) = cloud_env {
            if !portable_env(cloud_env) {
                return Err(SpecError::InvalidCredentialEnv(cloud_env.into()));
            }
        }
        if credential_envs
            .iter()
            .any(|env| env != config_env && Some(env.as_str()) != cloud_env)
        {
            return Err(SpecError::InvalidConfig(
                "credential_envs may name only provider and cloud_sync key environments".into(),
            ));
        }
    }
    Ok(())
}

fn contains_secret_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let key = key.to_ascii_lowercase();
            (key.contains("secret")
                || key.contains("credential")
                || key.contains("password")
                || key.contains("token")
                || key == "authorization"
                || key == "access_key"
                || key == "private_key"
                || key == "api_key"
                || key == "apikey")
                || contains_secret_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_secret_key),
        _ => false,
    }
}

fn portable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn portable_env(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && (byte.is_ascii_alphabetic() || byte == b'_'))
                || (index > 0 && (byte.is_ascii_alphanumeric() || byte == b'_'))
        })
}

fn portable_host(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value.len() <= 256
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'@' | b':' | b'-' | b'[' | b']')
        })
}

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("invalid SSH host: {0}")]
    InvalidHost(String),
    #[error("invalid project id: {0}")]
    InvalidProject(String),
    #[error("remote_root and trajectory_root must be absolute")]
    AbsoluteRootsRequired,
    #[error("project and trajectory roots may not be the filesystem root")]
    RootTooBroad,
    #[error("project and trajectory roots may not contain parent-directory components")]
    UnsafeRoot,
    #[error("remote path contains NUL")]
    NulPath,
    #[error("invalid remote worker binary: {0}")]
    InvalidRemoteBinary(String),
    #[error("local worker binary must be an absolute UTF-8 path: {0}")]
    InvalidLocalBinary(PathBuf),
    #[error("invalid credential environment variable: {0}")]
    InvalidCredentialEnv(String),
    #[error("worker config contains a credential or secret field")]
    CredentialInConfig,
    #[error("invalid worker config: {0}")]
    InvalidConfig(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(config: Value) -> RemoteWorkerSpec {
        RemoteWorkerSpec {
            host: "cpu".into(),
            project_id: "fixture".into(),
            remote_root: PathBuf::from("/workspace/fixture"),
            trajectory_root: PathBuf::from("/workspace/.clark/trajectory"),
            worker_config: config,
            local_binary: None,
            local_binaries: BTreeMap::new(),
            remote_binary: None,
            credential_envs: vec!["CLARK_CODE_API_KEY".into()],
        }
    }

    fn config() -> Value {
        serde_json::json!({
            "schema_version": 1,
            "projects": [{"id": "fixture", "root": "/workspace/fixture"}],
            "trajectory_root": "/workspace/.clark/trajectory",
            "provider": {"api_key_env": "CLARK_CODE_API_KEY"},
            "execution_residency": "remote_worker"
        })
    }

    #[test]
    fn config_is_bound_to_the_remote_root_and_has_no_secret_fields() {
        assert!(spec(config()).validate().is_ok());
        let mut secret = config();
        secret["provider"]["api_key"] = Value::String("nope".into());
        assert!(matches!(
            spec(secret).validate(),
            Err(SpecError::CredentialInConfig)
        ));
    }

    #[test]
    fn rejects_shell_and_path_injection() {
        let mut invalid = spec(config());
        invalid.host = "cpu;touch /tmp/pwned".into();
        assert!(matches!(invalid.validate(), Err(SpecError::InvalidHost(_))));
        invalid = spec(config());
        invalid.remote_binary = Some("relative/worker".into());
        assert!(matches!(
            invalid.validate(),
            Err(SpecError::InvalidRemoteBinary(_))
        ));
        invalid.remote_binary = None;
        invalid.local_binary = Some("relative/worker".into());
        assert!(matches!(
            invalid.validate(),
            Err(SpecError::InvalidLocalBinary(_))
        ));
        invalid = spec(config());
        invalid.remote_root = PathBuf::from("/");
        assert!(matches!(invalid.validate(), Err(SpecError::RootTooBroad)));
        invalid = spec(config());
        invalid.host = "-oProxyCommand=bad".into();
        assert!(matches!(invalid.validate(), Err(SpecError::InvalidHost(_))));
    }

    #[test]
    fn rejects_nested_credential_aliases() {
        for key in ["authorization", "access_key", "private_key", "credential"] {
            let mut config = config();
            config["provider"][key] = Value::String("secret".into());
            assert!(matches!(
                spec(config).validate(),
                Err(SpecError::CredentialInConfig)
            ));
        }
    }

    #[test]
    fn accepts_unattended_permissions_only_in_the_remote_worker_config() {
        let mut config = config();
        config["provider"]["permission_mode"] = Value::String("allow_anything".into());
        config["provider"]["sandbox_mode"] = Value::String("danger-full-access".into());
        assert!(spec(config).validate().is_ok());
    }

    #[test]
    fn remote_bootstrap_accepts_separate_declared_model_and_cloud_keys() {
        let mut config = config();
        config["provider"]["api_key_env"] = Value::String("OPENROUTER_API_KEY".into());
        config["cloud_sync"] = serde_json::json!({
            "api_base_url": "https://api.clarkslabs.com/v1",
            "organization_id": "00000000-0000-0000-0000-000000000000",
            "scope_id": "gpu-science",
            "api_key_env": "CLARK_API_KEY"
        });
        let mut remote = spec(config);
        remote.credential_envs = vec!["OPENROUTER_API_KEY".into(), "CLARK_API_KEY".into()];
        assert!(remote.validate().is_ok());
    }
}
