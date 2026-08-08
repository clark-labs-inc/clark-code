use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{
    AdapterPageRequest, NormalizedLink, NormalizedRecord, RedactionSummary, SafeFieldValue,
};

use super::{CloudAsset, Folder, GcpPage, Organization, Project};
use crate::error::{RuntimeError, RuntimeResult};
use crate::vault::ProviderCursor;

pub(super) fn finish_page<T>(
    request: &AdapterPageRequest,
    mut rows: Vec<T>,
    page_size: u32,
    cursor_kind: u8,
    key: impl Fn(&T) -> String,
    normalize: impl Fn(&AdapterPageRequest, T) -> RuntimeResult<NormalizedRecord>,
) -> RuntimeResult<GcpPage> {
    if rows.len() > page_size.saturating_add(1) as usize {
        return Err(RuntimeError::ProviderProtocol);
    }
    let has_next = rows.len() > page_size as usize;
    if has_next {
        rows.pop();
    }
    let next_key = has_next.then(|| rows.last().map(&key)).flatten();
    if has_next && next_key.is_none() {
        return Err(RuntimeError::ProviderProtocol);
    }
    let source_records_seen = rows.len() as u64;
    let records = rows
        .into_iter()
        .map(|row| normalize(request, row))
        .collect::<RuntimeResult<Vec<_>>>()?;
    Ok(GcpPage {
        next_cursor: next_key.map(|key| ProviderCursor::GcpAfterKey {
            operation: cursor_kind,
            key,
        }),
        redaction: RedactionSummary {
            source_records_seen,
            records_emitted: records.len() as u64,
            fields_omitted: 0,
            values_rejected: 0,
        },
        records,
    })
}

pub(super) fn normalize_org(
    request: &AdapterPageRequest,
    row: Organization,
) -> RuntimeResult<NormalizedRecord> {
    normalize(
        request,
        "global",
        row.name.clone(),
        "cloud_organization",
        [
            ("name", Some(SafeFieldValue::Text(row.name))),
            ("display_name", row.display_name.map(SafeFieldValue::Text)),
            (
                "directory_customer_id",
                row.directory_customer_id.map(SafeFieldValue::Text),
            ),
            ("state", row.state.map(SafeFieldValue::Text)),
        ],
        BTreeSet::new(),
    )
}

pub(super) fn normalize_folder(
    request: &AdapterPageRequest,
    row: Folder,
) -> RuntimeResult<NormalizedRecord> {
    if row
        .name
        .strip_prefix("folders/")
        .is_none_or(|id| !numeric_id(id))
    {
        return Err(RuntimeError::ProviderProtocol);
    }
    let links = folder_parent_link(row.parent.as_deref())?;
    normalize(
        request,
        "global",
        row.name.clone(),
        "cloud_folder",
        [
            ("name", Some(SafeFieldValue::Text(row.name))),
            ("display_name", row.display_name.map(SafeFieldValue::Text)),
            ("state", row.state.map(SafeFieldValue::Text)),
            ("parent", row.parent.map(SafeFieldValue::Text)),
        ],
        links,
    )
}

pub(super) fn normalize_project(
    request: &AdapterPageRequest,
    row: Project,
) -> RuntimeResult<NormalizedRecord> {
    let (parent_type, parent_id) = row
        .parent
        .map(|parent| (parent.parent_type, parent.id))
        .unwrap_or_default();
    let links = project_parent_link(parent_type.as_deref(), parent_id.as_deref())?;
    let native_id = match row.project_number.as_deref() {
        Some(project_number) if numeric_id(project_number) => {
            format!("projects/{project_number}")
        }
        Some(_) => return Err(RuntimeError::ProviderProtocol),
        None => format!("projects/id:{}", row.project_id),
    };
    normalize(
        request,
        "global",
        native_id,
        "cloud_project",
        [
            ("project_id", Some(SafeFieldValue::Text(row.project_id))),
            (
                "project_number",
                row.project_number.map(SafeFieldValue::Text),
            ),
            ("name", row.name.map(SafeFieldValue::Text)),
            (
                "lifecycle_state",
                row.lifecycle_state.map(SafeFieldValue::Text),
            ),
            ("parent_type", parent_type.map(SafeFieldValue::Text)),
            ("parent_id", parent_id.map(SafeFieldValue::Text)),
        ],
        links,
    )
}

pub(super) fn normalize_asset(
    request: &AdapterPageRequest,
    row: CloudAsset,
) -> RuntimeResult<NormalizedRecord> {
    let links = asset_owner_links(row.project.as_deref(), row.organization.as_deref());
    normalize(
        request,
        "global",
        row.name.clone(),
        "cloud_resource",
        [
            ("name", Some(SafeFieldValue::Text(row.name))),
            ("asset_type", row.asset_type.map(SafeFieldValue::Text)),
            ("project", row.project.map(SafeFieldValue::Text)),
            ("organization", row.organization.map(SafeFieldValue::Text)),
            ("location", row.location.map(SafeFieldValue::Text)),
            ("display_name", row.display_name.map(SafeFieldValue::Text)),
            ("state", row.state.map(SafeFieldValue::Text)),
        ],
        links,
    )
}

fn normalize<const N: usize>(
    request: &AdapterPageRequest,
    identity_authority_scope: &str,
    native_id: String,
    semantic_kind: &str,
    candidates: [(&str, Option<SafeFieldValue>); N],
    links: BTreeSet<NormalizedLink>,
) -> RuntimeResult<NormalizedRecord> {
    let fields = candidates
        .into_iter()
        .filter(|(name, _)| request.query.projection.contains(*name))
        .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value)))
        .collect::<BTreeMap<_, _>>();
    NormalizedRecord::new(
        request.adapter_id.clone(),
        "gcp".to_owned(),
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

fn project_parent_link(
    parent_type: Option<&str>,
    parent_id: Option<&str>,
) -> RuntimeResult<BTreeSet<NormalizedLink>> {
    let link = match (parent_type, parent_id) {
        (None, None) => return Ok(BTreeSet::new()),
        (Some("organization"), Some(id)) if numeric_id(id) => NormalizedLink {
            relationship_type: "member_of".to_owned(),
            target_provider_namespace: "gcp".to_owned(),
            target_provider_type: "gcp.organization".to_owned(),
            target_authority_scope: "global".to_owned(),
            target_native_id: format!("organizations/{id}"),
            qualifier: None,
        },
        (Some("folder"), Some(id)) if numeric_id(id) => NormalizedLink {
            relationship_type: "member_of".to_owned(),
            target_provider_namespace: "gcp".to_owned(),
            target_provider_type: "gcp.folder".to_owned(),
            target_authority_scope: "global".to_owned(),
            target_native_id: format!("folders/{id}"),
            qualifier: None,
        },
        _ => return Err(RuntimeError::ProviderProtocol),
    };
    Ok(BTreeSet::from([link]))
}

fn folder_parent_link(parent: Option<&str>) -> RuntimeResult<BTreeSet<NormalizedLink>> {
    let link = match parent {
        None => return Ok(BTreeSet::new()),
        Some(parent) => {
            let (provider_type, native_id) = if let Some(id) = parent
                .strip_prefix("organizations/")
                .filter(|id| numeric_id(id))
            {
                ("gcp.organization", format!("organizations/{id}"))
            } else if let Some(id) = parent.strip_prefix("folders/").filter(|id| numeric_id(id)) {
                ("gcp.folder", format!("folders/{id}"))
            } else {
                return Err(RuntimeError::ProviderProtocol);
            };
            NormalizedLink {
                relationship_type: "member_of".to_owned(),
                target_provider_namespace: "gcp".to_owned(),
                target_provider_type: provider_type.to_owned(),
                target_authority_scope: "global".to_owned(),
                target_native_id: native_id,
                qualifier: None,
            }
        }
    };
    Ok(BTreeSet::from([link]))
}

fn asset_owner_links(
    project: Option<&str>,
    organization: Option<&str>,
) -> BTreeSet<NormalizedLink> {
    if let Some(project) = project.and_then(gcp_numeric_name) {
        return BTreeSet::from([NormalizedLink {
            relationship_type: "owned_by".to_owned(),
            target_provider_namespace: "gcp".to_owned(),
            target_provider_type: "gcp.project".to_owned(),
            target_authority_scope: "global".to_owned(),
            target_native_id: format!("projects/{project}"),
            qualifier: None,
        }]);
    }
    organization
        .and_then(|value| value.strip_prefix("organizations/"))
        .filter(|value| numeric_id(value))
        .map(|organization| NormalizedLink {
            relationship_type: "owned_by".to_owned(),
            target_provider_namespace: "gcp".to_owned(),
            target_provider_type: "gcp.organization".to_owned(),
            target_authority_scope: "global".to_owned(),
            target_native_id: format!("organizations/{organization}"),
            qualifier: None,
        })
        .into_iter()
        .collect()
}

fn gcp_numeric_name(value: &str) -> Option<&str> {
    value
        .strip_prefix("projects/")
        .filter(|value| numeric_id(value))
}

fn numeric_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}
