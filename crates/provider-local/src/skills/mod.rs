//! Clark-owned skill discovery and progressive disclosure.
//!
//! Skills are instruction packages, not executable plugins or authorization.
//! The catalog exposes bounded metadata to the model; [`ReadSkill`](crate::tools::skill::ReadSkill)
//! and explicit `$skill` mentions load the bounded instruction body only when needed.

mod bundled;
mod loader;
mod plugin;
mod render;
mod selection;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::exec::Executor;

pub(crate) use loader::discover_catalog;
pub(crate) use render::render_catalog;
pub(crate) use selection::explicit_skill_injections;

const MAX_SKILL_BODY_BYTES: usize = 48 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SkillScope {
    Bundled,
    Project,
    User,
}

impl SkillScope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Project => "project",
            Self::User => "user",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Project => 0,
            Self::User => 1,
            Self::Bundled => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SkillOrigin {
    Clark,
    Compatible,
    Claude,
    Plugin,
}

impl SkillOrigin {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Clark => "Clark",
            Self::Compatible => "external",
            Self::Claude => "Claude",
            Self::Plugin => "plugin",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Clark | Self::Plugin => 0,
            Self::Compatible => 1,
            Self::Claude => 2,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum SkillResource {
    ExecutorFile(PathBuf),
    Embedded {
        locator: &'static str,
        contents: &'static str,
    },
}

impl SkillResource {
    pub(crate) fn locator(&self) -> String {
        match self {
            Self::ExecutorFile(path) => path.to_string_lossy().into_owned(),
            Self::Embedded { locator, .. } => (*locator).to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Skill {
    /// Collision-safe model-facing name (`plugin:skill` for plugin skills).
    pub name: String,
    /// Frontmatter name before plugin qualification.
    pub base_name: String,
    pub description: String,
    pub scope: SkillScope,
    pub origin: SkillOrigin,
    pub resource: SkillResource,
    pub required_tools: Vec<String>,
    pub allow_implicit_invocation: bool,
    pub enabled: bool,
}

impl Skill {
    pub(crate) fn locator(&self) -> String {
        self.resource.locator()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SkillCatalog {
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
}

impl SkillCatalog {
    pub(crate) fn sort_and_dedupe(&mut self) {
        self.skills.sort_by(|left, right| {
            left.scope
                .rank()
                .cmp(&right.scope.rank())
                .then_with(|| left.origin.rank().cmp(&right.origin.rank()))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.locator().cmp(&right.locator()))
        });
        let mut seen = HashSet::new();
        self.skills
            .retain(|skill| seen.insert(skill.name.to_ascii_lowercase()));
    }

    pub(crate) fn resolve_capabilities(
        &mut self,
        available_tools: &HashSet<String>,
        disabled_names: &[String],
    ) {
        let disabled = disabled_names
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        for skill in &mut self.skills {
            let configured_off = disabled.contains(skill.name.as_str())
                || (skill.name == skill.base_name && disabled.contains(skill.base_name.as_str()));
            let dependencies_available = skill
                .required_tools
                .iter()
                .all(|required| available_tools.contains(required));
            skill.enabled = !configured_off && dependencies_available;
        }
    }

    pub(crate) fn enabled(&self) -> impl Iterator<Item = &Skill> {
        self.skills.iter().filter(|skill| skill.enabled)
    }

    pub(crate) fn prompt_visible(&self) -> impl Iterator<Item = &Skill> {
        self.enabled()
            .filter(|skill| skill.allow_implicit_invocation)
    }

    pub(crate) fn resolve_name(&self, requested: &str) -> Result<&Skill, String> {
        let requested = requested.trim();
        let exact = self
            .enabled()
            .filter(|skill| skill.name.eq_ignore_ascii_case(requested))
            .collect::<Vec<_>>();
        if exact.len() == 1 {
            return Ok(exact[0]);
        }

        let base = self
            .enabled()
            .filter(|skill| skill.base_name.eq_ignore_ascii_case(requested))
            .collect::<Vec<_>>();
        match base.as_slice() {
            [skill] => Ok(*skill),
            [] => Err(format!("skill `{requested}` is not available")),
            _ => {
                let mut names = base
                    .iter()
                    .map(|skill| skill.name.as_str())
                    .collect::<Vec<_>>();
                names.sort_unstable();
                Err(format!(
                    "skill name `{requested}` is ambiguous; use one of: {}",
                    names.join(", ")
                ))
            }
        }
    }

    pub(crate) async fn read(&self, exec: &dyn Executor, skill: &Skill) -> Result<String, String> {
        self.read_resource(exec, skill, None).await
    }

    pub(crate) async fn read_resource(
        &self,
        exec: &dyn Executor,
        skill: &Skill,
        relative: Option<&str>,
    ) -> Result<String, String> {
        let bytes = match (&skill.resource, relative) {
            (SkillResource::ExecutorFile(path), None) => exec
                .read(path)
                .await
                .map_err(|error| format!("{}: {error}", path.display()))?,
            (SkillResource::ExecutorFile(skill_path), Some(relative)) => {
                let relative = safe_relative_resource(relative)?;
                let directory = skill_path
                    .parent()
                    .ok_or_else(|| "skill source has no parent directory".to_string())?;
                let path = directory.join(&relative);
                reject_symlink_components(exec, directory, &relative).await?;
                exec.read(&path)
                    .await
                    .map_err(|error| format!("{}: {error}", path.display()))?
            }
            (SkillResource::Embedded { contents, .. }, None) => contents.as_bytes().to_vec(),
            (SkillResource::Embedded { .. }, Some(relative)) => {
                return Err(format!(
                    "bundled skill `{}` has no readable resource `{relative}`",
                    skill.name
                ));
            }
        };
        let contents = String::from_utf8(bytes).map_err(|_| {
            format!(
                "skill resource `{}` is not UTF-8 text",
                relative.unwrap_or("SKILL.md")
            )
        })?;
        Ok(truncate_utf8(contents, MAX_SKILL_BODY_BYTES))
    }

    pub(crate) fn name_counts(&self) -> HashMap<&str, usize> {
        let mut counts = HashMap::new();
        for skill in self.enabled() {
            *counts.entry(skill.base_name.as_str()).or_insert(0) += 1;
        }
        counts
    }
}

pub(crate) fn render_injection(skill: &Skill, contents: &str) -> String {
    format!(
        "[runtime skill — selected from the current skill catalog]\n<skill>\n<name>{}</name>\n{}\n</skill>",
        xml_escape(&skill.name),
        contents.trim()
    )
}

pub(crate) fn render_resource(skill: &Skill, resource: &str, contents: &str) -> String {
    format!(
        "[runtime skill resource — data for the selected skill]\n<skill-resource>\n<skill>{}</skill>\n<resource>{}</resource>\n{}\n</skill-resource>",
        xml_escape(&skill.name),
        xml_escape(resource),
        contents.trim()
    )
}

fn safe_relative_resource(raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw.trim());
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("skill resource must be a relative path inside the skill directory".into());
    }
    Ok(path.to_path_buf())
}

async fn reject_symlink_components(
    exec: &dyn Executor,
    directory: &Path,
    relative: &Path,
) -> Result<(), String> {
    let mut current = directory.to_path_buf();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            continue;
        };
        current.push(segment);
        if exec
            .metadata(&current)
            .await
            .is_ok_and(|metadata| metadata.is_symlink)
        {
            return Err(format!(
                "skill resource refuses symlink path component: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str("\n\n[skill truncated by Clark at the context safety limit]");
    value
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
