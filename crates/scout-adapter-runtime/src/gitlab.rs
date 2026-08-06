use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use reqwest::{Client, StatusCode, Url};
use scout_adapter_protocol::{
    AdapterId, AdapterPageRequest, AuthContextDescriptor, AuthSourceKind, NormalizedLink,
    NormalizedRecord, RedactionSummary, SafeFieldValue, TargetIdentity,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{RuntimeError, RuntimeResult};
use crate::process::ProcessRunner;
use crate::types::random_auth_handle;
use crate::vault::{ProviderCursor, StoredAuthRef};

const MAX_GITLAB_PAGE_SIZE: u32 = 100;
const MAX_GITLAB_PAGE: u32 = 1_000_000;
const PRIVATE_TOKEN: HeaderName = HeaderName::from_static("private-token");
const NEXT_PAGE: HeaderName = HeaderName::from_static("x-next-page");

pub(crate) fn adapter_id() -> AdapterId {
    AdapterId::new("clark/gitlab-group@1").expect("constant adapter id")
}

pub(crate) struct GitlabAdapter {
    client: Client,
    api_base: Url,
    identity_authority_scope: String,
    max_body_bytes: u64,
}

pub(crate) struct GitlabPage {
    pub(crate) records: Vec<NormalizedRecord>,
    pub(crate) next_cursor: Option<ProviderCursor>,
    pub(crate) redaction: RedactionSummary,
}

#[derive(Deserialize)]
struct GitlabUser {
    id: u64,
    username: String,
}

#[derive(Deserialize)]
struct GitlabGroup {
    id: u64,
    full_path: String,
}

#[derive(Deserialize)]
struct GitlabProject {
    id: u64,
    name: String,
    path: String,
    path_with_namespace: String,
    visibility: String,
    archived: bool,
    default_branch: Option<String>,
    web_url: Option<String>,
    namespace: GitlabNamespace,
    #[serde(default)]
    topics: BTreeSet<String>,
}

#[derive(Deserialize)]
struct GitlabNamespace {
    id: u64,
    full_path: String,
}

struct HttpPage {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl GitlabAdapter {
    pub(crate) fn new(
        api_base: Url,
        timeout: Duration,
        max_body_bytes: u64,
    ) -> RuntimeResult<Self> {
        if max_body_bytes == 0 || max_body_bytes > 8 * 1024 * 1024 {
            return Err(RuntimeError::InvalidRequest);
        }
        let client = clark_http::build_client(clark_http::ClientOptions {
            request_timeout: Some(timeout),
            ..Default::default()
        })
        .map_err(|_| RuntimeError::ProviderUnavailable)?;
        let identity_authority_scope = instance_scope(&api_base)?;
        Ok(Self {
            client,
            api_base,
            identity_authority_scope,
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
        let group_path = requested_scope.ok_or(RuntimeError::InvalidRequest)?;
        validate_group_path(group_path)?;
        let StoredAuthRef::GitlabEnvironment { variable } = reference else {
            return Err(RuntimeError::UnsupportedAdapter);
        };
        let token = runner
            .environment()
            .utf8(variable)
            .filter(|token| !token.is_empty())
            .ok_or(RuntimeError::AuthStale)?;
        let user = self
            .native_json::<GitlabUser>(&["user"], token, &[])
            .await?;
        let group = self
            .native_json::<GitlabGroup>(&["groups", group_path], token, &[])
            .await?;
        if user.id == 0
            || group.id == 0
            || group.full_path != group_path
            || validate_group_path(&group.full_path).is_err()
        {
            return Err(RuntimeError::AccessDenied);
        }
        let grant_digest = digest(
            format!(
                "gitlab\0{}\0{}\0{}",
                self.identity_authority_scope, group.id, user.id
            )
            .as_bytes(),
        );
        AuthContextDescriptor::new(
            random_auth_handle(),
            target.target_id.clone(),
            adapter_id(),
            "gitlab".to_owned(),
            group.full_path,
            format!("gitlab-user:{}:{}", user.id, user.username),
            AuthSourceKind::EnvironmentReference,
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
    ) -> RuntimeResult<GitlabPage> {
        validate_query(request)?;
        let page = match cursor {
            None => 1,
            Some(ProviderCursor::GitlabPage(page)) if page > 1 => page,
            Some(_) => return Err(RuntimeError::TargetMismatch),
        };
        let StoredAuthRef::GitlabEnvironment { variable } = reference else {
            return Err(RuntimeError::UnsupportedAdapter);
        };
        let token = runner
            .environment()
            .utf8(variable)
            .filter(|token| !token.is_empty())
            .ok_or(RuntimeError::AuthStale)?;
        let page_size = request
            .query
            .page_size
            .min(request.limits.max_records)
            .min(MAX_GITLAB_PAGE_SIZE);
        let response = self
            .native_get(
                &["groups", &request.query.authority_scope, "projects"],
                token,
                &[
                    ("include_subgroups", "true".to_owned()),
                    ("with_shared", "false".to_owned()),
                    ("order_by", "id".to_owned()),
                    ("sort", "asc".to_owned()),
                    ("per_page", page_size.to_string()),
                    ("page", page.to_string()),
                ],
            )
            .await?;
        classify_http(&response)?;
        let projects: Vec<GitlabProject> =
            serde_json::from_slice(&response.body).map_err(|_| RuntimeError::ProviderProtocol)?;
        if projects.len() > page_size as usize {
            return Err(RuntimeError::ProviderProtocol);
        }
        let source_records_seen = projects.len() as u64;
        let records = projects
            .into_iter()
            .map(|project| {
                normalize_project(request, project, self.identity_authority_scope.as_str())
            })
            .collect::<RuntimeResult<Vec<_>>>()?;
        let next_cursor = next_page(&response.headers, page)?.map(ProviderCursor::GitlabPage);
        Ok(GitlabPage {
            redaction: RedactionSummary {
                source_records_seen,
                records_emitted: records.len() as u64,
                fields_omitted: source_records_seen.saturating_mul(
                    (9_usize.saturating_sub(request.query.projection.len())) as u64,
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
        let token = HeaderValue::from_str(token).map_err(|_| RuntimeError::AuthStale)?;
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
            .header(PRIVATE_TOKEN, token)
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
    if request.adapter_id != adapter_id()
        || request.query.operation != "list_group_projects"
        || request.query.provider_resource_type != "gitlab.project"
        || !request.query.filters.is_empty()
    {
        return Err(RuntimeError::UnsupportedAdapter);
    }
    validate_group_path(&request.query.authority_scope)
}

fn normalize_project(
    request: &AdapterPageRequest,
    project: GitlabProject,
    identity_authority_scope: &str,
) -> RuntimeResult<NormalizedRecord> {
    if project.id == 0
        || project.namespace.id == 0
        || validate_group_path(&project.namespace.full_path).is_err()
        || !group_contains(&request.query.authority_scope, &project.namespace.full_path)
        || project.path_with_namespace
            != format!("{}/{}", project.namespace.full_path, project.path)
    {
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
    insert("name", Some(SafeFieldValue::Text(project.name)));
    insert("path", Some(SafeFieldValue::Text(project.path)));
    insert(
        "path_with_namespace",
        Some(SafeFieldValue::Text(project.path_with_namespace)),
    );
    insert("visibility", Some(SafeFieldValue::Text(project.visibility)));
    insert("archived", Some(SafeFieldValue::Boolean(project.archived)));
    insert(
        "default_branch",
        project.default_branch.map(SafeFieldValue::Text),
    );
    insert("web_url", project.web_url.map(SafeFieldValue::Text));
    insert(
        "namespace_full_path",
        Some(SafeFieldValue::Text(project.namespace.full_path.clone())),
    );
    insert("topics", Some(SafeFieldValue::TextSet(project.topics)));
    let links = BTreeSet::from([NormalizedLink {
        relationship_type: "owned_by".to_owned(),
        target_provider_namespace: "gitlab".to_owned(),
        target_provider_type: "gitlab.group".to_owned(),
        target_authority_scope: identity_authority_scope.to_owned(),
        target_native_id: format!("gitlab-group:{}", project.namespace.id),
        qualifier: None,
    }]);
    NormalizedRecord::new(
        request.adapter_id.clone(),
        "gitlab".to_owned(),
        request.query.provider_resource_type.clone(),
        identity_authority_scope.to_owned(),
        format!("gitlab-project:{}", project.id),
        Some("code_repository".to_owned()),
        BTreeSet::new(),
        fields,
        links,
    )
    .map_err(Into::into)
}

fn instance_scope(api_base: &Url) -> RuntimeResult<String> {
    if !matches!(api_base.scheme(), "http" | "https")
        || api_base.host_str().is_none()
        || !api_base.username().is_empty()
        || api_base.password().is_some()
    {
        return Err(RuntimeError::InvalidRequest);
    }
    let host = api_base.host_str().expect("checked above");
    let mut scope = format!("{}://{host}", api_base.scheme());
    if let Some(port) = api_base.port() {
        scope.push(':');
        scope.push_str(&port.to_string());
    }
    Ok(scope)
}

fn group_contains(root: &str, candidate: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn classify_http(response: &HttpPage) -> RuntimeResult<()> {
    match response.status {
        StatusCode::OK => Ok(()),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            Err(RuntimeError::AccessDenied)
        }
        StatusCode::TOO_MANY_REQUESTS => Err(RuntimeError::RateLimited),
        status if status.is_server_error() => Err(RuntimeError::ProviderUnavailable),
        _ => Err(RuntimeError::ProviderProtocol),
    }
}

fn next_page(headers: &HeaderMap, current: u32) -> RuntimeResult<Option<u32>> {
    let Some(value) = headers.get(&NEXT_PAGE) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| RuntimeError::ProviderProtocol)?;
    if value.is_empty() {
        return Ok(None);
    }
    let page = value
        .parse::<u32>()
        .map_err(|_| RuntimeError::ProviderProtocol)?;
    if page <= current || page > MAX_GITLAB_PAGE {
        return Err(RuntimeError::ProviderProtocol);
    }
    Ok(Some(page))
}

pub(crate) fn validate_group_path(value: &str) -> RuntimeResult<()> {
    if value.is_empty()
        || value.len() > 256
        || value.starts_with('/')
        || value.ends_with('/')
        || value.split('/').any(|segment| {
            segment.is_empty()
                || segment == "."
                || segment == ".."
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(RuntimeError::InvalidRequest);
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
