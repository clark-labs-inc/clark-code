use std::collections::HashMap;
use std::path::{Path, PathBuf};

use agent_core::{
    Provider, ProviderConfig, ProviderConfiguration, ProviderConfigurationChange, Session,
    SessionOptions,
};
use provider_specialist::{prepare_native_config, SpecialistProvider};
use serde_json::json;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::cloud::RuntimeScope;
use crate::conversation::ActiveConversation;
use crate::tui::settings::{ConfigurationPreferences, ModelConfiguration};
use crate::tui::specialists::SpecialistContinuityKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Workspace {
    Code,
    Scout,
    SecurityScan,
    SecurityDiff,
    SecurityDeep,
    ScientistDiscover,
    ScientistReplicate,
    RsiResearch,
    RsiCreateEvals,
    RsiBuildWorld,
    RsiStressTest,
    RsiRegression,
}

impl Workspace {
    pub fn label(self) -> &'static str {
        match self {
            Self::Code => "Code",
            Self::Scout => "Scout",
            Self::SecurityScan | Self::SecurityDiff | Self::SecurityDeep => "Security",
            Self::ScientistDiscover | Self::ScientistReplicate => "Scientist",
            Self::RsiResearch
            | Self::RsiCreateEvals
            | Self::RsiBuildWorld
            | Self::RsiStressTest
            | Self::RsiRegression => "RSI",
        }
    }

    pub fn default_prompt(self, prompt: &str) -> String {
        match self {
            Self::Code
            | Self::ScientistDiscover
            | Self::ScientistReplicate
            | Self::RsiResearch
            | Self::RsiCreateEvals
            | Self::RsiBuildWorld
            | Self::RsiStressTest
            | Self::RsiRegression => prompt.to_string(),
            Self::Scout => format!("$scout:scout {prompt}"),
            Self::SecurityScan => format!("$security:security-scan {prompt}"),
            Self::SecurityDiff => format!("$security:security-diff {prompt}"),
            Self::SecurityDeep => format!("$security:security-deep {prompt}"),
        }
    }

    fn specialist(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::ScientistDiscover => Some(("scientist", "scientist:discover")),
            Self::ScientistReplicate => Some(("scientist", "scientist:replicate")),
            Self::RsiResearch => Some(("rsi", "rsi:research")),
            Self::RsiCreateEvals => Some(("rsi", "rsi:create-evals")),
            Self::RsiBuildWorld => Some(("rsi", "rsi:build-world")),
            Self::RsiStressTest => Some(("rsi", "rsi:stress-test")),
            Self::RsiRegression => Some(("rsi", "rsi:regression")),
            _ => None,
        }
    }

    pub fn paid_specialist_kind(self) -> Option<&'static str> {
        match self {
            Self::Code => None,
            Self::Scout => Some("scout"),
            Self::SecurityScan | Self::SecurityDiff | Self::SecurityDeep => Some("security"),
            Self::ScientistDiscover | Self::ScientistReplicate => Some("scientist"),
            Self::RsiResearch
            | Self::RsiCreateEvals
            | Self::RsiBuildWorld
            | Self::RsiStressTest
            | Self::RsiRegression => Some("rsi"),
        }
    }
}

pub struct ConnectedRuntime {
    pub provider: Box<dyn Provider>,
    pub session: Session,
    pub diagnostics: RuntimeDiagnostics,
    pub model_configuration: ModelConfiguration,
    pub conversation: ActiveConversation,
    security_cloud: Option<SecurityCloudRuntime>,
}

#[derive(Clone, Debug)]
pub struct RuntimeDiagnosticValue {
    pub label: String,
    pub value: String,
    pub source: String,
}

impl RuntimeDiagnosticValue {
    fn new(label: impl Into<String>, value: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            source: source.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeDiagnostics {
    pub authentication: RuntimeDiagnosticValue,
    pub organization: RuntimeDiagnosticValue,
    pub plan: RuntimeDiagnosticValue,
    pub workspace: RuntimeDiagnosticValue,
    pub provider: RuntimeDiagnosticValue,
    pub sync: RuntimeDiagnosticValue,
    pub configuration: Vec<RuntimeDiagnosticValue>,
}

struct SecurityCloudRuntime {
    rest_base: String,
    api_key: Zeroizing<String>,
    owner_scope: String,
    organization_id: String,
    binding: crate::cloud::SecurityScope,
    identity_root: PathBuf,
    http: reqwest::Client,
}

impl SecurityCloudRuntime {
    fn from_scope(scope: &RuntimeScope, api_key: &str) -> Result<Option<Self>, String> {
        let Some(binding) = scope.security.clone() else {
            return Ok(None);
        };
        let owner_scope = scope
            .owner_scope
            .clone()
            .ok_or_else(|| "Clark Security account binding is missing".to_string())?;
        let organization_id = scope
            .organization_id
            .clone()
            .ok_or_else(|| "Clark Security organization binding is missing".to_string())?;
        let http = clark_http::build_client(clark_http::ClientOptions {
            request_timeout: Some(std::time::Duration::from_secs(120)),
            user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
            ..Default::default()
        })
        .map_err(|error| format!("could not initialize Clark Security cloud sync: {error}"))?;
        Ok(Some(Self {
            rest_base: crate::auth::platform_api_origin()?,
            api_key: Zeroizing::new(api_key.to_string()),
            owner_scope,
            organization_id,
            binding,
            identity_root: clark_home()?.join("security-identities"),
            http,
        }))
    }

    async fn sync(&self) -> Result<String, String> {
        let result =
            security_cloud_sync::sync_security_scans(security_cloud_sync::SecuritySyncRequest {
                rest_base: self.rest_base.clone(),
                api_key: self.api_key.to_string(),
                owner_scope: self.owner_scope.clone(),
                organization_id: self.organization_id.clone(),
                repository_id: self.binding.repository_id.clone(),
                policy_id: Some(self.binding.policy_id.clone()),
                root: self.binding.root.clone(),
                identity_root: self.identity_root.clone(),
                repository: self.binding.repository.clone(),
                http: self.http.clone(),
                scanner_display_name: "Clark CLI Security".into(),
                scanner_version: format!("clark-cli/{}", env!("CARGO_PKG_VERSION")),
                trigger: "cli".into(),
                source: "clark-cli".into(),
            })
            .await?;
        if result.failed_count > 0 || result.pending_count > 0 {
            let detail = result
                .scans
                .iter()
                .find_map(|scan| scan.message.as_deref())
                .unwrap_or("Clark Security cloud synchronization did not complete");
            return Err(format!(
                "Clark Security refused to finish locally: {} sealed scan(s), {} pending, {} failed. {detail}",
                result.sealed_scan_count, result.pending_count, result.failed_count
            ));
        }
        Ok(format!(
            "Clark Security cloud synchronized ({} new, {} already present).",
            result.synced_count, result.already_synced_count
        ))
    }
}

impl ConnectedRuntime {
    pub async fn configure(
        &mut self,
        change: ProviderConfigurationChange,
    ) -> Result<ProviderConfiguration, String> {
        self.provider
            .configure(&self.session.id, change)
            .await
            .map_err(|error| format!("Clark rejected the configuration change: {error}"))
    }

    pub async fn begin_turn(&mut self, prompt: &agent_core::PromptInput) -> Result<(), String> {
        self.provider
            .validate_prompt(&self.session.id, prompt)
            .await
            .map_err(|error| format!("Clark rejected the turn: {error}"))?;
        self.conversation.begin_turn(prompt).await
    }

    pub async fn record_event(&mut self, event: &agent_core::AgentEvent) -> Result<(), String> {
        self.conversation.apply(event).await
    }

    pub async fn sync_after_finish(&mut self) -> Result<Option<String>, String> {
        let specialist = match &self.security_cloud {
            Some(security) => Some(security.sync().await?),
            None => None,
        };
        let conversation = self.conversation.finish_turn().await?;
        Ok(Some(match specialist {
            Some(specialist) => format!("{specialist}\n{conversation}"),
            None => conversation,
        }))
    }
}

pub async fn sync_before_start(
    workspace: Workspace,
    scope: &RuntimeScope,
    api_key: &str,
) -> Result<(), String> {
    if matches!(
        workspace,
        Workspace::ScientistDiscover
            | Workspace::ScientistReplicate
            | Workspace::RsiResearch
            | Workspace::RsiCreateEvals
            | Workspace::RsiBuildWorld
            | Workspace::RsiStressTest
            | Workspace::RsiRegression
    ) {
        let organization_id = scope.organization_id.as_deref().ok_or_else(|| {
            "Clark science artifact synchronization has no organization binding. No worker or model was started."
                .to_string()
        })?;
        crate::science_cloud::preflight(api_key, organization_id).await?;
    }
    if let Some(security) = SecurityCloudRuntime::from_scope(scope, api_key)? {
        security.sync().await?;
    }
    Ok(())
}

pub async fn connect(
    workspace: Workspace,
    cwd: &Path,
    api_key: &str,
    credential_created_by: &str,
    scope: &RuntimeScope,
    mut conversation: ActiveConversation,
) -> Result<ConnectedRuntime, String> {
    let cwd = cwd
        .canonicalize()
        .map_err(|error| format!("could not open project {}: {error}", cwd.display()))?;
    let permission_path = crate::tui::permission_profiles::PermissionProfileState::path(&cwd);
    let permission_profile = crate::tui::permission_profiles::PermissionProfileState::load(
        &permission_path,
    )
    .map_err(|error| format!("Clark cannot start with invalid permission state: {error}"))?;
    let configuration_path = ModelConfiguration::path(&cwd);
    let configuration_capabilities = provider_local::configuration_capabilities();
    let code_preferences = if workspace == Workspace::Code {
        Some(
            ConfigurationPreferences::load(&configuration_path, &configuration_capabilities)
                .map_err(|error| {
                    format!("Clark cannot start with invalid agent settings: {error}")
                })?,
        )
    } else {
        None
    };
    let permission_modes = json!({
        "bash": permission_profile.profile().mode_for("bash"),
        "write_file": permission_profile.profile().mode_for("write_file"),
        "edit_file": permission_profile.profile().mode_for("edit_file"),
    });
    let sandbox_read_roots = permission_profile
        .read_roots()
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let mut provider: Box<dyn Provider> =
        if let Some((specialist, workflow)) = workspace.specialist() {
            let worker = specialist_worker()?;
            let runtime_root = clark_home()?.join("specialist-runtime");
            std::fs::create_dir_all(&runtime_root).map_err(|error| {
                format!(
                    "could not create specialist runtime {}: {error}",
                    runtime_root.display()
                )
            })?;
            let base = ProviderConfig {
                endpoint: None,
                command: None,
                cwd: Some(cwd.to_string_lossy().into_owned()),
                headers: HashMap::new(),
                auth_token: Some(api_key.to_string()),
                extra: json!({
                    "specialist": specialist,
                    "workflow": workflow,
                    "organizationId": scope.organization_id,
                    "permissions": permission_modes,
                    "sandbox_mode": permission_profile.sandbox_mode(),
                    "sandbox_read_roots": sandbox_read_roots,
                }),
            };
            let prepared = prepare_native_config(base, &clark_home()?, &worker, Default::default())
                .map_err(|error| format!("could not configure {}: {error}", workspace.label()))?;
            let mut provider = SpecialistProvider::new();
            provider
                .connect(prepared)
                .await
                .map_err(|error| format!("could not start {}: {error}", workspace.label()))?;
            Box::new(provider)
        } else {
            let model = code_preferences
                .as_ref()
                .map_or(provider_local::DEFAULT_MODEL, |preferences| {
                    preferences.model()
                });
            let reasoning_effort = code_preferences
                .as_ref()
                .and_then(|preferences| preferences.reasoning_effort(&configuration_capabilities))
                .unwrap_or("xhigh");
            let memories_enabled = code_preferences
                .as_ref()
                .is_none_or(ConfigurationPreferences::memories_enabled);
            let browser_enabled = code_preferences
                .as_ref()
                .and_then(|preferences| preferences.experiment_enabled("browser"))
                .unwrap_or(true);
            let mut extra = json!({
                "base_url": provider_local::DEFAULT_BASE_URL,
                "model": model,
                "reasoning_effort": reasoning_effort,
                "permissions": permission_modes,
                "research": true,
                "memories": memories_enabled,
                "orchestration": true,
                "browser_enabled": browser_enabled,
                "computer_use_enabled": false,
                "project_knowledge": true,
                "sandbox_mode": permission_profile.sandbox_mode(),
                "sandbox_read_roots": sandbox_read_roots,
            });
            if workspace == Workspace::Scout {
                let organization_id = scope
                    .organization_id
                    .as_deref()
                    .ok_or("Clark Scout organization binding is missing")?;
                let workspace_id = scope
                    .workspace_id
                    .as_deref()
                    .ok_or("Clark Scout workspace binding is missing")?;
                let identity_root = clark_home()?
                    .join("scout")
                    .join(format!("{organization_id}-{workspace_id}-local"));
                extra["scout_cartography"] = json!({
                    "organization_id": organization_id,
                    "workspace_id": workspace_id,
                    "identity_root": identity_root,
                    "platform": std::env::consts::OS,
                    "architecture": std::env::consts::ARCH,
                });
            }
            let config = ProviderConfig {
                endpoint: None,
                command: None,
                cwd: Some(cwd.to_string_lossy().into_owned()),
                headers: HashMap::new(),
                auth_token: Some(api_key.to_string()),
                extra,
            };
            let mut provider = provider_local::LocalAgentProvider::new();
            provider
                .connect(config)
                .await
                .map_err(|error| format!("could not start Clark Code: {error}"))?;
            Box::new(provider)
        };
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(cwd.to_string_lossy().into_owned()),
            resume: conversation.snapshot.resume_transcript(),
            ..SessionOptions::default()
        })
        .await
        .map_err(|error| format!("could not open {} workspace: {error}", workspace.label()))?;
    if let Some(preferences) = code_preferences.as_ref() {
        provider
            .configure(
                &session.id,
                ProviderConfigurationChange::OutputStyle {
                    style: preferences.output_style().to_string(),
                },
            )
            .await
            .map_err(|error| format!("could not restore Clark personality: {error}"))?;
    }
    let live_configuration = provider
        .configuration(&session.id)
        .await
        .map_err(|error| format!("could not inspect Clark provider settings: {error}"))?;
    let model_configuration = if workspace == Workspace::Code {
        ModelConfiguration::active(live_configuration.clone())
    } else {
        ModelConfiguration::locked(format!(
            "{} is a paid specialist workspace. Its model and reasoning policy are selected by the specialist's Clark capability.",
            workspace.label()
        ))
    };
    let security_cloud = SecurityCloudRuntime::from_scope(scope, api_key)?;
    conversation.bind_session(&session.id);
    if let Err(error) = conversation.publish_ready().await {
        let _ = provider.close_session(&session.id).await;
        return Err(error);
    }
    let diagnostics = runtime_diagnostics(
        workspace,
        &cwd,
        credential_created_by,
        scope,
        &permission_profile,
        &live_configuration,
    );
    Ok(ConnectedRuntime {
        provider,
        session,
        diagnostics,
        model_configuration,
        conversation,
        security_cloud,
    })
}

fn runtime_diagnostics(
    workspace: Workspace,
    cwd: &Path,
    credential_created_by: &str,
    scope: &RuntimeScope,
    permission_profile: &crate::tui::permission_profiles::PermissionProfileState,
    configuration: &ProviderConfiguration,
) -> RuntimeDiagnostics {
    let specialist = workspace.paid_specialist_kind();
    let provider = if matches!(
        workspace,
        Workspace::ScientistDiscover | Workspace::ScientistReplicate
    ) || matches!(
        workspace,
        Workspace::RsiResearch
            | Workspace::RsiCreateEvals
            | Workspace::RsiBuildWorld
            | Workspace::RsiStressTest
            | Workspace::RsiRegression
    ) {
        "Clark native specialist worker"
    } else {
        "Clark local provider"
    };
    let sync = match workspace {
        Workspace::SecurityScan | Workspace::SecurityDiff | Workspace::SecurityDeep => {
            "mandatory sealed preflight and post-run synchronization"
        }
        Workspace::Scout => "authoritative Clark Scout cloud workspace; no local-only journal",
        Workspace::ScientistDiscover
        | Workspace::ScientistReplicate
        | Workspace::RsiResearch
        | Workspace::RsiCreateEvals
        | Workspace::RsiBuildWorld
        | Workspace::RsiStressTest
        | Workspace::RsiRegression => {
            "worker-enforced startup reconciliation and verified per-action cloud receipts"
        }
        Workspace::Code => "mandatory account-scoped Clark Cloud conversation synchronization",
    };
    let sync_source = specialist
        .and_then(SpecialistContinuityKind::from_name)
        .map(SpecialistContinuityKind::continuity_owner)
        .unwrap_or("Clark runtime continuity gate");
    let model = if workspace.paid_specialist_kind().is_some() {
        RuntimeDiagnosticValue::new(
            "Model",
            "selected by specialist capability",
            "Clark specialist worker",
        )
    } else {
        RuntimeDiagnosticValue::new(
            "Model",
            configuration.model.as_deref().unwrap_or("not reported"),
            "Clark provider capability",
        )
    };
    RuntimeDiagnostics {
        authentication: RuntimeDiagnosticValue::new(
            "Authentication",
            format!("verified ({credential_created_by})"),
            "Clark Cloud /cli/context",
        ),
        organization: RuntimeDiagnosticValue::new(
            "Organization",
            scope
                .organization_id
                .as_deref()
                .unwrap_or("personal account"),
            "Clark credential resolution",
        ),
        plan: RuntimeDiagnosticValue::new(
            "Plan",
            if specialist.is_some() {
                "paid specialist entitlement verified"
            } else {
                "Code access verified"
            },
            "Clark access response",
        ),
        workspace: RuntimeDiagnosticValue::new(
            "Workspace",
            format!("{} · {}", workspace.label(), cwd.display()),
            "Clark CLI selection",
        ),
        provider: RuntimeDiagnosticValue::new("Provider", provider, "Clark runtime dispatch"),
        sync: RuntimeDiagnosticValue::new("Cloud sync", sync, sync_source),
        configuration: vec![
            model,
            RuntimeDiagnosticValue::new(
                "Reasoning",
                if workspace.paid_specialist_kind().is_some() {
                    "selected by specialist capability"
                } else {
                    configuration
                        .reasoning_effort
                        .as_deref()
                        .unwrap_or("provider default")
                },
                "Clark provider capability",
            ),
            RuntimeDiagnosticValue::new(
                "Permissions",
                format!(
                    "{} (shell={}, file writes={})",
                    permission_profile.profile().name(),
                    permission_profile.profile().mode_for("bash"),
                    permission_profile.profile().mode_for("write_file"),
                ),
                "Clark durable permission profile",
            ),
            RuntimeDiagnosticValue::new(
                "Sandbox",
                format!(
                    "{}; {} additional readable roots",
                    permission_profile.sandbox_mode(),
                    permission_profile.read_roots().len()
                ),
                "Clark durable permission profile",
            ),
            RuntimeDiagnosticValue::new(
                "Cloud credential",
                "present and verified; secret withheld",
                credential_created_by,
            ),
        ],
    }
}

pub fn specialist_worker() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CLARK_SPECIALIST_WORKER") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "CLARK_SPECIALIST_WORKER does not name a file: {}",
            path.display()
        ));
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate the Clark executable: {error}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| "Clark executable has no parent directory".to_string())?;
    let filename = if cfg!(windows) {
        "clark-code-headless.exe"
    } else {
        "clark-code-headless"
    };
    let worker = directory.join(filename);
    if worker.is_file() {
        Ok(worker)
    } else {
        Err(format!(
            "Clark's specialist worker is missing at {}. Reinstall with `curl -fsSL https://www.clarkchat.com/install.sh | sh`.",
            worker.display()
        ))
    }
}

pub fn worker_sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "could not read specialist worker {}: {error}",
            path.display()
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn clark_home() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CLARK_HOME") {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".clark"))
        .ok_or_else(|| "could not locate the current user's home directory".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_the_unprefixed_default_workspace() {
        assert_eq!(Workspace::Code.default_prompt("fix it"), "fix it");
        assert_eq!(
            Workspace::Scout.default_prompt("map it"),
            "$scout:scout map it"
        );
    }

    #[test]
    fn all_five_human_workspaces_have_labels() {
        let labels = [
            Workspace::Code,
            Workspace::Scout,
            Workspace::SecurityScan,
            Workspace::ScientistDiscover,
            Workspace::RsiCreateEvals,
        ]
        .map(Workspace::label);
        assert_eq!(labels, ["Code", "Scout", "Security", "Scientist", "RSI"]);
    }
}
