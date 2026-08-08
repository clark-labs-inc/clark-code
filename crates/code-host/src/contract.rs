use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
}
