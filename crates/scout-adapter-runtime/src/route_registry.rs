use scout_adapter_protocol::{AdapterPageRequest, NormalizedRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{RuntimeError, RuntimeResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AuthorityRule {
    GithubOrganization,
    GitlabGroup,
    AwsAccount,
    Global,
    GcpHierarchyParent,
    GcpOrganizationOrProject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CoverageScopeRule {
    Global,
    AwsRegion,
    GcpAssetScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IdentityAuthorityRule {
    Global,
    AwsAccount,
    GitlabInstance,
}

struct RegisteredRoute {
    adapter_id: &'static str,
    provider_namespace: &'static str,
    operation: &'static str,
    provider_resource_type: &'static str,
    allowed_projection: &'static [&'static str],
    coverage_resource_kind: &'static str,
    authority_rule: AuthorityRule,
    coverage_scope_rule: CoverageScopeRule,
    identity_authority_rule: IdentityAuthorityRule,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterRouteManifest {
    pub adapter_id: String,
    pub provider_namespace: String,
    pub operation: String,
    pub provider_resource_type: String,
    pub allowed_projection: Vec<String>,
    pub coverage_resource_kind: String,
    pub authority_rule: String,
    pub coverage_scope_rule: String,
    pub identity_authority_rule: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterCoverageManifest {
    pub runtime_protocol_version: u16,
    pub routes: Vec<AdapterRouteManifest>,
    pub manifest_sha256: String,
}

const ROUTES: &[RegisteredRoute] = &[
    RegisteredRoute {
        adapter_id: "clark/github-organization@1",
        provider_namespace: "github",
        operation: "list_organizations",
        provider_resource_type: "github.organization",
        allowed_projection: &["login"],
        coverage_resource_kind: "organization",
        authority_rule: AuthorityRule::Global,
        coverage_scope_rule: CoverageScopeRule::Global,
        identity_authority_rule: IdentityAuthorityRule::Global,
    },
    RegisteredRoute {
        adapter_id: "clark/github-organization@1",
        provider_namespace: "github",
        operation: "list_accessible_repositories",
        provider_resource_type: "github.repository",
        allowed_projection: &[
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
        ],
        coverage_resource_kind: "repository",
        authority_rule: AuthorityRule::Global,
        coverage_scope_rule: CoverageScopeRule::Global,
        identity_authority_rule: IdentityAuthorityRule::Global,
    },
    RegisteredRoute {
        adapter_id: "clark/github-organization@1",
        provider_namespace: "github",
        operation: "list_repositories",
        provider_resource_type: "github.repository",
        allowed_projection: &[
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
        ],
        coverage_resource_kind: "repository",
        authority_rule: AuthorityRule::GithubOrganization,
        coverage_scope_rule: CoverageScopeRule::Global,
        identity_authority_rule: IdentityAuthorityRule::Global,
    },
    RegisteredRoute {
        adapter_id: "clark/gitlab-group@1",
        provider_namespace: "gitlab",
        operation: "list_group_projects",
        provider_resource_type: "gitlab.project",
        allowed_projection: &[
            "name",
            "path",
            "path_with_namespace",
            "visibility",
            "archived",
            "default_branch",
            "web_url",
            "namespace_full_path",
            "topics",
        ],
        coverage_resource_kind: "repository",
        authority_rule: AuthorityRule::GitlabGroup,
        coverage_scope_rule: CoverageScopeRule::Global,
        identity_authority_rule: IdentityAuthorityRule::GitlabInstance,
    },
    RegisteredRoute {
        adapter_id: "clark/aws-enterprise@1",
        provider_namespace: "aws",
        operation: "list_organization_accounts",
        provider_resource_type: "aws.organizations.account",
        allowed_projection: &["id", "arn", "email", "name", "state", "joined_method"],
        coverage_resource_kind: "account",
        authority_rule: AuthorityRule::AwsAccount,
        coverage_scope_rule: CoverageScopeRule::Global,
        identity_authority_rule: IdentityAuthorityRule::Global,
    },
    RegisteredRoute {
        adapter_id: "clark/aws-enterprise@1",
        provider_namespace: "aws",
        operation: "list_resource_explorer_resources",
        provider_resource_type: "aws.resource_explorer.resource",
        allowed_projection: &[
            "arn",
            "owning_account_id",
            "region",
            "resource_type",
            "service",
            "url",
        ],
        coverage_resource_kind: "cloud_resource",
        authority_rule: AuthorityRule::AwsAccount,
        coverage_scope_rule: CoverageScopeRule::AwsRegion,
        identity_authority_rule: IdentityAuthorityRule::AwsAccount,
    },
    RegisteredRoute {
        adapter_id: "clark/gcp-enterprise@1",
        provider_namespace: "gcp",
        operation: "list_organizations",
        provider_resource_type: "gcp.organization",
        allowed_projection: &["name", "display_name", "directory_customer_id", "state"],
        coverage_resource_kind: "organization",
        authority_rule: AuthorityRule::Global,
        coverage_scope_rule: CoverageScopeRule::Global,
        identity_authority_rule: IdentityAuthorityRule::Global,
    },
    RegisteredRoute {
        adapter_id: "clark/gcp-enterprise@1",
        provider_namespace: "gcp",
        operation: "list_folders",
        provider_resource_type: "gcp.folder",
        allowed_projection: &["name", "display_name", "state", "parent"],
        coverage_resource_kind: "folder",
        authority_rule: AuthorityRule::GcpHierarchyParent,
        coverage_scope_rule: CoverageScopeRule::Global,
        identity_authority_rule: IdentityAuthorityRule::Global,
    },
    RegisteredRoute {
        adapter_id: "clark/gcp-enterprise@1",
        provider_namespace: "gcp",
        operation: "list_projects",
        provider_resource_type: "gcp.project",
        allowed_projection: &[
            "project_id",
            "project_number",
            "name",
            "lifecycle_state",
            "parent_type",
            "parent_id",
        ],
        coverage_resource_kind: "project",
        authority_rule: AuthorityRule::GcpHierarchyParent,
        coverage_scope_rule: CoverageScopeRule::Global,
        identity_authority_rule: IdentityAuthorityRule::Global,
    },
    RegisteredRoute {
        adapter_id: "clark/gcp-enterprise@1",
        provider_namespace: "gcp",
        operation: "search_all_resources",
        provider_resource_type: "gcp.cloud_asset.resource",
        allowed_projection: &[
            "name",
            "asset_type",
            "project",
            "organization",
            "location",
            "display_name",
            "state",
        ],
        coverage_resource_kind: "cloud_resource",
        authority_rule: AuthorityRule::GcpOrganizationOrProject,
        coverage_scope_rule: CoverageScopeRule::GcpAssetScope,
        identity_authority_rule: IdentityAuthorityRule::Global,
    },
];

pub fn adapter_coverage_manifest() -> AdapterCoverageManifest {
    let routes = ROUTES
        .iter()
        .map(|route| AdapterRouteManifest {
            adapter_id: route.adapter_id.to_owned(),
            provider_namespace: route.provider_namespace.to_owned(),
            operation: route.operation.to_owned(),
            provider_resource_type: route.provider_resource_type.to_owned(),
            allowed_projection: route
                .allowed_projection
                .iter()
                .map(|field| (*field).to_owned())
                .collect(),
            coverage_resource_kind: route.coverage_resource_kind.to_owned(),
            authority_rule: rule_name(route.authority_rule),
            coverage_scope_rule: scope_rule_name(route.coverage_scope_rule),
            identity_authority_rule: identity_rule_name(route.identity_authority_rule),
        })
        .collect::<Vec<_>>();
    let body = serde_json::to_vec(&(crate::types::RUNTIME_PROTOCOL_VERSION, &routes))
        .expect("registered route manifest is serializable");
    AdapterCoverageManifest {
        runtime_protocol_version: crate::types::RUNTIME_PROTOCOL_VERSION,
        routes,
        manifest_sha256: format!("{:x}", Sha256::digest(body)),
    }
}

pub(crate) fn validate_registered_route(request: &AdapterPageRequest) -> RuntimeResult<()> {
    request.query.validate()?;
    let route = registered_route(request)?;
    if request.query.provider_resource_type != route.provider_resource_type
        || !request.query.filters.is_empty()
        || !request
            .query
            .projection
            .iter()
            .all(|field| route.allowed_projection.contains(&field.as_str()))
        || request.coverage.resource_kind != route.coverage_resource_kind
    {
        return Err(RuntimeError::InvalidRequest);
    }
    validate_authority(route.authority_rule, &request.query.authority_scope)?;
    validate_coverage_scope(
        route.coverage_scope_rule,
        &request.query.authority_scope,
        &request.coverage.region_or_project,
    )
}

pub(crate) fn validate_registered_records(
    request: &AdapterPageRequest,
    records: &[NormalizedRecord],
) -> RuntimeResult<()> {
    let route = registered_route(request)?;
    for record in records {
        let identity_valid = match route.identity_authority_rule {
            IdentityAuthorityRule::Global => record.identity_authority_scope == "global",
            IdentityAuthorityRule::AwsAccount => {
                aws_account(&record.identity_authority_scope).is_some()
            }
            IdentityAuthorityRule::GitlabInstance => {
                gitlab_instance(&record.identity_authority_scope)
            }
        };
        if record.adapter_id.as_str() != route.adapter_id
            || record.provider_namespace != route.provider_namespace
            || record.provider_type != route.provider_resource_type
            || !identity_valid
        {
            return Err(RuntimeError::ProviderProtocol);
        }
    }
    Ok(())
}

fn registered_route(request: &AdapterPageRequest) -> RuntimeResult<&'static RegisteredRoute> {
    ROUTES
        .iter()
        .find(|route| {
            request.adapter_id.as_str() == route.adapter_id
                && request.query.operation == route.operation
        })
        .ok_or(RuntimeError::UnsupportedAdapter)
}

fn validate_authority(rule: AuthorityRule, authority: &str) -> RuntimeResult<()> {
    match rule {
        AuthorityRule::GithubOrganization => {
            crate::process_support::validate_github_name(authority)
        }
        AuthorityRule::GitlabGroup => crate::gitlab::validate_group_path(authority),
        AuthorityRule::AwsAccount => {
            if aws_account(authority).is_some() {
                Ok(())
            } else {
                Err(RuntimeError::InvalidRequest)
            }
        }
        AuthorityRule::Global if authority == "global" => Ok(()),
        AuthorityRule::GcpHierarchyParent
            if gcp_organization(authority).is_some() || gcp_folder(authority).is_some() =>
        {
            Ok(())
        }
        AuthorityRule::GcpOrganizationOrProject
            if gcp_organization(authority).is_some() || gcp_project(authority).is_some() =>
        {
            Ok(())
        }
        AuthorityRule::Global
        | AuthorityRule::GcpHierarchyParent
        | AuthorityRule::GcpOrganizationOrProject => Err(RuntimeError::InvalidRequest),
    }
}

fn rule_name(rule: AuthorityRule) -> String {
    serde_json::to_value(rule)
        .expect("authority rule serializes")
        .as_str()
        .expect("authority rule uses a string representation")
        .to_owned()
}

fn scope_rule_name(rule: CoverageScopeRule) -> String {
    serde_json::to_value(rule)
        .expect("coverage scope rule serializes")
        .as_str()
        .expect("coverage scope rule uses a string representation")
        .to_owned()
}

fn identity_rule_name(rule: IdentityAuthorityRule) -> String {
    serde_json::to_value(rule)
        .expect("identity authority rule serializes")
        .as_str()
        .expect("identity authority rule uses a string representation")
        .to_owned()
}

fn gitlab_instance(scope: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(scope) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/")
}

fn aws_account(scope: &str) -> Option<&str> {
    (scope.len() == 12 && scope.bytes().all(|byte| byte.is_ascii_digit())).then_some(scope)
}

fn validate_coverage_scope(
    rule: CoverageScopeRule,
    authority: &str,
    coverage_scope: &str,
) -> RuntimeResult<()> {
    match rule {
        CoverageScopeRule::Global if coverage_scope == "global" => Ok(()),
        CoverageScopeRule::AwsRegion => crate::process_support::validate_region(coverage_scope),
        CoverageScopeRule::GcpAssetScope if gcp_organization(authority).is_some() => {
            if coverage_scope == "global" {
                Ok(())
            } else {
                Err(RuntimeError::InvalidRequest)
            }
        }
        CoverageScopeRule::GcpAssetScope if gcp_project(authority).is_some() => {
            if coverage_scope == authority {
                Ok(())
            } else {
                Err(RuntimeError::InvalidRequest)
            }
        }
        CoverageScopeRule::Global | CoverageScopeRule::GcpAssetScope => {
            Err(RuntimeError::InvalidRequest)
        }
    }
}

fn gcp_organization(scope: &str) -> Option<&str> {
    scope
        .strip_prefix("organizations/")
        .filter(|id| !id.is_empty() && id.len() <= 32)
        .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn gcp_project(scope: &str) -> Option<&str> {
    scope
        .strip_prefix("projects/")
        .filter(|id| !id.is_empty() && id.len() <= 128)
        .filter(|id| {
            id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        })
}

fn gcp_folder(scope: &str) -> Option<&str> {
    scope
        .strip_prefix("folders/")
        .filter(|id| !id.is_empty() && id.len() <= 32)
        .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
}
