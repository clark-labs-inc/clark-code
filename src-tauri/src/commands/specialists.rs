use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use super::cloud::read_json_or_err;
use super::cloud_authority::{clark_http_client, current_cloud_access};
use crate::state::AppState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Specialist {
    key: &'static str,
    operations: &'static [&'static str],
}

impl Specialist {
    fn parse(value: &str) -> Result<Self, String> {
        SPECIALISTS
            .iter()
            .copied()
            .find(|specialist| specialist.key == value.trim())
            .ok_or_else(|| "Clark specialist is not registered".into())
    }

    fn key(self) -> &'static str {
        self.key
    }

    fn allows(self, operation: &str) -> bool {
        self.operations.contains(&operation)
    }

    fn projection_path(self, organization_id: Uuid) -> Result<String, String> {
        match self.key {
            "scientist" => Ok(format!("/api/orgs/{organization_id}/research/overview")),
            "rsi" => Ok(format!("/api/orgs/{organization_id}/rsi/overview")),
            _ => Err("Clark specialist does not publish an overview projection".into()),
        }
    }
}

const SPECIALISTS: &[Specialist] = &[
    Specialist {
        key: "scout",
        operations: &[
            "scout_workspaces",
            "scout_snapshot",
            "scout_changes",
            "scout_simulations",
        ],
    },
    Specialist {
        key: "security",
        operations: &[
            "security_posture",
            "security_repositories",
            "security_findings",
            "security_candidates",
            "security_scans",
            "security_campaigns",
        ],
    },
    Specialist {
        key: "scientist",
        operations: &["scientist_overview", "scientist_artifacts"],
    },
    Specialist {
        key: "rsi",
        operations: &["rsi_overview", "rsi_artifacts"],
    },
];

#[tauri::command]
pub fn desktop_specialist_catalog() -> Result<Value, String> {
    let mut catalog: Value = serde_json::from_str(include_str!(
        "../../../app/src/lib/first-party-specialists.json"
    ))
    .map_err(|error| format!("bundled specialist catalog is invalid: {error}"))?;
    let expected = catalog
        .get("catalogSha256")
        .and_then(Value::as_str)
        .filter(|value| value.len() == 64)
        .ok_or("bundled specialist catalog has no digest")?
        .to_string();
    catalog
        .as_object_mut()
        .ok_or("bundled specialist catalog is not an object")?
        .remove("catalogSha256");
    let canonical = serde_json::to_vec(&catalog)
        .map_err(|error| format!("bundled specialist catalog is not canonical: {error}"))?;
    let actual = format!("{:x}", Sha256::digest(canonical));
    if actual != expected {
        return Err(format!(
            "bundled specialist catalog digest mismatch: expected {expected}, calculated {actual}"
        ));
    }
    catalog["catalogSha256"] = Value::String(expected);
    Ok(catalog)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpecialistProjectionPublished {
    specialist: String,
    organization_id: String,
    sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    program_id: Option<String>,
}

pub(crate) async fn publish_projection_from_trace(
    state: &AppState,
    payload: &Value,
) -> Result<SpecialistProjectionPublished, String> {
    let root = exact_object(payload, "specialist projection trace")?;
    let specialist_key = root
        .get("specialist")
        .and_then(Value::as_str)
        .ok_or("specialist projection trace has no specialist")?;
    let envelope_key = match specialist_key {
        "scientist" => "researchProjection",
        "rsi" => "rsiProjection",
        _ => return Err("specialist projection trace names an unsupported publisher".into()),
    };
    let organization_id = root
        .get("organizationId")
        .and_then(Value::as_str)
        .ok_or("specialist projection has no organization binding")?;
    let organization_uuid = uuid(organization_id, "organization id")?;
    let envelope = exact_object(
        root.get(envelope_key)
            .ok_or("specialist projection trace has no overview envelope")?,
        "specialist projection envelope",
    )?;
    exact_keys(
        envelope,
        &["schemaVersion", "sequence", "projection"],
        "specialist projection envelope",
    )?;
    if envelope.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("specialist projection envelope schema is invalid".into());
    }
    let sequence = envelope
        .get("sequence")
        .and_then(Value::as_u64)
        .filter(|sequence| *sequence > 0)
        .ok_or("specialist projection envelope sequence is invalid")?;
    let projection = envelope
        .get("projection")
        .filter(|value| value.is_object())
        .ok_or("specialist projection envelope payload is invalid")?;
    let account = state
        .runtime_registry
        .cloud_account()
        .await
        .ok_or("Clark cloud authority is unavailable for specialist publication")?;
    let specialist = Specialist::parse(specialist_key)?;
    let path = specialist.projection_path(organization_uuid)?;
    let response = clark_http_client()?
        .put(format!("{}{}", account.rest_base, path))
        .bearer_auth(account.token.as_str())
        .json(&json!({
            "schemaVersion": 1,
            "sequence": sequence,
            "projection": projection,
        }))
        .send()
        .await
        .map_err(|error| format!("Clark specialist publication failed: {error}"))?;
    let _ = read_json_or_err(response, "Clark specialist publication").await?;
    Ok(SpecialistProjectionPublished {
        specialist: specialist_key.into(),
        organization_id: organization_id.into(),
        sequence,
        program_id: root
            .get("programId")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn exact_object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} is not an object"))
}

fn exact_keys(value: &Map<String, Value>, expected: &[&str], label: &str) -> Result<(), String> {
    if value.len() != expected.len() || expected.iter().any(|key| !value.contains_key(*key)) {
        return Err(format!("{label} does not match the v1 schema"));
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
enum SpecialistRequest {
    Get {
        path: String,
        query: Vec<(&'static str, String)>,
    },
    Post {
        path: String,
        body: Value,
    },
}

fn uuid(value: &str, label: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value.trim()).map_err(|_| format!("Clark specialist {label} is invalid"))
}

fn request_for(
    specialist: Specialist,
    operation: &str,
    organization_id: Uuid,
    workspace_id: Option<&str>,
    repository_id: Option<&str>,
) -> Result<SpecialistRequest, String> {
    if !specialist.allows(operation) {
        return Err("Clark specialist operation is not allowed".into());
    }
    let root = format!("/api/orgs/{organization_id}");
    match (specialist.key(), operation) {
        ("scout", "scout_workspaces") => Ok(SpecialistRequest::Get {
            path: format!("{root}/system-cartography/workspaces"),
            query: vec![],
        }),
        ("scout", operation @ ("scout_snapshot" | "scout_changes" | "scout_simulations")) => {
            let workspace_id = uuid(
                workspace_id.ok_or_else(|| format!("{operation} requires a Scout workspace"))?,
                "workspace id",
            )?;
            let base = format!("{root}/system-cartography/workspaces/{workspace_id}");
            match operation {
                "scout_snapshot" => Ok(SpecialistRequest::Post {
                    path: format!("{base}/snapshots/query"),
                    body: json!({
                        "organization_id": organization_id,
                        "workspace_id": workspace_id,
                        "object_kinds": [],
                        "limit": 1000,
                        "cursor": null,
                    }),
                }),
                "scout_changes" => Ok(SpecialistRequest::Post {
                    path: format!("{base}/changes/query"),
                    body: json!({
                        "organization_id": organization_id,
                        "workspace_id": workspace_id,
                        "after_sequence": 0,
                        "limit": 1000,
                    }),
                }),
                _ => Ok(SpecialistRequest::Get {
                    path: format!("{base}/simulation-overlays"),
                    query: vec![],
                }),
            }
        }
        ("security", "security_posture") => Ok(SpecialistRequest::Get {
            path: format!("{root}/security/posture"),
            query: vec![],
        }),
        ("security", "security_repositories") => Ok(SpecialistRequest::Get {
            path: format!("{root}/security/repositories"),
            query: vec![("limit", "100".into())],
        }),
        ("security", operation @ ("security_findings" | "security_candidates")) => {
            let suffix = if operation == "security_findings" {
                "findings"
            } else {
                "candidates"
            };
            Ok(SpecialistRequest::Get {
                path: format!("{root}/security/{suffix}"),
                query: vec![("limit", "200".into())],
            })
        }
        ("security", "security_scans") => {
            let repository_id = uuid(
                repository_id.ok_or_else(|| "security_scans requires a repository".to_string())?,
                "repository id",
            )?;
            Ok(SpecialistRequest::Get {
                path: format!("{root}/security/repositories/{repository_id}/scan-runs"),
                query: vec![("limit", "50".into())],
            })
        }
        ("security", "security_campaigns") => Ok(SpecialistRequest::Get {
            path: format!("{root}/security/campaigns"),
            query: vec![("limit", "50".into())],
        }),
        ("scientist", "scientist_overview") => Ok(SpecialistRequest::Get {
            path: format!("{root}/research/overview"),
            query: vec![],
        }),
        ("scientist", "scientist_artifacts") | ("rsi", "rsi_artifacts") => {
            Ok(SpecialistRequest::Get {
                path: format!("{root}/science/artifacts"),
                query: vec![],
            })
        }
        ("rsi", "rsi_overview") => Ok(SpecialistRequest::Get {
            path: format!("{root}/rsi/overview"),
            query: vec![],
        }),
        _ => Err("Clark specialist operation is not allowed".into()),
    }
}

#[tauri::command]
pub async fn desktop_specialist_organizations(state: State<'_, AppState>) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let response = clark_http_client()?
        .get(format!("{}/api/orgs", access.rest_base))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|error| format!("Clark specialist organization request failed: {error}"))?;
    read_json_or_err(response, "Clark specialist organizations").await
}

#[tauri::command]
pub async fn desktop_specialist_entitlement(
    specialist: String,
    organization_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let specialist = Specialist::parse(&specialist)?;
    let organization_id = organization_id
        .as_deref()
        .map(|value| uuid(value, "organization id"))
        .transpose()?;
    let mut request = clark_http_client()?
        .get(format!(
            "{}/api/specialists/{}/entitlement",
            access.rest_base,
            specialist.key(),
        ))
        .bearer_auth(&token);
    if let Some(organization_id) = organization_id {
        request = request.query(&[("organizationId", organization_id.to_string())]);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Clark specialist entitlement request failed: {error}"))?;
    read_json_or_err(response, "Clark specialist entitlement").await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn desktop_specialist_query(
    specialist: String,
    operation: String,
    organization_id: String,
    workspace_id: Option<String>,
    repository_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let organization_id = uuid(&organization_id, "organization id")?;
    let request = request_for(
        Specialist::parse(&specialist)?,
        operation.trim(),
        organization_id,
        workspace_id.as_deref(),
        repository_id.as_deref(),
    )?;
    let client = clark_http_client()?;
    let response = match request {
        SpecialistRequest::Get { path, query } => {
            client
                .get(format!("{}{}", access.rest_base, path))
                .bearer_auth(&token)
                .query(&query)
                .send()
                .await
        }
        SpecialistRequest::Post { path, body } => {
            client
                .post(format!("{}{}", access.rest_base, path))
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await
        }
    }
    .map_err(|error| format!("Clark specialist request failed: {error}"))?;
    read_json_or_err(response, "Clark specialist request").await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn desktop_specialist_publish(
    specialist: String,
    organization_id: String,
    schema_version: u32,
    sequence: u64,
    projection: Value,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if schema_version != 1 || sequence == 0 {
        return Err("Clark specialist projection schema or sequence is invalid".into());
    }
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let organization_id = uuid(&organization_id, "organization id")?;
    let path = Specialist::parse(&specialist)?.projection_path(organization_id)?;
    let response = clark_http_client()?
        .put(format!("{}{}", access.rest_base, path))
        .bearer_auth(&token)
        .json(&json!({
            "schemaVersion": schema_version,
            "sequence": sequence,
            "projection": projection,
        }))
        .send()
        .await
        .map_err(|error| format!("Clark specialist publication failed: {error}"))?;
    read_json_or_err(response, "Clark specialist publication").await
}

/// Derive a portable stable key from a human workspace name, matching the
/// `clark-cli` `scout_workspace_key` contract so the desktop and the CLI can
/// name the same workspace.
fn scout_workspace_key(name: &str) -> String {
    let mut key: String = name
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_ascii_lowercase() || character.is_ascii_digit() {
                character
            } else {
                '-'
            }
        })
        .collect();
    while key.contains("--") {
        key = key.replace("--", "-");
    }
    let key = key.trim_matches('-');
    let key = if key.is_empty() { "workspace" } else { key };
    format!("cli-{}", key.chars().take(100).collect::<String>())
}

/// Create a Scout cartography workspace for an organization. The first Scout
/// run for an organization that has no workspace yet cannot enroll —
/// `scout_enterprise enroll` fails with "not host-configured" because no
/// `workspace_id` exists. Auto-creating a workspace here lets the desktop reach
/// the same `POST /cli/scout/workspaces` contract `clark-cli` uses with
/// `--create-workspace`, so the first Scout run can enroll and upload evidence
/// instead of silently sealing local-`Partial`.
#[tauri::command]
pub async fn desktop_specialist_create_workspace(
    organization_id: String,
    display_name: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let organization_id = uuid(&organization_id, "organization id")?;
    let name = display_name.trim();
    if name.is_empty() {
        return Err("Scout workspace name must not be empty".into());
    }
    let stable_key = scout_workspace_key(name);
    let response = clark_http_client()?
        .post(format!("{}/cli/scout/workspaces", access.rest_base))
        .bearer_auth(&token)
        .json(&json!({
            "organizationId": organization_id,
            "stableKey": stable_key,
            "displayName": name,
        }))
        .send()
        .await
        .map_err(|error| format!("could not create Clark Scout workspace: {error}"))?;
    read_json_or_err(response, "Clark Scout workspace creation").await
}

fn security_campaign_create_request(
    organization_id: Uuid,
    title: &str,
    description: &str,
    finding_ids: &[String],
) -> Result<SpecialistRequest, String> {
    let title = title.trim();
    let description = description.trim();
    if title.is_empty() || description.is_empty() {
        return Err("Security campaign title and description must not be empty".into());
    }
    if finding_ids.is_empty() {
        return Err("Security campaign requires at least one finding".into());
    }
    let finding_ids = finding_ids
        .iter()
        .map(|value| uuid(value, "finding id"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SpecialistRequest::Post {
        path: format!("/api/orgs/{organization_id}/security/campaigns"),
        body: json!({
            "title": title,
            "description": description,
            "findingIds": finding_ids,
            "dueAt": null,
            "idempotencyKey": format!("desktop:{}", Uuid::new_v4()),
        }),
    })
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn desktop_specialist_create_security_campaign(
    organization_id: String,
    title: String,
    description: String,
    finding_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let request = security_campaign_create_request(
        uuid(&organization_id, "organization id")?,
        &title,
        &description,
        &finding_ids,
    )?;
    let SpecialistRequest::Post { path, body } = request else {
        return Err("Security campaign request is invalid".into());
    };
    let response = clark_http_client()?
        .post(format!("{}{}", access.rest_base, path))
        .bearer_auth(&access.token)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("could not create Clark Security campaign: {error}"))?;
    read_json_or_err(response, "Clark Security campaign creation").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specialist_query_is_a_closed_allowlist_with_route_bound_ids() {
        let org = Uuid::nil();
        let workspace = Uuid::from_u128(1).to_string();
        assert_eq!(
            request_for(
                Specialist::parse("scout").unwrap(),
                "scout_snapshot",
                org,
                Some(&workspace),
                None,
            )
            .expect("snapshot request"),
            SpecialistRequest::Post {
                path: format!(
                    "/api/orgs/{org}/system-cartography/workspaces/{workspace}/snapshots/query"
                ),
                body: json!({
                    "organization_id": org,
                    "workspace_id": Uuid::from_u128(1),
                    "object_kinds": [],
                    "limit": 1000,
                    "cursor": null,
                }),
            },
        );
        assert_eq!(
            request_for(
                Specialist::parse("scientist").unwrap(),
                "scientist_artifacts",
                org,
                None,
                None,
            )
            .unwrap(),
            SpecialistRequest::Get {
                path: format!("/api/orgs/{org}/science/artifacts"),
                query: vec![],
            },
        );
        assert_eq!(
            request_for(
                Specialist::parse("security").unwrap(),
                "security_campaigns",
                org,
                None,
                None,
            )
            .unwrap(),
            SpecialistRequest::Get {
                path: format!("/api/orgs/{org}/security/campaigns"),
                query: vec![("limit", "50".into())],
            },
        );
        assert!(request_for(
            Specialist::parse("security").unwrap(),
            "arbitrary_url",
            org,
            None,
            None,
        )
        .is_err());
        assert!(request_for(
            Specialist::parse("security").unwrap(),
            "security_scans",
            org,
            None,
            Some("../../billing"),
        )
        .is_err());
        assert_eq!(
            request_for(
                Specialist::parse("scientist").unwrap(),
                "scientist_overview",
                org,
                None,
                None,
            )
            .unwrap(),
            SpecialistRequest::Get {
                path: format!("/api/orgs/{org}/research/overview"),
                query: vec![],
            },
        );
    }

    #[test]
    fn projection_trace_requires_an_exact_versioned_envelope() {
        let envelope = json!({
            "schemaVersion": 1,
            "sequence": 7,
            "projection": {},
        });
        let parsed = exact_object(&envelope, "envelope").unwrap();
        assert!(exact_keys(
            parsed,
            &["schemaVersion", "sequence", "projection"],
            "envelope"
        )
        .is_ok());
        let extra = json!({
            "schemaVersion": 1,
            "sequence": 7,
            "projection": {},
            "privateTrajectory": {},
        });
        assert!(exact_keys(
            exact_object(&extra, "envelope").unwrap(),
            &["schemaVersion", "sequence", "projection"],
            "envelope"
        )
        .is_err());
    }

    #[test]
    fn bundled_specialist_catalog_has_a_valid_digest() {
        let catalog = desktop_specialist_catalog().unwrap();
        assert_eq!(catalog["schemaVersion"], 1);
        assert_eq!(catalog["catalogVersion"], "1.0.0");
        assert_eq!(catalog["manifests"].as_array().unwrap().len(), 4);
        assert_eq!(catalog["catalogSha256"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn only_research_specialists_can_publish_overview_projections() {
        let org = Uuid::nil();
        assert_eq!(
            Specialist::parse("scientist")
                .unwrap()
                .projection_path(org)
                .unwrap(),
            format!("/api/orgs/{org}/research/overview"),
        );
        assert_eq!(
            Specialist::parse("rsi")
                .unwrap()
                .projection_path(org)
                .unwrap(),
            format!("/api/orgs/{org}/rsi/overview"),
        );
        assert!(Specialist::parse("security")
            .unwrap()
            .projection_path(org)
            .is_err());
    }

    #[test]
    fn scout_workspace_keys_match_the_cli_contract() {
        assert_eq!(
            scout_workspace_key("Production Systems"),
            "cli-production-systems"
        );
        assert_eq!(scout_workspace_key("  "), "cli-workspace");
        // Non-alphanumerics collapse to single dashes; a leading/trailing dash
        // is trimmed, and an all-symbol name falls back to the default key.
        assert_eq!(
            scout_workspace_key("-- API --- Gateway --"),
            "cli-api-gateway"
        );
        assert_eq!(
            scout_workspace_key("My 2nd Workspace!"),
            "cli-my-2nd-workspace"
        );
    }

    #[test]
    fn security_campaign_create_is_typed_and_route_bound() {
        let org = Uuid::nil();
        let finding = Uuid::from_u128(7).to_string();
        let SpecialistRequest::Post { path, body } = security_campaign_create_request(
            org,
            " Tenant hardening ",
            " Verify the fix ",
            std::slice::from_ref(&finding),
        )
        .unwrap() else {
            panic!("expected campaign post");
        };
        assert_eq!(path, format!("/api/orgs/{org}/security/campaigns"));
        assert_eq!(body["title"], "Tenant hardening");
        assert_eq!(body["description"], "Verify the fix");
        assert_eq!(body["findingIds"], json!([Uuid::from_u128(7)]));
        assert_eq!(body["dueAt"], Value::Null);
        assert!(body["idempotencyKey"]
            .as_str()
            .is_some_and(|value| value.starts_with("desktop:")));
        assert!(security_campaign_create_request(org, "x", "y", &[]).is_err());
        assert!(security_campaign_create_request(org, "x", "y", &["../finding".into()]).is_err());
    }
}
