//! Compile-time product boundary for branded distributions.
//!
//! The open host owns native execution, provider lifecycles, and safety. A
//! product supplies opaque operations and presentation metadata from a private
//! composition crate; the renderer never receives credentials or host state.

use agent_core::{Provider, ProviderConfig};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::AppHandle;
use zeroize::Zeroizing;

use crate::runtime_registry::{AccountKey, CloudAccountState};
use crate::state::AppState;
use crate::ProviderInfo;

pub struct ProductRequestContext<'a> {
    pub app: &'a AppHandle,
    pub state: &'a AppState,
}

pub struct ProductAccountAuthority {
    service_base: String,
    account_id: String,
    bearer: Zeroizing<String>,
}

pub struct ProductProviderConfig {
    pub config: ProviderConfig,
    pub account_id: Option<String>,
}

pub struct ProductRemoteWorkerRequest {
    pub host: String,
    pub remote_root: PathBuf,
    pub model: String,
    pub reasoning_effort: String,
}

pub struct ProductRemoteWorkerLaunch {
    pub owner_scope: String,
    pub spec: code_remote::RemoteWorkerSpec,
    pub credentials: HashMap<String, String>,
}

impl ProductAccountAuthority {
    pub fn service_base(&self) -> &str {
        &self.service_base
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn bearer(&self) -> &str {
        self.bearer.as_str()
    }
}

impl ProductRequestContext<'_> {
    pub async fn read_workspace_markdown(
        &self,
        source_uri: &str,
        conversation_id: &str,
    ) -> Result<(String, Vec<u8>), String> {
        crate::commands::read_workspace_markdown(source_uri, conversation_id).await
    }

    pub async fn write_workspace_markdown(
        &self,
        conversation_id: &str,
        filename: &str,
        markdown: &[u8],
    ) -> Result<(), String> {
        crate::commands::write_workspace_markdown(conversation_id, filename, markdown).await
    }

    pub async fn remove_workspace_markdown(
        &self,
        source_uri: &str,
        conversation_id: &str,
    ) -> Result<(), String> {
        crate::commands::remove_workspace_markdown(source_uri, conversation_id).await
    }

    pub async fn skill_catalog_service(&self) -> Arc<provider_local::SkillCatalogService> {
        self.state.runtime_registry.current_skill_catalogs().await
    }

    pub async fn store_remote_command_claim(
        &self,
        command_id: String,
        host_id: String,
        instance_id: String,
        claim_token: String,
    ) -> Result<(), String> {
        let authority = self.account_authority().await?;
        let account = AccountKey::new(authority.account_id().to_string())?;
        self.state
            .runtime_registry
            .store_command_claim(account, command_id, host_id, instance_id, claim_token)
            .await
    }

    pub async fn remote_command_claim(
        &self,
        command_id: &str,
        host_id: &str,
        instance_id: &str,
    ) -> Result<String, String> {
        let authority = self.account_authority().await?;
        let account = AccountKey::new(authority.account_id().to_string())?;
        self.state
            .runtime_registry
            .command_claim(&account, command_id, host_id, instance_id)
            .await
    }

    pub async fn remove_remote_command_claim(&self, command_id: &str) -> Result<(), String> {
        let authority = self.account_authority().await?;
        let account = AccountKey::new(authority.account_id().to_string())?;
        self.state
            .runtime_registry
            .remove_command_claim(&account, command_id)
            .await;
        Ok(())
    }

    pub async fn account_authority(&self) -> Result<ProductAccountAuthority, String> {
        let authority = self
            .state
            .runtime_registry
            .cloud_account()
            .await
            .ok_or("this product requires an authenticated account")?;
        Ok(ProductAccountAuthority {
            service_base: authority.rest_base,
            account_id: authority.account.as_str().to_string(),
            bearer: Zeroizing::new(authority.token.as_str().to_string()),
        })
    }

    pub async fn model_api_key(&self) -> Result<Zeroizing<String>, String> {
        let authority = self.account_authority().await?;
        self.state
            .credentials
            .code_key(authority.account_id())
            .await?
            .ok_or("this account has no model API credential".to_string())
    }

    pub async fn existing_model_api_key(&self) -> Result<Option<Zeroizing<String>>, String> {
        let authority = self.account_authority().await?;
        self.state
            .credentials
            .code_key(authority.account_id())
            .await
    }

    pub async fn store_model_api_key(&self, secret: String) -> Result<(), String> {
        let authority = self.account_authority().await?;
        self.state
            .credentials
            .set_code_key(authority.account_id(), secret)
            .await
    }

    pub async fn mcp_environment(
        &self,
        credential_ref: &str,
        names: &[String],
    ) -> Result<HashMap<String, String>, String> {
        let authority = self.account_authority().await?;
        self.state
            .credentials
            .mcp_environment(authority.account_id(), credential_ref, names)
            .await
    }

    pub async fn retained_session(&self) -> Result<Option<Zeroizing<String>>, String> {
        self.state.credentials.retained_auth().await
    }

    pub async fn bind_account(
        &self,
        service_base: String,
        account_id: String,
        bearer: String,
        retained_session: String,
    ) -> Result<(), String> {
        let account = AccountKey::new(account_id)?;
        let _switch = self.state.account_lifecycle.write().await;
        if let Some(active) = self.state.runtime_registry.cloud_account().await {
            if active.account != account || active.rest_base != service_base {
                return Err(
                    "account_mismatch: sign out before connecting a different product account"
                        .into(),
                );
            }
        }
        self.state
            .credentials
            .set_retained_auth(retained_session)
            .await?;
        self.state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: service_base,
                account,
                token: Zeroizing::new(bearer),
            }))
            .await;
        Ok(())
    }

    pub async fn clear_account(&self) -> Result<(), String> {
        let _switch = self.state.account_lifecycle.write().await;
        let mut generation = self
            .state
            .runtime_registry
            .cloud_account_generation_write()
            .await;
        let active = generation.clone();
        let active_owner = active
            .as_ref()
            .map(|account| account.account.as_str().to_string());
        self.state
            .credentials
            .sign_out(active_owner.as_deref())
            .await?;
        let active_account = active.map(|account| account.account);
        let removed_sessions = if let Some(account) = active_account.as_ref() {
            self.state
                .runtime_registry
                .take_account_sessions(account)
                .await
        } else {
            Vec::new()
        };
        *generation = None;
        drop(generation);
        if let Some(account) = active_account {
            self.state
                .runtime_registry
                .disconnect_account(&account)
                .await;
        }
        for entry in removed_sessions {
            let mut live = entry.lock().await;
            live.closing = true;
            let session_id = live.session.id.clone();
            if let Err(error) = live.provider.close_session(&session_id).await {
                tracing::warn!(%error, session = %session_id, "signed-out provider close failed");
            }
        }
        Ok(())
    }
}

#[async_trait]
pub trait ProductIntegration: Send + Sync {
    fn id(&self) -> &str;

    fn updates_enabled(&self, _debug_build: bool, _bundle_identifier: &str) -> bool {
        false
    }

    fn credential_envelope_policy(&self) -> CredentialEnvelopePolicy {
        CredentialEnvelopePolicy::neutral()
    }

    fn computer_use_permission_owner(
        &self,
        _debug_build: bool,
        platform: &str,
    ) -> (String, String) {
        let bundle_id = match platform {
            "windows" => "agent-computer-use-helper.exe",
            "linux" => "org.agentdesktop.ComputerUse",
            _ => "org.agentdesktop.computer-use",
        };
        ("Agent Computer Use".into(), bundle_id.into())
    }

    #[cfg(target_os = "macos")]
    fn qa_data_store_identifier(&self) -> Option<[u8; 16]> {
        None
    }

    fn providers(&self, builtins: Vec<ProviderInfo>) -> Vec<ProviderInfo> {
        builtins
    }

    async fn make_provider(
        &self,
        _provider_id: &str,
        _config: &ProviderConfig,
        _context: ProductRequestContext<'_>,
    ) -> Result<Option<Box<dyn Provider>>, String> {
        Ok(None)
    }

    async fn prepare_provider_config(
        &self,
        _provider_id: &str,
        _config: ProviderConfig,
        _context: ProductRequestContext<'_>,
    ) -> Result<Option<ProductProviderConfig>, String> {
        Ok(None)
    }

    async fn prepare_remote_worker(
        &self,
        _request: ProductRemoteWorkerRequest,
        _context: ProductRequestContext<'_>,
    ) -> Result<Option<ProductRemoteWorkerLaunch>, String> {
        Ok(None)
    }

    async fn request(
        &self,
        operation: &str,
        payload: Value,
        context: ProductRequestContext<'_>,
    ) -> Result<Value, String>;

    async fn publish_projection(
        &self,
        _payload: &Value,
        _context: ProductRequestContext<'_>,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }
}

#[derive(Clone, Copy)]
pub struct CredentialEnvelopePolicy {
    pub magic: &'static [u8; 8],
    pub obsolete_magic: &'static [u8; 8],
    pub aad: &'static [u8],
}

impl CredentialEnvelopePolicy {
    pub const fn neutral() -> Self {
        Self {
            magic: b"AGTCRD02",
            obsolete_magic: b"AGTCRD01",
            aad: b"agent-desktop-credentials-v2",
        }
    }
}

pub struct NeutralProduct;

#[async_trait]
impl ProductIntegration for NeutralProduct {
    fn id(&self) -> &str {
        "neutral"
    }

    async fn request(
        &self,
        _operation: &str,
        _payload: Value,
        _context: ProductRequestContext<'_>,
    ) -> Result<Value, String> {
        Err("this build has no product integration".to_string())
    }
}
