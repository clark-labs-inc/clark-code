use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{
    AdapterId, AdapterPageRequest, AuthContextDescriptor, AuthSourceKind, NormalizedLink,
    NormalizedRecord, RedactionSummary, SafeFieldValue, TargetIdentity,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{RuntimeError, RuntimeResult};
use crate::process::{AwsAuthMode, ProcessOutput, ProcessRunner};
use crate::types::random_auth_handle;
use crate::vault::{ProviderCursor, StoredAuthRef};

const MAX_AWS_PAGE: u32 = 999;

pub(crate) fn adapter_id() -> AdapterId {
    AdapterId::new("clark/aws-enterprise@1").expect("constant adapter id")
}

pub(crate) struct AwsPage {
    pub(crate) records: Vec<NormalizedRecord>,
    pub(crate) next_cursor: Option<ProviderCursor>,
    pub(crate) redaction: RedactionSummary,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CallerIdentity {
    account: String,
    arn: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AccountsPage {
    accounts: Vec<AwsAccount>,
    next_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AwsAccount {
    id: String,
    arn: String,
    email: Option<String>,
    name: Option<String>,
    state: Option<String>,
    joined_method: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ResourcesPage {
    resources: Vec<AwsResource>,
    next_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AwsResource {
    arn: String,
    owning_account_id: Option<String>,
    region: Option<String>,
    resource_type: Option<String>,
    service: Option<String>,
    url: Option<String>,
}

pub(crate) async fn census_profiles(runner: &ProcessRunner) -> RuntimeResult<Vec<String>> {
    if !runner.has_aws() {
        return Ok(Vec::new());
    }
    let output = runner.aws_profiles().await?;
    classify_cli(&output)?;
    let text = std::str::from_utf8(&output.stdout).map_err(|_| RuntimeError::ProviderProtocol)?;
    let mut profiles = BTreeSet::new();
    for line in text.lines() {
        let profile = line.trim();
        if profile.is_empty() {
            continue;
        }
        validate_profile(profile)?;
        if profiles.len() >= 256 {
            return Err(RuntimeError::BoundExceeded);
        }
        profiles.insert(profile.to_owned());
    }
    Ok(profiles.into_iter().collect())
}

pub(crate) async fn verify(
    reference: &StoredAuthRef,
    runner: &ProcessRunner,
    target: &TargetIdentity,
    requested_scope: Option<&str>,
    now_ms: u64,
) -> RuntimeResult<AuthContextDescriptor> {
    let auth = reference
        .aws_mode()
        .ok_or(RuntimeError::UnsupportedAdapter)?;
    ensure_auth_available(reference, runner)?;
    let identity: CallerIdentity = parse_cli_json(runner.aws_sts(&auth).await?)?;
    validate_account_id(&identity.account)?;
    if requested_scope.is_some_and(|scope| scope != identity.account) {
        return Err(RuntimeError::AccessDenied);
    }
    let source_kind = match reference {
        StoredAuthRef::AwsEnvironment => AuthSourceKind::EnvironmentReference,
        StoredAuthRef::AwsProfile { .. } => AuthSourceKind::CliProfile,
        StoredAuthRef::AwsWorkload => AuthSourceKind::WorkloadIdentity,
        _ => return Err(RuntimeError::UnsupportedAdapter),
    };
    let grant_digest = digest(format!("aws\0{}\0{}", identity.account, identity.arn).as_bytes());
    AuthContextDescriptor::new(
        random_auth_handle(),
        target.target_id.clone(),
        adapter_id(),
        "aws".to_owned(),
        identity.account,
        identity.arn,
        source_kind,
        grant_digest,
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
) -> RuntimeResult<AwsPage> {
    ensure_auth_available(reference, runner)?;
    let auth = reference
        .aws_mode()
        .ok_or(RuntimeError::UnsupportedAdapter)?;
    match request.query.operation.as_str() {
        "list_organization_accounts" => fetch_accounts(request, runner, &auth, cursor).await,
        "list_resource_explorer_resources" => fetch_resources(request, runner, &auth, cursor).await,
        _ => Err(RuntimeError::UnsupportedAdapter),
    }
}

async fn fetch_accounts(
    request: &AdapterPageRequest,
    runner: &ProcessRunner,
    auth: &AwsAuthMode,
    cursor: Option<ProviderCursor>,
) -> RuntimeResult<AwsPage> {
    validate_query(
        request,
        "list_organization_accounts",
        "aws.organizations.account",
        &["id", "arn", "email", "name", "state", "joined_method"],
    )?;
    let token = match cursor {
        None => None,
        Some(ProviderCursor::AwsOrganizations(token)) => Some(token),
        Some(_) => return Err(RuntimeError::TargetMismatch),
    };
    let page_size = request
        .query
        .page_size
        .min(request.limits.max_records)
        .min(MAX_AWS_PAGE);
    let page: AccountsPage = parse_cli_json(
        runner
            .aws_accounts(auth, page_size, token.as_deref())
            .await?,
    )?;
    if page.accounts.len() > page_size as usize {
        return Err(RuntimeError::ProviderProtocol);
    }
    let source_records_seen = page.accounts.len() as u64;
    let records = page
        .accounts
        .into_iter()
        .map(|account| normalize_account(request, account))
        .collect::<RuntimeResult<Vec<_>>>()?;
    Ok(AwsPage {
        next_cursor: page.next_token.map(ProviderCursor::AwsOrganizations),
        redaction: redaction(source_records_seen, records.len(), 6, request),
        records,
    })
}

async fn fetch_resources(
    request: &AdapterPageRequest,
    runner: &ProcessRunner,
    auth: &AwsAuthMode,
    cursor: Option<ProviderCursor>,
) -> RuntimeResult<AwsPage> {
    validate_query(
        request,
        "list_resource_explorer_resources",
        "aws.resource_explorer.resource",
        &[
            "arn",
            "owning_account_id",
            "region",
            "resource_type",
            "service",
            "url",
        ],
    )?;
    let token = match cursor {
        None => None,
        Some(ProviderCursor::AwsResources(token)) => Some(token),
        Some(_) => return Err(RuntimeError::TargetMismatch),
    };
    let page_size = request
        .query
        .page_size
        .min(request.limits.max_records)
        .min(MAX_AWS_PAGE);
    let page: ResourcesPage = parse_cli_json(
        runner
            .aws_resources(
                auth,
                &request.coverage.region_or_project,
                page_size,
                token.as_deref(),
            )
            .await?,
    )?;
    if page.resources.len() > page_size as usize {
        return Err(RuntimeError::ProviderProtocol);
    }
    let source_records_seen = page.resources.len() as u64;
    let records = page
        .resources
        .into_iter()
        .map(|resource| normalize_resource(request, resource))
        .collect::<RuntimeResult<Vec<_>>>()?;
    Ok(AwsPage {
        next_cursor: page.next_token.map(ProviderCursor::AwsResources),
        redaction: redaction(source_records_seen, records.len(), 6, request),
        records,
    })
}

fn normalize_account(
    request: &AdapterPageRequest,
    account: AwsAccount,
) -> RuntimeResult<NormalizedRecord> {
    validate_account_id(&account.id)?;
    let fields = projected_fields(
        request,
        [
            ("id", Some(SafeFieldValue::Text(account.id.clone()))),
            ("arn", Some(SafeFieldValue::Text(account.arn))),
            ("email", account.email.map(SafeFieldValue::Text)),
            ("name", account.name.map(SafeFieldValue::Text)),
            ("state", account.state.map(SafeFieldValue::Text)),
            (
                "joined_method",
                account.joined_method.map(SafeFieldValue::Text),
            ),
        ],
    );
    normalized(
        request,
        "global",
        format!("aws-account:{}", account.id),
        "cloud_account",
        fields,
        BTreeSet::new(),
    )
}

fn normalize_resource(
    request: &AdapterPageRequest,
    resource: AwsResource,
) -> RuntimeResult<NormalizedRecord> {
    if resource.arn.is_empty() {
        return Err(RuntimeError::ProviderProtocol);
    }
    if let Some(account_id) = &resource.owning_account_id {
        validate_account_id(account_id)?;
    }
    let native_id = resource.arn.clone();
    let identity_authority_scope = resource
        .owning_account_id
        .as_deref()
        .unwrap_or(&request.query.authority_scope)
        .to_owned();
    let links = resource
        .owning_account_id
        .as_ref()
        .map(|account_id| NormalizedLink {
            relationship_type: "owned_by".to_owned(),
            target_provider_namespace: "aws".to_owned(),
            target_provider_type: "aws.organizations.account".to_owned(),
            target_authority_scope: "global".to_owned(),
            target_native_id: format!("aws-account:{account_id}"),
            qualifier: None,
        })
        .into_iter()
        .collect();
    let fields = projected_fields(
        request,
        [
            ("arn", Some(SafeFieldValue::Text(resource.arn))),
            (
                "owning_account_id",
                resource.owning_account_id.map(SafeFieldValue::Text),
            ),
            ("region", resource.region.map(SafeFieldValue::Text)),
            (
                "resource_type",
                resource.resource_type.map(SafeFieldValue::Text),
            ),
            ("service", resource.service.map(SafeFieldValue::Text)),
            ("url", resource.url.map(SafeFieldValue::Text)),
        ],
    );
    normalized(
        request,
        &identity_authority_scope,
        native_id,
        "cloud_resource",
        fields,
        links,
    )
}

fn normalized(
    request: &AdapterPageRequest,
    identity_authority_scope: &str,
    native_id: String,
    semantic_kind: &str,
    fields: BTreeMap<String, SafeFieldValue>,
    links: BTreeSet<NormalizedLink>,
) -> RuntimeResult<NormalizedRecord> {
    NormalizedRecord::new(
        request.adapter_id.clone(),
        "aws".to_owned(),
        request.query.provider_resource_type.clone(),
        identity_authority_scope.to_owned(),
        native_id,
        Some(semantic_kind.to_owned()),
        BTreeSet::new(),
        fields,
        links,
    )
    .map_err(Into::into)
}

fn projected_fields<const N: usize>(
    request: &AdapterPageRequest,
    candidates: [(&str, Option<SafeFieldValue>); N],
) -> BTreeMap<String, SafeFieldValue> {
    candidates
        .into_iter()
        .filter(|(name, _)| request.query.projection.contains(*name))
        .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value)))
        .collect()
}

fn validate_query(
    request: &AdapterPageRequest,
    operation: &str,
    provider_type: &str,
    allowed_fields: &[&str],
) -> RuntimeResult<()> {
    request.query.validate()?;
    if request.adapter_id != adapter_id()
        || request.query.operation != operation
        || request.query.provider_resource_type != provider_type
        || !request.query.filters.is_empty()
        || !request
            .query
            .projection
            .iter()
            .all(|field| allowed_fields.contains(&field.as_str()))
    {
        return Err(RuntimeError::UnsupportedAdapter);
    }
    Ok(())
}

fn ensure_auth_available(reference: &StoredAuthRef, runner: &ProcessRunner) -> RuntimeResult<()> {
    let environment = runner.environment();
    let available = match reference {
        StoredAuthRef::AwsEnvironment => {
            environment.present("AWS_ACCESS_KEY_ID") && environment.present("AWS_SECRET_ACCESS_KEY")
        }
        StoredAuthRef::AwsProfile { profile } => validate_profile(profile).is_ok(),
        StoredAuthRef::AwsWorkload => {
            environment.present("AWS_WEB_IDENTITY_TOKEN_FILE")
                && environment.present("AWS_ROLE_ARN")
        }
        _ => false,
    };
    available.then_some(()).ok_or(RuntimeError::AuthStale)
}

fn parse_cli_json<T: DeserializeOwned>(output: ProcessOutput) -> RuntimeResult<T> {
    classify_cli(&output)?;
    serde_json::from_slice(&output.stdout).map_err(|_| RuntimeError::ProviderProtocol)
}

fn classify_cli(output: &ProcessOutput) -> RuntimeResult<()> {
    if output.success {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if stderr.contains("expiredtoken") || stderr.contains("invalidclienttokenid") {
        Err(RuntimeError::AuthStale)
    } else if stderr.contains("accessdenied") || stderr.contains("unauthorized") {
        Err(RuntimeError::AccessDenied)
    } else if stderr.contains("throttl") || stderr.contains("too many requests") {
        Err(RuntimeError::RateLimited)
    } else {
        Err(RuntimeError::ProviderUnavailable)
    }
}

fn redaction(
    source_records_seen: u64,
    emitted: usize,
    possible_fields: usize,
    request: &AdapterPageRequest,
) -> RedactionSummary {
    RedactionSummary {
        source_records_seen,
        records_emitted: emitted as u64,
        fields_omitted: source_records_seen
            .saturating_mul(possible_fields.saturating_sub(request.query.projection.len()) as u64),
        values_rejected: 0,
    }
}

fn validate_profile(profile: &str) -> RuntimeResult<()> {
    if profile.trim() != profile
        || profile.is_empty()
        || profile.len() > 256
        || profile
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(RuntimeError::ProviderProtocol);
    }
    Ok(())
}

fn validate_account_id(account: &str) -> RuntimeResult<()> {
    if account.len() != 12 || !account.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RuntimeError::ProviderProtocol);
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
