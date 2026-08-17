use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Trusted, credential-free recipe attached to one coding session on a shared
/// remote worker. The desktop product prepares this recipe after validating
/// account and specialist authority; the worker applies it only to the new
/// session rather than replacing the durable project worker.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingSessionRecipe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specialist_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hard_constraints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scout_cartography: Option<ScoutCartographyRecipe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<CodingSessionExtensionRecipe>,
}

/// A compile-time product extension envelope. The public host validates the
/// bounded identifier and payload size; the branded worker registered for the
/// exact id must strictly deserialize and authorize the payload before use.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingSessionExtensionRecipe {
    pub id: String,
    pub config: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoutCartographyRecipe {
    pub organization_id: String,
    pub workspace_id: String,
    pub identity_root: PathBuf,
    pub platform: String,
    pub architecture: String,
    pub route_prefix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_run_request_id: Option<String>,
}

impl CodingSessionRecipe {
    pub fn validate(&self, project_root: &Path) -> Result<(), String> {
        if let Some(kind) = self.specialist_kind.as_deref() {
            validate_portable_value("specialist_kind", kind, 64)?;
        }
        let hard_constraints = self.hard_constraints.iter().collect::<BTreeSet<_>>();
        if self.hard_constraints.len() > 2
            || hard_constraints.len() != self.hard_constraints.len()
            || self
                .hard_constraints
                .iter()
                .any(|constraint| !matches!(constraint.as_str(), "no_delete" | "no_github_push"))
        {
            return Err("coding session hard constraints are invalid".into());
        }
        if let Some(scout) = self.scout_cartography.as_ref() {
            if self.specialist_kind.as_deref() != Some("scout") {
                return Err("Scout cartography requires specialist_kind=scout".into());
            }
            if !uuid_shape(&scout.organization_id) || !uuid_shape(&scout.workspace_id) {
                return Err("Scout organization and workspace ids are invalid".into());
            }
            validate_portable_value("Scout platform", &scout.platform, 64)?;
            validate_portable_value("Scout architecture", &scout.architecture, 64)?;
            if !scout.identity_root.is_absolute()
                || scout.identity_root == project_root
                || !scout.identity_root.starts_with(project_root)
                || scout
                    .identity_root
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(
                    "Scout identity root must be inside the registered remote project".into(),
                );
            }
            if !scout.route_prefix.starts_with('/')
                || scout.route_prefix.len() < 2
                || scout.route_prefix.len() > 128
                || scout.route_prefix.contains(['?', '#'])
                || scout.route_prefix.contains("..")
            {
                return Err("Scout route prefix is invalid".into());
            }
            if let Some(request_id) = scout.human_run_request_id.as_deref() {
                let valid = request_id.strip_prefix("scout-run:").is_some_and(|digest| {
                    digest.len() == 64
                        && digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                });
                if !valid {
                    return Err("Scout human run request id is invalid".into());
                }
            }
        }
        if self.extensions.len() > 8 {
            return Err("remote session recipe has too many product extensions".into());
        }
        let mut ids = BTreeSet::new();
        for extension in &self.extensions {
            validate_portable_value("session extension id", &extension.id, 64)?;
            if !ids.insert(extension.id.as_str()) {
                return Err(format!(
                    "remote session recipe repeats extension {}",
                    extension.id
                ));
            }
            let bytes = serde_json::to_vec(&extension.config)
                .map_err(|error| format!("session extension is invalid: {error}"))?;
            if bytes.len() > 64 * 1024 {
                return Err(format!(
                    "remote session extension {} exceeds 65536 bytes",
                    extension.id
                ));
            }
        }
        Ok(())
    }
}

fn validate_portable_value(field: &str, value: &str, max_len: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("{field} is invalid"));
    }
    Ok(())
}

fn uuid_shape(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

/// A project is registered by a trusted launcher, never by a request payload.
/// Requests refer to the stable `id`; this prevents a caller from turning a
/// generic worker into an arbitrary-path file service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRegistration {
    pub id: String,
    pub root: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectRegistry {
    projects: BTreeMap<String, PathBuf>,
}

impl ProjectRegistry {
    pub fn new(
        registrations: impl IntoIterator<Item = ProjectRegistration>,
    ) -> Result<Self, RegistryError> {
        let mut projects = BTreeMap::new();
        for registration in registrations {
            validate_identifier("project_id", &registration.id)?;
            let root =
                registration
                    .root
                    .canonicalize()
                    .map_err(|source| RegistryError::ProjectRoot {
                        path: registration.root,
                        source,
                    })?;
            if projects.insert(registration.id.clone(), root).is_some() {
                return Err(RegistryError::DuplicateProject(registration.id));
            }
        }
        Ok(Self { projects })
    }

    pub fn resolve(&self, project_id: &str) -> Result<&Path, RegistryError> {
        self.projects
            .get(project_id)
            .map(PathBuf::as_path)
            .ok_or_else(|| RegistryError::UnknownProject(project_id.to_string()))
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.projects.keys().map(String::as_str)
    }
}

pub(crate) fn validate_identifier(field: &str, value: &str) -> Result<(), RegistryError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RegistryError::InvalidIdentifier {
            field: field.to_string(),
            value: value.to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("invalid {field} identifier: {value}")]
    InvalidIdentifier { field: String, value: String },
    #[error("project root {path:?}: {source}")]
    ProjectRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("duplicate project registration: {0}")]
    DuplicateProject(String),
    #[error("unknown registered project: {0}")]
    UnknownProject(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_canonicalizes_and_rejects_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ProjectRegistry::new([ProjectRegistration {
            id: "fixture".into(),
            root: temp.path().to_path_buf(),
        }])
        .unwrap();
        let canonical = temp.path().canonicalize().unwrap();
        assert_eq!(registry.resolve("fixture").unwrap(), canonical.as_path());

        let error = ProjectRegistry::new([
            ProjectRegistration {
                id: "fixture".into(),
                root: temp.path().to_path_buf(),
            },
            ProjectRegistration {
                id: "fixture".into(),
                root: temp.path().to_path_buf(),
            },
        ])
        .unwrap_err();
        assert!(matches!(error, RegistryError::DuplicateProject(_)));
    }

    #[test]
    fn identifier_and_relative_path_rules_are_portable() {
        assert!(validate_identifier("task", "task-1.v1").is_ok());
        assert!(validate_identifier("task", "../escape").is_err());
    }

    #[test]
    fn remote_session_recipe_is_project_scoped_and_strict() {
        let project = Path::new("/srv/client/neon");
        let recipe = CodingSessionRecipe {
            specialist_kind: Some("scout".into()),
            hard_constraints: vec!["no_delete".into()],
            scout_cartography: Some(ScoutCartographyRecipe {
                organization_id: "59b8fe20-6072-4c16-9dae-9d7cbbf2533c".into(),
                workspace_id: "2fac2db5-20d6-499c-b691-47ad19fc0ca8".into(),
                identity_root: project.join(".clark/scout/identity/binding"),
                platform: "linux".into(),
                architecture: "x86_64".into(),
                route_prefix: "/v1/system-cartography".into(),
                human_run_request_id: Some(format!("scout-run:{}", "a".repeat(64))),
            }),
            extensions: vec![CodingSessionExtensionRecipe {
                id: "example_advisor".into(),
                config: serde_json::json!({ "organization_id": "org-1" }),
            }],
        };
        recipe.validate(project).unwrap();

        let mut escaped = recipe;
        escaped.scout_cartography.as_mut().unwrap().identity_root =
            PathBuf::from("/tmp/other-client");
        assert!(escaped.validate(project).is_err());
    }

    #[test]
    fn remote_session_extension_envelope_is_bounded_and_unique() {
        let project = Path::new("/srv/client/neon");
        let extension = CodingSessionExtensionRecipe {
            id: "example_advisor".into(),
            config: serde_json::json!({ "enabled": true }),
        };
        let mut recipe = CodingSessionRecipe {
            extensions: vec![extension.clone(), extension],
            ..CodingSessionRecipe::default()
        };
        assert!(recipe.validate(project).is_err());

        recipe.extensions.truncate(1);
        recipe.extensions[0].config = serde_json::json!({ "payload": "x".repeat(65_536) });
        assert!(recipe.validate(project).is_err());

        let constraints = CodingSessionRecipe {
            hard_constraints: vec!["no_delete".into(), "no_delete".into()],
            ..CodingSessionRecipe::default()
        };
        assert!(constraints.validate(project).is_err());
    }
}
