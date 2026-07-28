use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{HeaderMap, AUTHORIZATION, LINK, USER_AGENT};
use reqwest::{Client, StatusCode, Url};
use scout_adapter_protocol::{
    AdapterId, AdapterPageRequest, AuthContextDescriptor, AuthSourceKind, NormalizedRecord,
    RedactionSummary, SafeFieldValue, TargetIdentity,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{RuntimeError, RuntimeResult};
use crate::process::ProcessRunner;
use crate::types::random_auth_handle;
use crate::vault::{ProviderCursor, StoredAuthRef};

const MAX_GITHUB_PAGE: u32 = 100;

pub(crate) fn adapter_id() -> AdapterId {
    AdapterId::new("clark/github-organization@1").expect("constant adapter id")
}

pub(crate) struct GithubAdapter {
    client: Client,
    api_base: Url,
    max_body_bytes: u64,
}

pub(crate) struct GithubPage {
    pub(crate) records: Vec<NormalizedRecord>,
    pub(crate) next_cursor: Option<ProviderCursor>,
    pub(crate) redaction: RedactionSummary,
}

#[derive(Deserialize)]
struct GithubUser {
    id: u64,
    login: String,
}

#[derive(Deserialize)]
struct GithubOrganization {
    id: u64,
    login: String,
}

#[derive(Deserialize)]
struct GithubRepository {
    id: u64,
    name: String,
    full_name: String,
    private: bool,
    archived: bool,
    disabled: bool,
    fork: bool,
    default_branch: Option<String>,
    visibility: Option<String>,
    html_url: Option<String>,
    owner: Option<GithubOwner>,
}

#[derive(Deserialize)]
struct GithubOwner {
    login: String,
}

struct HttpPage {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl GithubAdapter {
    pub(crate) fn new(
        api_base: Url,
        timeout: Duration,
        max_body_bytes: u64,
    ) -> RuntimeResult<Self> {
        if max_body_bytes == 0 || max_body_bytes > 8 * 1024 * 1024 {
            return Err(RuntimeError::InvalidRequest);
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .timeout(timeout)
            .build()
            .map_err(|_| RuntimeError::ProviderUnavailable)?;
        Ok(Self {
            client,
            api_base,
            max_body_bytes,
        })
    }

    pub(crate) async fn verify(
        &self,
        reference: &StoredAuthRef,
        runner: &ProcessRunner,
        target: &TargetIdentity,
        requested_scope: Option<&str>,
        now_ms: u64,
    ) -> RuntimeResult<AuthContextDescriptor> {
        let organization = requested_scope.ok_or(RuntimeError::InvalidRequest)?;
        validate_organization(organization)?;
        let (user, verified_org, source_kind) = match reference {
            StoredAuthRef::GithubEnvironment { variable } => {
                let token = runner
                    .environment()
                    .utf8(variable)
                    .filter(|token| !token.is_empty())
                    .ok_or(RuntimeError::AuthStale)?;
                let user = self
                    .native_json::<GithubUser>(&["user"], token, &[])
                    .await?;
                let org = self
                    .native_json::<GithubOrganization>(&["orgs", organization], token, &[])
                    .await?;
                (user, org, AuthSourceKind::EnvironmentReference)
            }
            StoredAuthRef::GithubCli => {
                let user = parse_cli_json::<GithubUser>(runner.gh_user().await?)?;
                let org = parse_cli_json::<GithubOrganization>(runner.gh_org(organization).await?)?;
                (user, org, AuthSourceKind::CliProfile)
            }
            StoredAuthRef::AwsEnvironment
            | StoredAuthRef::AwsProfile { .. }
            | StoredAuthRef::AwsWorkload
            | StoredAuthRef::GitlabEnvironment { .. }
            | StoredAuthRef::GcpCli { .. } => return Err(RuntimeError::UnsupportedAdapter),
        };
        if !verified_org.login.eq_ignore_ascii_case(organization) || verified_org.id == 0 {
            return Err(RuntimeError::AccessDenied);
        }
        let grant_digest = digest(format!("github\0{}\0{}", verified_org.id, user.id).as_bytes());
        AuthContextDescriptor::new(
            random_auth_handle(),
            target.target_id.clone(),
            adapter_id(),
            "github".to_owned(),
            verified_org.login,
            format!("github-user:{}:{}", user.id, user.login),
            source_kind,
            grant_digest,
            now_ms,
            None,
        )
        .map_err(Into::into)
    }

    pub(crate) async fn fetch(
        &self,
        request: &AdapterPageRequest,
        reference: &StoredAuthRef,
        runner: &ProcessRunner,
        cursor: Option<ProviderCursor>,
    ) -> RuntimeResult<GithubPage> {
        validate_query(request)?;
        let page = match cursor {
            None => 1,
            Some(ProviderCursor::GithubPage(page)) if page > 1 => page,
            Some(_) => return Err(RuntimeError::TargetMismatch),
        };
        let page_size = request
            .query
            .page_size
            .min(request.limits.max_records)
            .min(MAX_GITHUB_PAGE);
        let (repositories, has_next) = match reference {
            StoredAuthRef::GithubEnvironment { variable } => {
                let token = runner
                    .environment()
                    .utf8(variable)
                    .filter(|token| !token.is_empty())
                    .ok_or(RuntimeError::AuthStale)?;
                let response = self
                    .native_get(
                        &["orgs", &request.query.authority_scope, "repos"],
                        token,
                        &[
                            ("per_page", page_size.to_string()),
                            ("page", page.to_string()),
                        ],
                    )
                    .await?;
                classify_http(&response)?;
                let has_next = has_next_link(&response.headers);
                let repositories = serde_json::from_slice(&response.body)
                    .map_err(|_| RuntimeError::ProviderProtocol)?;
                (repositories, has_next)
            }
            StoredAuthRef::GithubCli => {
                let output = runner
                    .gh_repositories(&request.query.authority_scope, page, page_size)
                    .await?;
                parse_gh_include(output)?
            }
            _ => return Err(RuntimeError::UnsupportedAdapter),
        };
        let repositories: Vec<GithubRepository> = repositories;
        if repositories.len() > page_size as usize {
            return Err(RuntimeError::ProviderProtocol);
        }
        let source_records_seen = repositories.len() as u64;
        let records = repositories
            .into_iter()
            .map(|repository| normalize_repository(request, repository))
            .collect::<RuntimeResult<Vec<_>>>()?;
        let next_cursor = if has_next {
            Some(ProviderCursor::GithubPage(
                page.checked_add(1).ok_or(RuntimeError::ProviderProtocol)?,
            ))
        } else {
            None
        };
        Ok(GithubPage {
            redaction: RedactionSummary {
                source_records_seen,
                records_emitted: records.len() as u64,
                fields_omitted: source_records_seen.saturating_mul(
                    (10_usize.saturating_sub(request.query.projection.len())) as u64,
                ),
                values_rejected: 0,
            },
            records,
            next_cursor,
        })
    }

    async fn native_json<T: DeserializeOwned>(
        &self,
        path: &[&str],
        token: &str,
        query: &[(&str, String)],
    ) -> RuntimeResult<T> {
        let response = self.native_get(path, token, query).await?;
        classify_http(&response)?;
        serde_json::from_slice(&response.body).map_err(|_| RuntimeError::ProviderProtocol)
    }

    async fn native_get(
        &self,
        path: &[&str],
        token: &str,
        query: &[(&str, String)],
    ) -> RuntimeResult<HttpPage> {
        let mut url = self.api_base.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| RuntimeError::InvalidRequest)?;
            segments.pop_if_empty();
            segments.extend(path.iter().copied());
        }
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(key, value)| (*key, value.as_str())));
        }
        let response = self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(USER_AGENT, "clark-scout-adapter-runtime")
            .send()
            .await
            .map_err(|_| RuntimeError::ProviderUnavailable)?;
        let status = response.status();
        let headers = response.headers().clone();
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| RuntimeError::ProviderUnavailable)?;
            if body.len().saturating_add(chunk.len()) as u64 > self.max_body_bytes {
                return Err(RuntimeError::BoundExceeded);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpPage {
            status,
            headers,
            body,
        })
    }
}

fn validate_query(request: &AdapterPageRequest) -> RuntimeResult<()> {
    request.query.validate()?;
    let allowed = BTreeSet::from([
        "name",
        "full_name",
        "visibility",
        "private",
        "archived",
        "disabled",
        "fork",
        "default_branch",
        "html_url",
        "owner_login",
    ]);
    if request.adapter_id != adapter_id()
        || request.query.operation != "list_repositories"
        || request.query.provider_resource_type != "github.repository"
        || !request.query.filters.is_empty()
        || !request
            .query
            .projection
            .iter()
            .all(|field| allowed.contains(field.as_str()))
    {
        return Err(RuntimeError::UnsupportedAdapter);
    }
    validate_organization(&request.query.authority_scope)
}

fn normalize_repository(
    request: &AdapterPageRequest,
    repository: GithubRepository,
) -> RuntimeResult<NormalizedRecord> {
    if repository.id == 0 {
        return Err(RuntimeError::ProviderProtocol);
    }
    let mut fields = BTreeMap::new();
    let mut insert = |name: &str, value: Option<SafeFieldValue>| {
        if request.query.projection.contains(name) {
            if let Some(value) = value {
                fields.insert(name.to_owned(), value);
            }
        }
    };
    insert("name", Some(SafeFieldValue::Text(repository.name)));
    insert(
        "full_name",
        Some(SafeFieldValue::Text(repository.full_name)),
    );
    insert(
        "visibility",
        repository.visibility.map(SafeFieldValue::Text),
    );
    insert("private", Some(SafeFieldValue::Boolean(repository.private)));
    insert(
        "archived",
        Some(SafeFieldValue::Boolean(repository.archived)),
    );
    insert(
        "disabled",
        Some(SafeFieldValue::Boolean(repository.disabled)),
    );
    insert("fork", Some(SafeFieldValue::Boolean(repository.fork)));
    insert(
        "default_branch",
        repository.default_branch.map(SafeFieldValue::Text),
    );
    insert("html_url", repository.html_url.map(SafeFieldValue::Text));
    insert(
        "owner_login",
        repository
            .owner
            .map(|owner| SafeFieldValue::Text(owner.login)),
    );
    NormalizedRecord::new(
        request.adapter_id.clone(),
        "github".to_owned(),
        request.query.provider_resource_type.clone(),
        "global".to_owned(),
        format!("github-repository:{}", repository.id),
        Some("code_repository".to_owned()),
        BTreeSet::new(),
        fields,
        BTreeSet::new(),
    )
    .map_err(Into::into)
}

fn parse_cli_json<T: DeserializeOwned>(output: crate::process::ProcessOutput) -> RuntimeResult<T> {
    classify_cli(&output)?;
    serde_json::from_slice(&output.stdout).map_err(|_| RuntimeError::ProviderProtocol)
}

fn parse_gh_include(
    output: crate::process::ProcessOutput,
) -> RuntimeResult<(Vec<GithubRepository>, bool)> {
    classify_cli(&output)?;
    let split = find_header_boundary(&output.stdout).ok_or(RuntimeError::ProviderProtocol)?;
    let (headers, body) = output.stdout.split_at(split);
    let body = &body[if output.stdout[split..].starts_with(b"\r\n\r\n") {
        4
    } else {
        2
    }..];
    let has_next = String::from_utf8_lossy(headers)
        .lines()
        .any(|line| line.to_ascii_lowercase().contains("rel=\"next\""));
    let repositories = serde_json::from_slice(body).map_err(|_| RuntimeError::ProviderProtocol)?;
    Ok((repositories, has_next))
}

fn find_header_boundary(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .or_else(|| bytes.windows(2).position(|window| window == b"\n\n"))
}

fn classify_cli(output: &crate::process::ProcessOutput) -> RuntimeResult<()> {
    if output.success {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if stderr.contains("rate limit") || stderr.contains("too many requests") {
        Err(RuntimeError::RateLimited)
    } else if stderr.contains("unauthorized")
        || stderr.contains("forbidden")
        || stderr.contains("authentication")
    {
        Err(RuntimeError::AccessDenied)
    } else {
        Err(RuntimeError::ProviderUnavailable)
    }
}

fn classify_http(response: &HttpPage) -> RuntimeResult<()> {
    match response.status {
        StatusCode::OK => Ok(()),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(RuntimeError::AccessDenied),
        StatusCode::TOO_MANY_REQUESTS => Err(RuntimeError::RateLimited),
        status if status.is_server_error() => Err(RuntimeError::ProviderUnavailable),
        _ => Err(RuntimeError::ProviderProtocol),
    }
}

fn has_next_link(headers: &HeaderMap) -> bool {
    headers
        .get_all(LINK)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.split(',').any(|part| part.contains("rel=\"next\"")))
}

fn validate_organization(value: &str) -> RuntimeResult<()> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RuntimeError::InvalidRequest);
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
