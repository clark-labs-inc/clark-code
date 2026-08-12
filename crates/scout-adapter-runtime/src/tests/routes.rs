use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{
    AdapterId, AdapterPageLimits, AdapterPageRequest, AdapterQuery, AuthContextDescriptor,
    AuthContextHandle, AuthSourceKind, CoverageBinding, NormalizedRecord, RequestId,
    SafeFieldValue, TargetIdentity,
};

use crate::route_registry::{validate_registered_records, validate_registered_route};

struct RouteFixture<'a> {
    adapter: &'a str,
    provider: &'a str,
    authority: &'a str,
    operation: &'a str,
    provider_type: &'a str,
    resource_kind: &'a str,
    coverage_scope: &'a str,
    projection: &'a [&'a str],
}

fn digest(byte: u8) -> String {
    char::from(byte).to_string().repeat(64)
}

fn request(fixture: &RouteFixture<'_>) -> AdapterPageRequest {
    let adapter = AdapterId::new(fixture.adapter).unwrap();
    let target = TargetIdentity::new(
        digest(b'1'),
        digest(b'2'),
        digest(b'3'),
        digest(b'4'),
        "linux".to_owned(),
        "x86_64".to_owned(),
    )
    .unwrap();
    let auth = AuthContextDescriptor::new(
        AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000001").unwrap(),
        target.target_id.clone(),
        adapter.clone(),
        fixture.provider.to_owned(),
        fixture.authority.to_owned(),
        "principal:route-test".to_owned(),
        AuthSourceKind::CliProfile,
        digest(b'5'),
        900,
        Some(10_000),
    )
    .unwrap();
    AdapterPageRequest {
        protocol_version: scout_adapter_protocol::ADAPTER_PROTOCOL_VERSION,
        request_id: RequestId::new("request:00000000-0000-4000-8000-000000000002").unwrap(),
        target_id: target.target_id.clone(),
        target_identity_sha256: target.fingerprint_sha256().unwrap(),
        adapter_id: adapter.clone(),
        auth_context_handle: auth.handle.clone(),
        auth_context_id: auth.context_id.clone(),
        coverage: CoverageBinding {
            enterprise_id: "enterprise:test".to_owned(),
            charter_id: "charter:test".to_owned(),
            discovery_epoch: 1,
            sequence: 1,
            adapter_id: adapter,
            auth_context_id: auth.context_id,
            tenant: fixture.authority.to_owned(),
            region_or_project: fixture.coverage_scope.to_owned(),
            resource_kind: fixture.resource_kind.to_owned(),
        },
        query: AdapterQuery {
            operation: fixture.operation.to_owned(),
            authority_scope: fixture.authority.to_owned(),
            provider_resource_type: fixture.provider_type.to_owned(),
            filters: BTreeMap::new(),
            projection: fixture
                .projection
                .iter()
                .map(|field| (*field).to_owned())
                .collect::<BTreeSet<_>>(),
            page_size: 100,
        },
        page_ordinal: 0,
        cursor_handle: None,
        limits: AdapterPageLimits {
            max_records: 100,
            max_response_bytes: 1_000_000,
            max_duration_ms: 30_000,
        },
        requested_at_ms: 1_000,
    }
}

fn routes() -> Vec<RouteFixture<'static>> {
    vec![
        RouteFixture {
            adapter: "clark/github-organization@1",
            provider: "github",
            authority: "global",
            operation: "list_organizations",
            provider_type: "github.organization",
            resource_kind: "organization",
            coverage_scope: "global",
            projection: &["login"],
        },
        RouteFixture {
            adapter: "clark/github-organization@1",
            provider: "github",
            authority: "global",
            operation: "list_accessible_repositories",
            provider_type: "github.repository",
            resource_kind: "repository",
            coverage_scope: "global",
            projection: &["name", "owner_login"],
        },
        RouteFixture {
            adapter: "clark/github-organization@1",
            provider: "github",
            authority: "acme",
            operation: "list_repositories",
            provider_type: "github.repository",
            resource_kind: "repository",
            coverage_scope: "global",
            projection: &["name", "owner_login"],
        },
        RouteFixture {
            adapter: "clark/gitlab-group@1",
            provider: "gitlab",
            authority: "acme/platform",
            operation: "list_group_projects",
            provider_type: "gitlab.project",
            resource_kind: "repository",
            coverage_scope: "global",
            projection: &["name", "path_with_namespace", "namespace_full_path"],
        },
        RouteFixture {
            adapter: "clark/aws-enterprise@1",
            provider: "aws",
            authority: "123456789012",
            operation: "list_organization_accounts",
            provider_type: "aws.organizations.account",
            resource_kind: "account",
            coverage_scope: "global",
            projection: &["id", "name"],
        },
        RouteFixture {
            adapter: "clark/aws-enterprise@1",
            provider: "aws",
            authority: "123456789012",
            operation: "list_resource_explorer_resources",
            provider_type: "aws.resource_explorer.resource",
            resource_kind: "cloud_resource",
            coverage_scope: "us-east-1",
            projection: &["arn", "owning_account_id", "region"],
        },
        RouteFixture {
            adapter: "clark/gcp-enterprise@1",
            provider: "gcp",
            authority: "global",
            operation: "list_organizations",
            provider_type: "gcp.organization",
            resource_kind: "organization",
            coverage_scope: "global",
            projection: &["name", "state"],
        },
        RouteFixture {
            adapter: "clark/gcp-enterprise@1",
            provider: "gcp",
            authority: "organizations/123",
            operation: "list_folders",
            provider_type: "gcp.folder",
            resource_kind: "folder",
            coverage_scope: "global",
            projection: &["name", "display_name", "parent"],
        },
        RouteFixture {
            adapter: "clark/gcp-enterprise@1",
            provider: "gcp",
            authority: "folders/7",
            operation: "list_projects",
            provider_type: "gcp.project",
            resource_kind: "project",
            coverage_scope: "global",
            projection: &["project_id", "project_number", "parent_id"],
        },
        RouteFixture {
            adapter: "clark/gcp-enterprise@1",
            provider: "gcp",
            authority: "organizations/123",
            operation: "search_all_resources",
            provider_type: "gcp.cloud_asset.resource",
            resource_kind: "cloud_resource",
            coverage_scope: "global",
            projection: &["name", "asset_type", "project"],
        },
        RouteFixture {
            adapter: "clark/gcp-enterprise@1",
            provider: "gcp",
            authority: "projects/acme-prod",
            operation: "search_all_resources",
            provider_type: "gcp.cloud_asset.resource",
            resource_kind: "cloud_resource",
            coverage_scope: "projects/acme-prod",
            projection: &["name", "asset_type", "project"],
        },
    ]
}

#[test]
fn every_registered_route_accepts_its_canonical_binding() {
    for fixture in routes() {
        let request = request(&fixture);
        assert!(
            validate_registered_route(&request).is_ok(),
            "{} should be registered",
            fixture.operation
        );
    }
}

#[test]
fn coverage_manifest_is_complete_deterministic_and_machine_readable() {
    let first = crate::adapter_coverage_manifest();
    let second = crate::adapter_coverage_manifest();
    assert_eq!(first, second);
    let expected_routes = routes()
        .into_iter()
        .map(|route| (route.adapter, route.operation))
        .collect::<BTreeSet<_>>();
    assert_eq!(first.routes.len(), expected_routes.len());
    assert_eq!(first.manifest_sha256.len(), 64);
    assert!(first.routes.iter().any(|route| {
        route.adapter_id == "clark/gitlab-group@1"
            && route.operation == "list_group_projects"
            && route.authority_rule == "gitlab_group"
            && route.identity_authority_rule == "gitlab_instance"
    }));
    let json = serde_json::to_string(&first).unwrap();
    assert!(!json.contains("token"));
    assert!(!json.contains("cursor"));
}

#[test]
fn every_registered_route_rejects_a_mislabeled_coverage_kind() {
    for fixture in routes() {
        let mut request = request(&fixture);
        request.coverage.resource_kind = "mislabeled".to_owned();
        assert!(
            validate_registered_route(&request).is_err(),
            "{} accepted a false coverage kind",
            fixture.operation
        );
    }
}

#[test]
fn route_binding_rejects_wrong_type_projection_filter_and_scope() {
    let fixture = routes()
        .into_iter()
        .find(|fixture| fixture.operation == "list_repositories")
        .unwrap();

    let mut wrong_type = request(&fixture);
    wrong_type.query.provider_resource_type = "github.organization".to_owned();
    assert!(validate_registered_route(&wrong_type).is_err());

    let mut wrong_projection = request(&fixture);
    wrong_projection.query.projection.insert("token".to_owned());
    assert!(validate_registered_route(&wrong_projection).is_err());

    let mut filtered = request(&fixture);
    filtered
        .query
        .filters
        .insert("name".to_owned(), SafeFieldValue::Text("clark".to_owned()));
    assert!(validate_registered_route(&filtered).is_err());

    let mut wrong_scope = request(&fixture);
    wrong_scope.coverage.region_or_project = "us-east-1".to_owned();
    assert!(validate_registered_route(&wrong_scope).is_err());
}

#[test]
fn project_scoped_gcp_assets_require_the_exact_project_coverage_scope() {
    let fixture = routes()
        .into_iter()
        .find(|fixture| fixture.authority == "projects/acme-prod")
        .unwrap();
    let mut request = request(&fixture);
    request.coverage.region_or_project = "global".to_owned();
    assert!(validate_registered_route(&request).is_err());
}

#[test]
fn route_registry_rejects_a_noncanonical_record_identity_authority() {
    let fixture = routes()
        .into_iter()
        .find(|fixture| fixture.operation == "list_repositories")
        .unwrap();
    let request = request(&fixture);
    let record = NormalizedRecord::new(
        request.adapter_id.clone(),
        "github".to_owned(),
        request.query.provider_resource_type.clone(),
        request.query.authority_scope.clone(),
        "github-repository:42".to_owned(),
        Some("code_repository".to_owned()),
        BTreeSet::new(),
        BTreeMap::new(),
        BTreeSet::new(),
    )
    .unwrap();
    assert!(validate_registered_records(&request, &[record]).is_err());
}
