use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use super::loader::{valid_namespace, SkillRoot, MAX_NAME_CHARS};
use super::{SkillOrigin, SkillScope};
use crate::exec::Executor;
use crate::markdown_frontmatter::read_text;

#[derive(Debug, Deserialize)]
struct PluginManifest {
    name: String,
    #[serde(default)]
    skills: Option<ManifestPaths>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ManifestPaths {
    One(String),
    Many(Vec<String>),
}

pub(super) async fn roots(
    exec: &dyn Executor,
    project_root: &Path,
    home: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Vec<SkillRoot> {
    let mut plugin_dirs = vec![project_root.join(".agent/plugins")];
    if let Some(home) = home {
        plugin_dirs.push(home.join(".agent/plugins"));
    }

    let mut roots = Vec::new();
    for (index, plugin_dir) in plugin_dirs.into_iter().enumerate() {
        let scope = if index == 0 {
            SkillScope::Project
        } else {
            SkillScope::User
        };
        let Ok(mut entries) = exec.read_dir(&plugin_dir).await else {
            continue;
        };
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        for entry in entries.into_iter().filter(|entry| entry.is_dir) {
            if entry.name.starts_with('.') {
                continue;
            }
            let plugin_root = plugin_dir.join(&entry.name);
            let manifest_paths = [
                plugin_root.join(".agent-plugin/plugin.json"),
                plugin_root.join(".codex-plugin/plugin.json"),
            ];
            let mut loaded = None;
            for manifest_path in manifest_paths {
                if let Some(text) = read_text(exec, &manifest_path).await {
                    loaded = Some((manifest_path, text));
                    break;
                }
            }
            let Some((manifest_path, text)) = loaded else {
                continue;
            };
            let manifest: PluginManifest = match serde_json::from_str(&text) {
                Ok(manifest) => manifest,
                Err(error) => {
                    warnings.push(format!(
                        "{}: invalid plugin manifest: {error}",
                        manifest_path.display()
                    ));
                    continue;
                }
            };
            let namespace = manifest.name.trim().to_string();
            if !valid_namespace(&namespace) {
                warnings.push(format!(
                    "{}: plugin name must be lowercase hyphen-case and at most {MAX_NAME_CHARS} characters",
                    manifest_path.display()
                ));
                continue;
            }
            for relative in manifest_skill_paths(manifest.skills) {
                let Some(path) = safe_plugin_path(&plugin_root, &relative) else {
                    warnings.push(format!(
                        "{}: ignored skills path `{relative}` outside the plugin root",
                        manifest_path.display()
                    ));
                    continue;
                };
                roots.push(SkillRoot {
                    path,
                    scope,
                    origin: SkillOrigin::Plugin,
                    namespace: Some(namespace.clone()),
                    identity_namespace: Some(format!("plugin:{namespace}")),
                    revision_context: None,
                });
            }
        }
    }
    roots
}

fn manifest_skill_paths(paths: Option<ManifestPaths>) -> Vec<String> {
    match paths {
        Some(ManifestPaths::One(path)) => vec![path],
        Some(ManifestPaths::Many(paths)) => paths,
        None => vec!["./skills".to_string()],
    }
}

fn safe_plugin_path(root: &Path, raw: &str) -> Option<PathBuf> {
    let relative = raw.strip_prefix("./")?;
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    Some(root.join(path))
}
