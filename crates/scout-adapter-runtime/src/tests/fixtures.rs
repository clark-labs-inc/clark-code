use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Url;
use scout_adapter_protocol::{
    AdapterPageLimits, AdapterPageRequest, AdapterQuery, AuthContextDescriptor, CoverageBinding,
    RequestId, SafeFieldValue, TargetIdentity,
};

use crate::process::TargetEnvironment;
use crate::service::RuntimeConfig;

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn environment(
    values: impl IntoIterator<Item = (&'static str, &'static str)>,
) -> TargetEnvironment {
    let mut environment = BTreeMap::<String, OsString>::new();
    for name in ["PATH", "HOME", "USERPROFILE", "TEMP", "TMP", "TMPDIR"] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(name.to_owned(), value);
        }
    }
    environment.extend(
        values
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.into())),
    );
    TargetEnvironment::from_values(environment)
}

pub(super) fn config(
    vault: &Path,
    environment: TargetEnvironment,
    gh: Option<PathBuf>,
    aws: Option<PathBuf>,
    gcloud: Option<PathBuf>,
    github_api_base: Url,
) -> RuntimeConfig {
    RuntimeConfig::for_test(vault, environment, gh, aws, gcloud, github_api_base)
}

pub(super) fn request(
    target: &TargetIdentity,
    auth: &AuthContextDescriptor,
    operation: &str,
    provider_type: &str,
    resource_kind: &str,
    region: &str,
    projection: &[&str],
) -> AdapterPageRequest {
    AdapterPageRequest {
        protocol_version: scout_adapter_protocol::ADAPTER_PROTOCOL_VERSION,
        request_id: RequestId::random(),
        target_id: target.target_id.clone(),
        target_identity_sha256: target.fingerprint_sha256().unwrap(),
        adapter_id: auth.adapter_id.clone(),
        auth_context_handle: auth.handle.clone(),
        auth_context_id: auth.context_id.clone(),
        coverage: CoverageBinding {
            enterprise_id: "enterprise:test".to_owned(),
            charter_id: "charter:test".to_owned(),
            discovery_epoch: 1,
            sequence: 1,
            adapter_id: auth.adapter_id.clone(),
            auth_context_id: auth.context_id.clone(),
            tenant: auth.authority_scope.clone(),
            region_or_project: region.to_owned(),
            resource_kind: resource_kind.to_owned(),
        },
        query: AdapterQuery {
            operation: operation.to_owned(),
            authority_scope: auth.authority_scope.clone(),
            provider_resource_type: provider_type.to_owned(),
            filters: BTreeMap::<String, SafeFieldValue>::new(),
            projection: projection
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
            max_duration_ms: 10_000,
        },
        requested_at_ms: now_ms(),
    }
}

#[cfg(unix)]
pub(super) fn fake_cli(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = root.join("fake-cloud-cli");
    std::fs::write(
        &path,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$0.log"
case "$*" in
  "configure list-profiles")
    printf 'primary\ndenied\nstale\n'
    ;;
  *"sts get-caller-identity"*)
    case "$AWS_PROFILE" in
      denied)
        printf 'AccessDenied: fixture-canary-secret\n' >&2
        exit 1
        ;;
      stale)
        printf 'ExpiredToken: fixture-canary-secret\n' >&2
        exit 1
        ;;
      *)
        printf '{"Account":"123456789012","Arn":"arn:aws:iam::123456789012:role/Scout","UserId":"fixture"}'
        ;;
    esac
    ;;
  *"organizations list-accounts"*)
    case "$*" in
      *"--starting-token"*)
        printf '{"Accounts":[{"Id":"222222222222","Arn":"arn:aws:organizations::123456789012:account/o-test/222222222222","Email":"ops@example.com","Name":"Workload","State":"ACTIVE","JoinedMethod":"INVITED"}]}'
        ;;
      *)
        printf '{"Accounts":[],"NextToken":"provider-cursor-canary-raw"}'
        ;;
    esac
    ;;
  *"resource-explorer-2 list-resources"*)
    printf '{"Resources":[{"Arn":"arn:aws:lambda:us-east-1:123456789012:function:checkout","OwningAccountId":"123456789012","Region":"us-east-1","ResourceType":"lambda:function","Service":"lambda","Url":"https://console.aws.amazon.com/example"}]}'
    ;;
  *)
    printf 'unexpected mutation or argv\n' >&2
    exit 9
    ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
pub(super) fn fake_gcloud(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = root.join("fake-gcloud");
    std::fs::write(
        &path,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$0.log"
case "$*" in
  *"auth list"*)
    printf '[{"account":"scout@example.com","status":"ACTIVE"}]'
    ;;
  *"organizations describe 999"*)
    printf 'PERMISSION_DENIED provider-secret-canary\n' >&2
    exit 1
    ;;
  *"organizations describe"*)
    printf '{"name":"organizations/123","displayName":"Acme","state":"ACTIVE"}'
    ;;
  *"projects describe"*)
    printf '{"projectId":"acme-prod","projectNumber":"42","name":"Acme Prod","lifecycleState":"ACTIVE","parent":{"type":"organization","id":"123"}}'
    ;;
  *"resource-manager folders describe"*)
    printf '{"name":"folders/7","displayName":"Platform","state":"ACTIVE","parent":"organizations/123"}'
    ;;
  *"resource-manager folders list"*)
    printf '[{"name":"folders/7","displayName":"Platform","state":"ACTIVE","parent":"organizations/123"}]'
    ;;
  *"organizations list"*"--filter=name>organizations/1"*)
    printf '[{"name":"organizations/2","displayName":"Second","directoryCustomerId":"C2","state":"ACTIVE"}]'
    ;;
  *"organizations list"*)
    printf '[{"name":"organizations/1","displayName":"First","directoryCustomerId":"C1","state":"ACTIVE"},{"name":"organizations/2","displayName":"Second","directoryCustomerId":"C2","state":"ACTIVE"}]'
    ;;
  *"projects list"*"parent.type=folder"*)
    printf '[{"projectId":"folder-prod","projectNumber":"43","name":"Folder Prod","lifecycleState":"ACTIVE","parent":{"type":"folder","id":"7"}}]'
    ;;
  *"projects list"*)
    printf '[{"projectId":"acme-prod","projectNumber":"42","name":"Acme Prod","lifecycleState":"ACTIVE","parent":{"type":"organization","id":"123"}}]'
    ;;
  *"asset search-all-resources"*)
    printf '[{"name":"//compute.googleapis.com/projects/acme-prod/zones/us-central1-a/instances/web","assetType":"compute.googleapis.com/Instance","project":"projects/42","organization":"organizations/123","location":"us-central1-a","displayName":"web","state":"RUNNING"}]'
    ;;
  *)
    printf 'unexpected or mutating gcloud argv\n' >&2
    exit 9
    ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}
