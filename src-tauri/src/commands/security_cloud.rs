use std::path::Path;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use super::cloud::read_json_or_err;
use super::cloud_authority::{clark_http_client, require_cloud_access};
use crate::state::AppState;

#[path = "security_cloud/client.rs"]
mod client;
#[path = "security_cloud/evidence.rs"]
mod evidence;
#[path = "security_cloud/identity.rs"]
mod identity;
#[path = "security_cloud/ingest.rs"]
mod ingest;
#[path = "security_cloud/model.rs"]
mod model;
#[path = "security_cloud/poc.rs"]
mod poc;

pub use model::SecurityCloudSyncResult;

#[tauri::command]
pub async fn desktop_security_organizations(
    endpoint: String,
    token: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = require_cloud_access(state.inner(), &endpoint, &token).await?;
    let response = clark_http_client()?
        .get(format!("{}/api/orgs", access.rest_base))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("Clark Security organization request failed: {error}"))?;
    read_json_or_err(response, "Clark Security organizations").await
}

#[tauri::command]
pub async fn desktop_security_register_repository(
    endpoint: String,
    token: String,
    organization_id: String,
    cwd: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let organization_id = Uuid::parse_str(organization_id.trim())
        .map_err(|_| "Clark Security organization id is invalid".to_string())?;
    let access = require_cloud_access(state.inner(), &endpoint, &token).await?;
    let repository =
        provider_local::inspect_repository(&provider_local::LocalExecutor, Path::new(cwd.trim()))
            .await?
            .ok_or_else(|| "Clark Security requires a Git repository".to_string())?;
    let body = registration_body(organization_id, &repository);
    let response = clark_http_client()?
        .post(format!(
            "{}/api/orgs/{organization_id}/security/repositories/register",
            access.rest_base
        ))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Clark Security repository registration failed: {error}"))?;
    read_json_or_err(response, "Clark Security repository registration").await
}

#[tauri::command]
pub async fn desktop_security_sync_scans(
    endpoint: String,
    token: String,
    api_key: String,
    organization_id: String,
    repository_id: String,
    policy_id: Option<String>,
    cwd: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SecurityCloudSyncResult, String> {
    let organization_id = Uuid::parse_str(organization_id.trim())
        .map_err(|_| "Clark Security organization id is invalid".to_string())?;
    let repository_id = Uuid::parse_str(repository_id.trim())
        .map_err(|_| "Clark Security repository id is invalid".to_string())?;
    let policy_id = policy_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| "Clark Security policy id is invalid".to_string())?;
    let access = require_cloud_access(state.inner(), &endpoint, &token).await?;
    let repository =
        provider_local::inspect_repository(&provider_local::LocalExecutor, Path::new(cwd.trim()))
            .await?
            .ok_or_else(|| "Clark Security requires a Git repository".to_string())?;
    let root = Path::new(&repository.root).to_path_buf();
    let identity_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("resolve Clark Security identity directory: {error}"))?
        .join("security-identities");
    ingest::sync_security_scans(ingest::SecuritySyncRequest {
        rest_base: access.rest_base,
        api_key,
        owner_scope: access.owner_scope,
        organization_id: organization_id.to_string(),
        repository_id: repository_id.to_string(),
        policy_id: policy_id.map(|id| id.to_string()),
        root,
        identity_root,
        repository,
        http: clark_http_client()?,
    })
    .await
}

fn registration_body(
    organization_id: Uuid,
    repository: &provider_local::RepositoryIdentity,
) -> Value {
    json!({
        "organizationId": organization_id,
        "fingerprint": repository.fingerprint,
        "vcs": repository.vcs,
        "canonicalRemote": repository.canonical_remote,
        "headOid": repository.head_oid,
        "currentBranch": repository.current_branch,
        "defaultBranch": repository.default_branch,
        "remotes": repository.remotes.iter().map(|remote| json!({
            "name": remote.name,
            "url": remote.url,
            "canonical": remote.canonical,
        })).collect::<Vec<_>>(),
        "reportedCommitCount": repository.commit_count,
        "shallow": repository.shallow,
        "dirty": repository.dirty,
        "refsFingerprint": repository.refs_fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_uses_canonical_cloud_field_names_without_local_paths() {
        let repository = provider_local::RepositoryIdentity {
            fingerprint: format!("git:{}", "a".repeat(64)),
            vcs: "git".into(),
            root: "/private/source/service".into(),
            head_oid: Some("b".repeat(40)),
            current_branch: Some("feature/security".into()),
            default_branch: Some("main".into()),
            canonical_remote: Some("github.com/example/service".into()),
            remotes: vec![provider_local::RepositoryRemote {
                name: "origin".into(),
                url: "https://github.com/example/service.git".into(),
                canonical: "github.com/example/service".into(),
            }],
            commit_count: 7,
            shallow: false,
            dirty: true,
            refs_fingerprint: "c".repeat(64),
        };
        let body = registration_body(Uuid::nil(), &repository);
        assert_eq!(body["organizationId"], Uuid::nil().to_string());
        assert_eq!(body["reportedCommitCount"], 7);
        assert_eq!(body["canonicalRemote"], "github.com/example/service");
        assert!(body.get("root").is_none());
        assert!(!body.to_string().contains("/private/source/service"));
    }
}
