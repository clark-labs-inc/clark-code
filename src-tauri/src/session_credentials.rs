//! App-owned encrypted Clark Code credentials without an operating-system vault.
//!
//! Clark Code stores one authenticated-encryption envelope and one random wrapping
//! key under its owner-private app-data directory. This avoids Keychain,
//! Credential Manager, and Secret Service prompts or platform behavior. It
//! protects against casual disclosure and detects tampering; a process already
//! running as the same OS user can read both local files and is outside this
//! threat boundary.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use zeroize::Zeroizing;

const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_PLAINTEXT_BYTES: usize = 768 * 1024;
const MAX_ACCOUNTS: usize = 32;
const MAX_MCP_SERVERS_PER_ACCOUNT: usize = 64;
const MAX_MCP_ENV_PER_SERVER: usize = 128;

mod storage;

use storage::{load_state, persist_state};

pub(crate) struct SessionCredentials {
    policy: crate::product::CredentialEnvelopePolicy,
    root: OnceLock<PathBuf>,
    secrets: RwLock<SecretState>,
    load_gate: Mutex<()>,
    loaded: AtomicBool,
}

#[derive(Clone, Default)]
struct SecretState {
    retained_auth: Option<Zeroizing<String>>,
    code_keys: HashMap<String, Zeroizing<String>>,
    mcp_env: HashMap<String, HashMap<String, HashMap<String, Zeroizing<String>>>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaintextState {
    version: u32,
    retained_auth: Option<String>,
    code_keys: HashMap<String, String>,
    #[serde(default)]
    mcp_env: HashMap<String, HashMap<String, HashMap<String, String>>>,
}

impl SessionCredentials {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_policy(crate::product::CredentialEnvelopePolicy::neutral())
    }

    pub(crate) fn with_policy(policy: crate::product::CredentialEnvelopePolicy) -> Self {
        Self {
            policy,
            root: OnceLock::new(),
            secrets: RwLock::new(SecretState::default()),
            load_gate: Mutex::new(()),
            loaded: AtomicBool::new(false),
        }
    }

    pub(crate) fn configure(&self, root: PathBuf) -> Result<(), String> {
        exec_private_fs::ensure_private_dir(&root).map_err(|_| {
            "could not initialize Clark Code's private credential directory".to_string()
        })?;
        self.root.set(root).map_err(|_| {
            "Clark Code credential directory was configured more than once".to_string()
        })
    }

    pub(crate) async fn code_key(
        &self,
        owner_scope: &str,
    ) -> Result<Option<Zeroizing<String>>, String> {
        validate_owner(owner_scope)?;
        self.ensure_loaded().await?;
        Ok(self
            .secrets
            .read()
            .await
            .code_keys
            .get(owner_scope)
            .map(|secret| Zeroizing::new(secret.as_str().to_string())))
    }

    pub(crate) async fn retained_auth(&self) -> Result<Option<Zeroizing<String>>, String> {
        self.ensure_loaded().await?;
        Ok(self
            .secrets
            .read()
            .await
            .retained_auth
            .as_ref()
            .map(|session| Zeroizing::new(session.as_str().to_string())))
    }

    pub(crate) async fn set_retained_auth(&self, retained: String) -> Result<(), String> {
        validate_retained_auth(&retained)?;
        self.ensure_loaded().await?;
        let mut state = self.secrets.write().await;
        let mut candidate = state.clone();
        candidate.retained_auth = Some(Zeroizing::new(retained));
        self.persist(&candidate).await?;
        *state = candidate;
        Ok(())
    }

    pub(crate) async fn sign_out(&self, owner_scope: Option<&str>) -> Result<(), String> {
        if let Some(owner_scope) = owner_scope {
            validate_owner(owner_scope)?;
        }
        self.ensure_loaded().await?;
        let mut state = self.secrets.write().await;
        let mut candidate = state.clone();
        candidate.retained_auth = None;
        if let Some(owner_scope) = owner_scope {
            candidate.code_keys.remove(owner_scope);
            candidate.mcp_env.remove(owner_scope);
        }
        self.persist(&candidate).await?;
        *state = candidate;
        Ok(())
    }

    pub(crate) async fn set_code_key(
        &self,
        owner_scope: &str,
        secret: String,
    ) -> Result<(), String> {
        validate_owner(owner_scope)?;
        validate_secret(&secret)?;
        self.ensure_loaded().await?;
        let mut state = self.secrets.write().await;
        let mut candidate = state.clone();
        candidate
            .code_keys
            .insert(owner_scope.to_string(), Zeroizing::new(secret));
        self.persist(&candidate).await?;
        *state = candidate;
        Ok(())
    }

    pub(crate) async fn sync_mcp_environment(
        &self,
        owner_scope: &str,
        servers: HashMap<String, HashMap<String, String>>,
    ) -> Result<(), String> {
        validate_owner(owner_scope)?;
        self.ensure_loaded().await?;
        if servers.len() > MAX_MCP_SERVERS_PER_ACCOUNT {
            return Err("too many MCP servers are configured".into());
        }
        for (server, environment) in &servers {
            validate_portable(server, "MCP server id")?;
            if environment.len() > MAX_MCP_ENV_PER_SERVER {
                return Err("too many MCP environment values are configured".into());
            }
            for (name, value) in environment {
                validate_env_name(name)?;
                if !value.is_empty() {
                    validate_mcp_value(value)?;
                }
            }
        }
        let mut state = self.secrets.write().await;
        let mut candidate = state.clone();
        let existing = candidate
            .mcp_env
            .entry(owner_scope.to_string())
            .or_default();
        existing.retain(|server, _| servers.contains_key(server));
        for (server, submitted) in servers {
            let values = existing.entry(server).or_default();
            values.retain(|name, _| submitted.contains_key(name));
            for (name, value) in submitted {
                if !value.is_empty() {
                    values.insert(name, Zeroizing::new(value));
                }
            }
        }
        self.persist(&candidate).await?;
        *state = candidate;
        Ok(())
    }

    pub(crate) async fn mcp_environment(
        &self,
        owner_scope: &str,
        server: &str,
        names: &[String],
    ) -> Result<HashMap<String, String>, String> {
        validate_owner(owner_scope)?;
        validate_portable(server, "MCP server id")?;
        self.ensure_loaded().await?;
        let state = self.secrets.read().await;
        let stored = state
            .mcp_env
            .get(owner_scope)
            .and_then(|servers| servers.get(server));
        let mut result = HashMap::new();
        for name in names {
            validate_env_name(name)?;
            let value = stored
                .and_then(|environment| environment.get(name))
                .ok_or_else(|| format!("MCP credential {name} is not configured"))?;
            result.insert(name.clone(), value.as_str().to_string());
        }
        Ok(result)
    }

    async fn ensure_loaded(&self) -> Result<(), String> {
        if self.loaded.load(Ordering::Acquire) {
            return Ok(());
        }
        let _loading = self.load_gate.lock().await;
        if self.loaded.load(Ordering::Acquire) {
            return Ok(());
        }
        let root = self.root()?.to_path_buf();
        let policy = self.policy;
        let loaded = tokio::task::spawn_blocking(move || load_state(&root, policy))
            .await
            .map_err(|_| "Clark Code credential read task failed".to_string())??;
        *self.secrets.write().await = SecretState {
            retained_auth: loaded.retained_auth.map(Zeroizing::new),
            code_keys: loaded
                .code_keys
                .into_iter()
                .map(|(owner, secret)| (owner, Zeroizing::new(secret)))
                .collect(),
            mcp_env: loaded
                .mcp_env
                .into_iter()
                .map(|(owner, servers)| {
                    (
                        owner,
                        servers
                            .into_iter()
                            .map(|(server, environment)| {
                                (
                                    server,
                                    environment
                                        .into_iter()
                                        .map(|(name, value)| (name, Zeroizing::new(value)))
                                        .collect(),
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
        };
        self.loaded.store(true, Ordering::Release);
        Ok(())
    }

    async fn persist(&self, state: &SecretState) -> Result<(), String> {
        let root = self.root()?.to_path_buf();
        let plaintext = PlaintextState {
            version: 2,
            retained_auth: state
                .retained_auth
                .as_ref()
                .map(|session| session.as_str().to_string()),
            code_keys: state
                .code_keys
                .iter()
                .map(|(owner, secret)| (owner.clone(), secret.as_str().to_string()))
                .collect(),
            mcp_env: state
                .mcp_env
                .iter()
                .map(|(owner, servers)| {
                    (
                        owner.clone(),
                        servers
                            .iter()
                            .map(|(server, environment)| {
                                (
                                    server.clone(),
                                    environment
                                        .iter()
                                        .map(|(name, value)| {
                                            (name.clone(), value.as_str().to_string())
                                        })
                                        .collect(),
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
        };
        let policy = self.policy;
        tokio::task::spawn_blocking(move || persist_state(&root, plaintext, policy))
            .await
            .map_err(|_| "Clark Code credential write task failed".to_string())?
    }

    fn root(&self) -> Result<&Path, String> {
        self.root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| "Clark Code credential directory is not initialized".to_string())
    }
}

fn validate_owner(owner_scope: &str) -> Result<(), String> {
    if owner_scope.is_empty()
        || owner_scope.len() > 256
        || owner_scope.chars().any(char::is_control)
    {
        return Err("Clark Code account identity is invalid".into());
    }
    Ok(())
}

fn validate_secret(secret: &str) -> Result<(), String> {
    if secret.len() < 16 || secret.len() > 4096 || secret.contains(['\n', '\r', '\0']) {
        return Err("Clark Code returned an invalid Code credential".into());
    }
    Ok(())
}

fn validate_mcp_value(value: &str) -> Result<(), String> {
    if value.len() > 4096 || value.contains(['\n', '\r', '\0']) {
        return Err("MCP credential value is invalid".into());
    }
    Ok(())
}

fn validate_retained_auth(retained: &str) -> Result<(), String> {
    if retained.is_empty() || retained.len() > 64 * 1024 || retained.contains('\0') {
        return Err("Clark Code retained auth is invalid".into());
    }
    serde_json::from_str::<serde_json::Value>(retained)
        .map(|_| ())
        .map_err(|_| "Clark Code retained auth is invalid".to_string())
}

fn validate_portable(value: &str, what: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character))
    {
        return Err(format!("{what} is invalid"));
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), String> {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return Err("MCP environment name is invalid".into());
    };
    if name.len() > 128
        || !(first.is_ascii_alphabetic() || first == '_')
        || !characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("MCP environment name is invalid".into());
    }
    Ok(())
}

fn validate_state(state: &PlaintextState) -> Result<(), String> {
    if state.version != 2
        || state.code_keys.len() > MAX_ACCOUNTS
        || state.mcp_env.len() > MAX_ACCOUNTS
    {
        return Err("Clark Code's encrypted credential payload is unsupported".into());
    }
    let mut payload_bytes = 0_usize;
    if let Some(retained) = &state.retained_auth {
        validate_retained_auth(retained)?;
        payload_bytes = payload_bytes.saturating_add(retained.len());
    }
    for (owner, secret) in &state.code_keys {
        validate_owner(owner)?;
        validate_secret(secret)?;
        payload_bytes = payload_bytes
            .saturating_add(owner.len())
            .saturating_add(secret.len());
    }
    for (owner, servers) in &state.mcp_env {
        validate_owner(owner)?;
        payload_bytes = payload_bytes.saturating_add(owner.len());
        if servers.len() > MAX_MCP_SERVERS_PER_ACCOUNT {
            return Err("Clark Code's encrypted credential payload is unsupported".into());
        }
        for (server, environment) in servers {
            validate_portable(server, "MCP server id")?;
            payload_bytes = payload_bytes.saturating_add(server.len());
            if environment.len() > MAX_MCP_ENV_PER_SERVER {
                return Err("Clark Code's encrypted credential payload is unsupported".into());
            }
            for (name, value) in environment {
                validate_env_name(name)?;
                validate_mcp_value(value)?;
                payload_bytes = payload_bytes
                    .saturating_add(name.len())
                    .saturating_add(value.len());
            }
        }
    }
    if payload_bytes > MAX_PLAINTEXT_BYTES {
        return Err("Clark Code's encrypted credential payload is too large".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_credentials/tests.rs"]
mod tests;
