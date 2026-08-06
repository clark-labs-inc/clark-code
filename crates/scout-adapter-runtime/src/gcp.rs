use std::collections::BTreeSet;
use std::ffi::OsString;

use scout_adapter_protocol::{
    AdapterId, AdapterPageRequest, AuthContextDescriptor, AuthSourceKind, NormalizedRecord,
    RedactionSummary, TargetIdentity,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{RuntimeError, RuntimeResult};
use crate::process::{ProcessOutput, ProcessRunner};
use crate::types::random_auth_handle;
use crate::vault::{ProviderCursor, StoredAuthRef};

mod normalize;
mod scope;
use normalize::{finish_page, normalize_asset, normalize_folder, normalize_org, normalize_project};
use scope::{hierarchy_parent, validate_scope};

const MAX_GCP_PAGE: u32 = 499;
const ORG_CURSOR: u8 = 1;
const FOLDER_CURSOR: u8 = 2;
const PROJECT_CURSOR: u8 = 3;
const ASSET_CURSOR: u8 = 4;

pub(crate) fn adapter_id() -> AdapterId {
    AdapterId::new("clark/gcp-enterprise@1").expect("constant adapter id")
}

pub(crate) struct GcpPage {
    pub(crate) records: Vec<NormalizedRecord>,
    pub(crate) next_cursor: Option<ProviderCursor>,
    pub(crate) redaction: RedactionSummary,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveAccount {
    account: String,
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Organization {
    name: String,
    display_name: Option<String>,
    directory_customer_id: Option<String>,
    state: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Folder {
    name: String,
    display_name: Option<String>,
    state: Option<String>,
    parent: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    project_id: String,
    project_number: Option<String>,
    name: Option<String>,
    lifecycle_state: Option<String>,
    parent: Option<ResourceParent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceParent {
    #[serde(rename = "type")]
    parent_type: Option<String>,
    id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudAsset {
    name: String,
    asset_type: Option<String>,
    project: Option<String>,
    organization: Option<String>,
    location: Option<String>,
    display_name: Option<String>,
    state: Option<String>,
}

pub(crate) async fn census_accounts(runner: &ProcessRunner) -> RuntimeResult<Vec<String>> {
    if !runner.has_gcloud() {
        return Ok(Vec::new());
    }
    let accounts: Vec<ActiveAccount> = parse_json(
        runner
            .gcloud(
                strings([
                    "auth",
                    "list",
                    "--filter=status:ACTIVE",
                    "--format=json(account,status)",
                ]),
                None,
            )
            .await?,
    )?;
    let mut unique = BTreeSet::new();
    for account in accounts {
        validate_account(&account.account)?;
        if account
            .status
            .as_deref()
            .is_none_or(|status| status.eq_ignore_ascii_case("ACTIVE"))
        {
            unique.insert(account.account);
        }
        if unique.len() > 128 {
            return Err(RuntimeError::BoundExceeded);
        }
    }
    Ok(unique.into_iter().collect())
}

pub(crate) async fn verify(
    reference: &StoredAuthRef,
    runner: &ProcessRunner,
    target: &TargetIdentity,
    requested_scope: Option<&str>,
    now_ms: u64,
) -> RuntimeResult<AuthContextDescriptor> {
    let StoredAuthRef::GcpCli { account } = reference else {
        return Err(RuntimeError::UnsupportedAdapter);
    };
    validate_account(account)?;
    let active = census_accounts(runner).await?;
    if !active.iter().any(|candidate| candidate == account) {
        return Err(RuntimeError::AuthStale);
    }
    let scope = requested_scope.unwrap_or("global");
    validate_scope(scope)?;
    if let Some(id) = scope.strip_prefix("organizations/") {
        let _: Organization = parse_json(
            runner
                .gcloud(
                    vec![
                        "organizations".into(),
                        "describe".into(),
                        id.into(),
                        "--format=json(name,displayName,state)".into(),
                    ],
                    Some(account),
                )
                .await?,
        )?;
    } else if let Some(id) = scope.strip_prefix("folders/") {
        let _: Folder = parse_json(
            runner
                .gcloud(
                    vec![
                        "resource-manager".into(),
                        "folders".into(),
                        "describe".into(),
                        id.into(),
                        "--format=json(name,displayName,state,parent)".into(),
                    ],
                    Some(account),
                )
                .await?,
        )?;
    } else if let Some(id) = scope.strip_prefix("projects/") {
        let _: Project = parse_json(
            runner
                .gcloud(
                    vec![
                        "projects".into(),
                        "describe".into(),
                        id.into(),
                        "--format=json(projectId,projectNumber,name,lifecycleState,parent)".into(),
                    ],
                    Some(account),
                )
                .await?,
        )?;
    }
    AuthContextDescriptor::new(
        random_auth_handle(),
        target.target_id.clone(),
        adapter_id(),
        "gcp".to_owned(),
        scope.to_owned(),
        account.clone(),
        AuthSourceKind::CliProfile,
        digest(format!("gcp\0{account}\0{scope}").as_bytes()),
        now_ms,
        None,
    )
    .map_err(Into::into)
}

pub(crate) async fn fetch(
    request: &AdapterPageRequest,
    reference: &StoredAuthRef,
    runner: &ProcessRunner,
    cursor: Option<ProviderCursor>,
) -> RuntimeResult<GcpPage> {
    let StoredAuthRef::GcpCli { account } = reference else {
        return Err(RuntimeError::UnsupportedAdapter);
    };
    validate_account(account)?;
    let page_size = request
        .query
        .page_size
        .min(request.limits.max_records)
        .min(MAX_GCP_PAGE);
    let fetch_limit = page_size
        .checked_add(1)
        .ok_or(RuntimeError::BoundExceeded)?;
    match request.query.operation.as_str() {
        "list_organizations" => {
            validate_query(
                request,
                "global",
                "gcp.organization",
                &["name", "display_name", "directory_customer_id", "state"],
            )?;
            let after = cursor_key(cursor, ORG_CURSOR)?;
            let mut argv = strings([
                "organizations",
                "list",
                "--sort-by=name",
                &format!("--limit={fetch_limit}"),
                "--format=json(name,displayName,directoryCustomerId,state)",
            ]);
            push_after_filter(&mut argv, "name", after.as_deref())?;
            let rows: Vec<Organization> = parse_json(runner.gcloud(argv, Some(account)).await?)?;
            finish_page(
                request,
                rows,
                page_size,
                ORG_CURSOR,
                |row| row.name.clone(),
                normalize_org,
            )
        }
        "list_projects" => {
            let (parent_type, parent_id) = hierarchy_parent(&request.query.authority_scope)?;
            validate_query(
                request,
                &request.query.authority_scope,
                "gcp.project",
                &[
                    "project_id",
                    "project_number",
                    "name",
                    "lifecycle_state",
                    "parent_type",
                    "parent_id",
                ],
            )?;
            let after = cursor_key(cursor, PROJECT_CURSOR)?;
            let mut filter = format!("parent.id={parent_id} AND parent.type={parent_type}");
            if let Some(after) = after {
                validate_key(&after)?;
                filter.push_str(&format!(" AND projectId>{after}"));
            }
            let rows: Vec<Project> = parse_json(
                runner
                    .gcloud(
                        vec![
                            "projects".into(),
                            "list".into(),
                            "--sort-by=projectId".into(),
                            format!("--limit={fetch_limit}").into(),
                            format!("--filter={filter}").into(),
                            "--format=json(projectId,projectNumber,name,lifecycleState,parent)"
                                .into(),
                        ],
                        Some(account),
                    )
                    .await?,
            )?;
            finish_page(
                request,
                rows,
                page_size,
                PROJECT_CURSOR,
                |row| row.project_id.clone(),
                normalize_project,
            )
        }
        "list_folders" => {
            let (parent_type, parent_id) = hierarchy_parent(&request.query.authority_scope)?;
            validate_query(
                request,
                &request.query.authority_scope,
                "gcp.folder",
                &["name", "display_name", "state", "parent"],
            )?;
            let after = cursor_key(cursor, FOLDER_CURSOR)?;
            let parent_flag = if parent_type == "organization" {
                format!("--organization={parent_id}")
            } else {
                format!("--folder={parent_id}")
            };
            let mut argv = vec![
                "resource-manager".into(),
                "folders".into(),
                "list".into(),
                parent_flag.into(),
                "--sort-by=name".into(),
                format!("--limit={fetch_limit}").into(),
                "--format=json(name,displayName,state,parent)".into(),
            ];
            push_after_filter(&mut argv, "name", after.as_deref())?;
            let rows: Vec<Folder> = parse_json(runner.gcloud(argv, Some(account)).await?)?;
            finish_page(
                request,
                rows,
                page_size,
                FOLDER_CURSOR,
                |row| row.name.clone(),
                normalize_folder,
            )
        }
        "search_all_resources" => {
            validate_scope(&request.query.authority_scope)?;
            if request.query.authority_scope == "global" {
                return Err(RuntimeError::InvalidRequest);
            }
            validate_query(
                request,
                &request.query.authority_scope,
                "gcp.cloud_asset.resource",
                &[
                    "name",
                    "asset_type",
                    "project",
                    "organization",
                    "location",
                    "display_name",
                    "state",
                ],
            )?;
            let after = cursor_key(cursor, ASSET_CURSOR)?;
            let mut argv = vec![
                "asset".into(),
                "search-all-resources".into(),
                format!("--scope={}", request.query.authority_scope).into(),
                "--sort-by=name".into(),
                format!("--limit={fetch_limit}").into(),
                format!("--page-size={fetch_limit}").into(),
                "--format=json(name,assetType,project,organization,location,displayName,state)"
                    .into(),
            ];
            push_after_filter(&mut argv, "name", after.as_deref())?;
            let rows: Vec<CloudAsset> = parse_json(runner.gcloud(argv, Some(account)).await?)?;
            finish_page(
                request,
                rows,
                page_size,
                ASSET_CURSOR,
                |row| row.name.clone(),
                normalize_asset,
            )
        }
        _ => Err(RuntimeError::UnsupportedAdapter),
    }
}

fn validate_query(
    request: &AdapterPageRequest,
    authority: &str,
    provider_type: &str,
    fields: &[&str],
) -> RuntimeResult<()> {
    request.query.validate()?;
    if request.adapter_id != adapter_id()
        || request.query.authority_scope != authority
        || request.query.provider_resource_type != provider_type
        || !request.query.filters.is_empty()
        || !request
            .query
            .projection
            .iter()
            .all(|field| fields.contains(&field.as_str()))
    {
        return Err(RuntimeError::UnsupportedAdapter);
    }
    Ok(())
}

fn cursor_key(cursor: Option<ProviderCursor>, expected: u8) -> RuntimeResult<Option<String>> {
    match cursor {
        None => Ok(None),
        Some(ProviderCursor::GcpAfterKey { operation, key }) if operation == expected => {
            validate_key(&key)?;
            Ok(Some(key))
        }
        Some(_) => Err(RuntimeError::TargetMismatch),
    }
}

fn push_after_filter(
    argv: &mut Vec<OsString>,
    field: &str,
    after: Option<&str>,
) -> RuntimeResult<()> {
    if let Some(after) = after {
        validate_key(after)?;
        argv.push(format!("--filter={field}>{after}").into());
    }
    Ok(())
}

fn validate_account(account: &str) -> RuntimeResult<()> {
    if account.len() > 320
        || !account.contains('@')
        || account
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(RuntimeError::ProviderProtocol);
    }
    Ok(())
}

fn validate_key(key: &str) -> RuntimeResult<()> {
    if key.is_empty()
        || key.len() > 2_048
        || key
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(RuntimeError::ProviderProtocol);
    }
    Ok(())
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(Into::into).collect()
}

fn parse_json<T: DeserializeOwned>(output: ProcessOutput) -> RuntimeResult<T> {
    classify_cli(&output)?;
    serde_json::from_slice(&output.stdout).map_err(|_| RuntimeError::ProviderProtocol)
}

fn classify_cli(output: &ProcessOutput) -> RuntimeResult<()> {
    if output.success {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if stderr.contains("permission") || stderr.contains("forbidden") {
        Err(RuntimeError::AccessDenied)
    } else if stderr.contains("credential")
        || stderr.contains("unauthenticated")
        || stderr.contains("login")
    {
        Err(RuntimeError::AuthStale)
    } else if stderr.contains("quota") || stderr.contains("rate") {
        Err(RuntimeError::RateLimited)
    } else {
        Err(RuntimeError::ProviderUnavailable)
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
