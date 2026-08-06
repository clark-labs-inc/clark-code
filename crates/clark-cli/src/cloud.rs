use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::runtime::Workspace;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliContext {
    account_id: String,
    key: CliKey,
    code: ProductAccess,
    specialists: BTreeMap<String, ProductAccess>,
    organizations: Vec<Organization>,
    organization_selection: OrganizationSelection,
    billing_url: String,
    login_hint: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliKey {
    name: String,
    purpose: String,
    cli_compatible: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductAccess {
    allowed: bool,
    state: String,
    organization_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Organization {
    id: String,
    name: String,
    role: String,
    status: String,
    seat_kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizationChoice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrganizationSelection {
    state: String,
    selected_organization_id: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeScope {
    pub owner_scope: Option<String>,
    pub organization_id: Option<String>,
    pub workspace_id: Option<String>,
    pub security: Option<SecurityScope>,
}

#[derive(Clone, Debug)]
pub struct SecurityScope {
    pub repository_id: String,
    pub policy_id: String,
    pub root: PathBuf,
    pub repository: provider_local::RepositoryIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductStatus {
    pub label: &'static str,
    pub allowed: bool,
    pub state: String,
}

impl CliContext {
    pub fn organization_choices(&self) -> Vec<OrganizationChoice> {
        self.organizations
            .iter()
            .filter(|organization| organization.status == "active")
            .map(|organization| OrganizationChoice {
                id: organization.id.clone(),
                name: organization.name.clone(),
            })
            .collect()
    }

    pub fn uses_metered_platform_key(&self) -> bool {
        self.key.purpose == "general"
    }

    pub fn product_statuses(&self) -> Result<Vec<ProductStatus>, String> {
        let mut statuses = vec![ProductStatus {
            label: "Code",
            allowed: self.code.allowed,
            state: self.code.state.clone(),
        }];
        for (kind, label) in [
            ("scout", "Scout"),
            ("security", "Security"),
            ("scientist", "Scientist"),
            ("rsi", "RSI"),
        ] {
            let access = self
                .specialists
                .get(kind)
                .ok_or_else(|| format!("Clark did not return {label} access state"))?;
            statuses.push(ProductStatus {
                label,
                allowed: access.allowed,
                state: access.state.clone(),
            });
        }
        Ok(statuses)
    }

    pub fn native_specialist_worker_required(&self) -> bool {
        ["scientist", "rsi"].iter().any(|kind| {
            self.specialists
                .get(*kind)
                .is_some_and(|access| access.allowed)
        })
    }

    pub fn authorize(&self, workspace: Workspace) -> Result<RuntimeScope, String> {
        if !self.key.cli_compatible || !self.code.allowed {
            return Err(format!(
                "The stored key ({}, purpose {}) is not accepted by Clark CLI. {}",
                self.key.name, self.key.purpose, self.login_hint
            ));
        }
        let base_scope = RuntimeScope {
            owner_scope: Some(self.account_id.clone()),
            ..RuntimeScope::default()
        };
        let Some(kind) = workspace.paid_specialist_kind() else {
            return Ok(base_scope);
        };
        let access = self
            .specialists
            .get(kind)
            .ok_or_else(|| format!("Clark did not return {kind} access state"))?;
        if access.allowed {
            let organization_id = access.organization_id.clone().ok_or_else(|| {
                format!(
                    "Clark authorized {} but returned no cloud organization for its durable data. No worker or model was started.",
                    workspace.label()
                )
            })?;
            return Ok(RuntimeScope {
                owner_scope: base_scope.owner_scope,
                organization_id: Some(organization_id),
                ..RuntimeScope::default()
            });
        }
        match access.state.as_str() {
            "subscription_required" => Err(format!(
                "Clark {} is available on paid plans. No worker or model was started. Upgrade or restore coverage at {}.",
                workspace.label(), self.billing_url
            )),
            "action_needed" => Err(format!(
                "Clark {} is paused because billing needs attention. No worker or model was started. Review {}.",
                workspace.label(), self.billing_url
            )),
            "organization_selection_required" => {
                let choices = self
                    .organizations
                    .iter()
                    .filter(|organization| organization.status == "active")
                    .map(|organization| {
                        format!(
                            "  {}  {} (role {}, seat {})",
                            organization.id,
                            organization.name,
                            organization.role,
                            organization.seat_kind,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Err(format!(
                    "More than one paid Clark organization can run {}. Choose explicitly with `--organization ORGANIZATION_ID`:\n{}",
                    workspace.label(), choices
                ))
            }
            "organization_required" => Err(format!(
                "Your paid plan covers Clark {}, but this account has no active cloud organization for its durable specialist data. No worker or model was started. Create or join a Clark workspace at https://www.clarkchat.com/team.",
                workspace.label()
            )),
            other => Err(format!(
                "Clark could not authorize {} (state {other}). No worker or model was started.",
                workspace.label()
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoutWorkspaceList {
    workspaces: Vec<ScoutWorkspace>,
}

#[derive(Clone, Debug, Deserialize)]
struct ScoutWorkspace {
    id: String,
    display_name: String,
    status: String,
}

pub async fn prepare_runtime_scope(
    context: &CliContext,
    workspace: Workspace,
    api_key: &str,
    requested_workspace_id: Option<&str>,
    create_workspace_name: Option<&str>,
    cwd: &Path,
) -> Result<RuntimeScope, String> {
    let mut scope = context.authorize(workspace)?;
    if matches!(
        workspace,
        Workspace::SecurityScan | Workspace::SecurityDiff | Workspace::SecurityDeep
    ) {
        if requested_workspace_id.is_some() || create_workspace_name.is_some() {
            return Err(
                "--workspace and --create-workspace are only valid with `clark scout`".into(),
            );
        }
        scope.security = Some(register_security_repository(&scope, api_key, cwd).await?);
        return Ok(scope);
    }
    if workspace != Workspace::Scout {
        if requested_workspace_id.is_some() || create_workspace_name.is_some() {
            return Err(
                "--workspace and --create-workspace are only valid with `clark scout`".into(),
            );
        }
        return Ok(scope);
    }
    let organization_id = scope
        .organization_id
        .as_deref()
        .ok_or_else(|| "Clark Scout authorization returned no organization".to_string())?;
    if requested_workspace_id.is_some_and(|value| uuid::Uuid::parse_str(value.trim()).is_err()) {
        return Err("--workspace must be a UUID shown by Clark Scout".into());
    }
    let client = cloud_client()?;
    let mut workspaces = list_scout_workspaces(&client, api_key, organization_id).await?;
    if let Some(name) = create_workspace_name {
        let name = name.trim();
        if name.is_empty() {
            return Err("--create-workspace requires a non-empty name".into());
        }
        let created = client
            .post(format!(
                "{}/cli/scout/workspaces",
                crate::auth::platform_api_base()
            ))
            .bearer_auth(api_key)
            .json(&serde_json::json!({
                "organizationId": organization_id,
                "stableKey": scout_workspace_key(name),
                "displayName": name,
            }))
            .send()
            .await
            .map_err(|error| format!("could not create Clark Scout workspace: {error}"))?;
        let status = created.status();
        let body = created.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "Clark Scout workspace creation failed ({status}): {}",
                body.chars().take(500).collect::<String>()
            ));
        }
        let created: ScoutWorkspace = serde_json::from_str(&body)
            .map_err(|error| format!("Clark returned an invalid Scout workspace: {error}"))?;
        workspaces.retain(|workspace| workspace.id != created.id);
        workspaces.insert(0, created);
    }
    let selected = if let Some(requested) = requested_workspace_id {
        workspaces
            .iter()
            .find(|workspace| workspace.id == requested.trim())
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Scout workspace {} is not active in organization {}",
                    requested.trim(),
                    organization_id
                )
            })?
    } else {
        match workspaces.as_slice() {
            [workspace] => workspace.clone(),
            [] => {
                return Err(
                    "Clark Scout needs a cartography workspace before it can start. No worker or model was started. Create one headlessly with `clark scout --create-workspace \"My systems\"`."
                        .into(),
                )
            }
            many => {
                let choices = many
                    .iter()
                    .map(|workspace| format!("  {}  {}", workspace.id, workspace.display_name))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Err(format!(
                    "More than one Scout workspace is available. Choose with `clark scout --workspace WORKSPACE_ID`:\n{choices}"
                ));
            }
        }
    };
    scope.workspace_id = Some(selected.id);
    Ok(scope)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecurityRegistration {
    repository: SecurityRepository,
    repository_policy: SecurityRepositoryPolicy,
}

#[derive(Debug, Deserialize)]
struct SecurityRepository {
    id: String,
}

#[derive(Debug, Deserialize)]
struct SecurityRepositoryPolicy {
    policy_id: String,
}

async fn register_security_repository(
    scope: &RuntimeScope,
    api_key: &str,
    cwd: &Path,
) -> Result<SecurityScope, String> {
    let organization_id = scope
        .organization_id
        .as_deref()
        .ok_or_else(|| "Clark Security authorization returned no organization".to_string())?;
    let repository = provider_local::inspect_repository(&provider_local::LocalExecutor, cwd)
        .await?
        .ok_or_else(|| {
            "Clark Security requires a Git repository. No worker or model was started.".to_string()
        })?;
    let client = cloud_client()?;
    let response = client
        .post(format!(
            "{}/cli/security/repositories",
            crate::auth::platform_api_base()
        ))
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "organizationId": organization_id,
            "fingerprint": repository.fingerprint,
            "vcs": repository.vcs,
            "canonicalRemote": repository.canonical_remote,
            "headOid": repository.head_oid,
            "currentBranch": repository.current_branch,
            "defaultBranch": repository.default_branch,
            "remotes": repository.remotes.iter().map(|remote| serde_json::json!({
                "name": remote.name,
                "url": remote.url,
                "canonical": remote.canonical,
            })).collect::<Vec<_>>(),
            "reportedCommitCount": repository.commit_count,
            "shallow": repository.shallow,
            "dirty": repository.dirty,
            "refsFingerprint": repository.refs_fingerprint,
        }))
        .send()
        .await
        .map_err(|error| format!("could not register Clark Security repository: {error}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Clark Security repository registration failed ({status}): {}. No worker or model was started.",
            body.chars().take(500).collect::<String>()
        ));
    }
    let registration: SecurityRegistration = serde_json::from_str(&body)
        .map_err(|error| format!("Clark returned an invalid Security registration: {error}"))?;
    Ok(SecurityScope {
        repository_id: registration.repository.id,
        policy_id: registration.repository_policy.policy_id,
        root: PathBuf::from(&repository.root),
        repository,
    })
}

fn cloud_client() -> Result<reqwest::Client, String> {
    clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(Duration::from_secs(30)),
        user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    })
    .map_err(|error| format!("could not initialize Clark network client: {error}"))
}

async fn list_scout_workspaces(
    client: &reqwest::Client,
    api_key: &str,
    organization_id: &str,
) -> Result<Vec<ScoutWorkspace>, String> {
    let mut url = url::Url::parse(&format!(
        "{}/cli/scout/workspaces",
        crate::auth::platform_api_base()
    ))
    .map_err(|error| format!("Clark Scout URL is invalid: {error}"))?;
    url.query_pairs_mut()
        .append_pair("organizationId", organization_id);
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|error| format!("could not list Clark Scout workspaces: {error}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Clark Scout workspace lookup failed ({status}): {}",
            body.chars().take(500).collect::<String>()
        ));
    }
    let list: ScoutWorkspaceList = serde_json::from_str(&body)
        .map_err(|error| format!("Clark returned invalid Scout workspaces: {error}"))?;
    Ok(list
        .workspaces
        .into_iter()
        .filter(|workspace| workspace.status == "active")
        .collect())
}

fn scout_workspace_key(name: &str) -> String {
    let mut key = name
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_ascii_lowercase() || character.is_ascii_digit() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while key.contains("--") {
        key = key.replace("--", "-");
    }
    let key = key.trim_matches('-');
    let key = if key.is_empty() { "workspace" } else { key };
    format!("cli-{}", key.chars().take(100).collect::<String>())
}

pub async fn load_context(
    api_key: &str,
    organization_id: Option<&str>,
) -> Result<CliContext, String> {
    if organization_id.is_some_and(|value| uuid::Uuid::parse_str(value.trim()).is_err()) {
        return Err("--organization must be a UUID shown by Clark".into());
    }
    let client = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(Duration::from_secs(20)),
        user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    })
    .map_err(|error| format!("could not initialize Clark network client: {error}"))?;
    let mut url = url::Url::parse(&format!("{}/cli/context", crate::auth::platform_api_base()))
        .map_err(|error| format!("Clark CLI context URL is invalid: {error}"))?;
    if let Some(organization_id) = organization_id {
        url.query_pairs_mut()
            .append_pair("organizationId", organization_id.trim());
    }
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|error| format!("could not verify Clark product access: {error}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Clark could not verify product access ({status}): {}",
            body.chars().take(500).collect::<String>()
        ));
    }
    let context: CliContext = serde_json::from_str(&body)
        .map_err(|error| format!("Clark returned invalid CLI access state: {error}"))?;
    if context.organization_selection.state == "ready"
        && context
            .organization_selection
            .selected_organization_id
            .is_none()
    {
        return Err("Clark returned an incomplete organization selection".into());
    }
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn context(state: &str, allowed: bool) -> CliContext {
        CliContext {
            account_id: Uuid::from_u128(99).to_string(),
            key: CliKey {
                name: "Clark CLI".into(),
                purpose: "clark_code_desktop".into(),
                cli_compatible: true,
            },
            code: ProductAccess {
                allowed: true,
                state: "ready".into(),
                organization_id: None,
            },
            specialists: ["scout", "security", "scientist", "rsi"]
                .into_iter()
                .map(|kind| {
                    (
                        kind.into(),
                        ProductAccess {
                            allowed,
                            state: state.into(),
                            organization_id: allowed.then(|| Uuid::from_u128(1).to_string()),
                        },
                    )
                })
                .collect(),
            organizations: Vec::new(),
            organization_selection: OrganizationSelection {
                state: if allowed { "ready" } else { "unavailable" }.into(),
                selected_organization_id: allowed.then(|| Uuid::from_u128(1).to_string()),
            },
            billing_url: "https://www.clarkchat.com/billing".into(),
            login_hint: "Run `clark login`.".into(),
        }
    }

    #[test]
    fn free_code_does_not_require_specialist_coverage() {
        let context = context("subscription_required", false);
        assert!(context.authorize(Workspace::Code).is_ok());
    }

    #[test]
    fn picker_statuses_put_included_code_before_four_paid_specialists() {
        let statuses = context("subscription_required", false)
            .product_statuses()
            .unwrap();
        assert_eq!(
            statuses
                .iter()
                .map(|status| (status.label, status.allowed))
                .collect::<Vec<_>>(),
            [
                ("Code", true),
                ("Scout", false),
                ("Security", false),
                ("Scientist", false),
                ("RSI", false),
            ]
        );
        assert!(statuses[1..]
            .iter()
            .all(|status| status.state == "subscription_required"));
    }

    #[test]
    fn scientist_fails_before_start_without_paid_coverage() {
        let context = context("subscription_required", false);
        let error = context.authorize(Workspace::ScientistDiscover).unwrap_err();
        assert!(error.contains("paid plans"));
        assert!(error.contains("No worker or model was started"));
    }

    #[test]
    fn general_platform_key_is_a_valid_cloud_credential_without_specialist_entitlement() {
        let mut context = context("subscription_required", false);
        context.key.purpose = "general".into();
        assert!(context.uses_metered_platform_key());
        assert!(context.authorize(Workspace::Code).is_ok());
        assert!(context
            .authorize(Workspace::ScientistDiscover)
            .unwrap_err()
            .contains("paid plans"));
    }

    #[test]
    fn paid_specialist_without_resolved_organization_fails_before_start() {
        let mut context = context("ready", true);
        context
            .specialists
            .get_mut("scientist")
            .expect("scientist access")
            .organization_id = None;
        let error = context.authorize(Workspace::ScientistDiscover).unwrap_err();
        assert!(error.contains("no cloud organization"));
        assert!(error.contains("No worker or model was started"));
    }

    #[test]
    fn scout_workspace_keys_are_portable_namespaces() {
        assert_eq!(
            scout_workspace_key("Production Systems"),
            "cli-production-systems"
        );
        assert_eq!(scout_workspace_key("  "), "cli-workspace");
    }
}
