//! Clark Code-owned skill discovery and progressive disclosure.
//!
//! Skills are instruction packages, not executable plugins or authorization.
//! The catalog exposes complete metadata to the model; [`ReadSkill`](crate::tools::skill::ReadSkill)
//! and explicit `$skill` mentions load the complete instruction body only when needed.

mod bundled;
mod loader;
mod managed;
mod plugin;
mod render;
mod selection;
mod service;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::exec::Executor;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub use managed::{
    install_skill_pack, list_skill_packs, uninstall_skill_pack, InstallSkillPackRequest,
    InstalledSkillPack, SkillPackAction, SkillPackReceipt, SkillPackScope,
};
pub(crate) use render::{render_catalog, replace_catalog_section};
pub(crate) use selection::{bound_skill_injections, explicit_skill_injections, invokes_skill};
pub use service::{skill_environment_id, SkillCatalogService};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    Bundled,
    Project,
    User,
}

impl SkillScope {
    pub fn label(self) -> &'static str {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillOrigin {
    Bundled,
    Compatible,
    Claude,
    Plugin,
}

impl SkillOrigin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bundled => "Clark Code",
            Self::Compatible => "external",
            Self::Claude => "Claude",
            Self::Plugin => "plugin",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Bundled | Self::Plugin => 0,
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
    /// Stable identity for this canonical source.
    pub id: String,
    /// Content-addressed revision of the skill instructions.
    pub revision: String,
    /// Collision-safe model-facing name (`plugin:skill` for plugin skills).
    pub name: String,
    /// Exact catalog invocation key. This equals `name` when unique and is
    /// source-qualified when multiple skills declare the same name.
    pub invocation_name: String,
    /// Frontmatter name before plugin qualification.
    pub base_name: String,
    pub description: String,
    pub scope: SkillScope,
    pub origin: SkillOrigin,
    pub resource: SkillResource,
    pub required_tools: Vec<String>,
    pub allow_implicit_invocation: bool,
    pub enabled: bool,
    pub missing_tools: Vec<String>,
    pub disabled_reason: Option<String>,
    pub has_name_collision: bool,
}

impl Skill {
    pub(crate) fn locator(&self) -> String {
        self.resource.locator()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillDiagnostic {
    pub severity: SkillDiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub source: Option<String>,
}

impl SkillDiagnostic {
    fn warning(
        code: impl Into<String>,
        source: Option<impl Into<String>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: SkillDiagnosticSeverity::Warning,
            code: code.into(),
            message: message.into(),
            source: source.map(Into::into),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillDiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogEntry {
    pub id: String,
    pub revision: String,
    pub name: String,
    pub invocation_name: String,
    pub description: String,
    pub scope: SkillScope,
    pub origin: SkillOrigin,
    pub source: String,
    pub required_tools: Vec<String>,
    pub missing_tools: Vec<String>,
    pub allow_implicit_invocation: bool,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub has_name_collision: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogSnapshot {
    pub revision: String,
    pub environment_id: String,
    pub project_root: String,
    pub skills: Vec<SkillCatalogEntry>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

pub async fn discover_skill_catalog_snapshot(
    exec: &dyn Executor,
    project_root: &Path,
    environment_id: impl Into<String>,
    available_tools: &HashSet<String>,
    disabled_names: &[String],
) -> SkillCatalogSnapshot {
    let mut catalog = loader::discover_catalog(exec, project_root).await;
    catalog.resolve_capabilities(available_tools, disabled_names);
    catalog.snapshot(environment_id, project_root)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SkillCatalog {
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

impl SkillCatalog {
    pub(crate) fn warning(
        &mut self,
        code: impl Into<String>,
        source: Option<impl Into<String>>,
        message: impl Into<String>,
    ) {
        self.diagnostics
            .push(SkillDiagnostic::warning(code, source, message));
    }

    pub(crate) fn sort_and_finalize(&mut self) {
        self.skills.sort_by(|left, right| {
            left.scope
                .rank()
                .cmp(&right.scope.rank())
                .then_with(|| left.origin.rank().cmp(&right.origin.rank()))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.locator().cmp(&right.locator()))
        });
        // The same canonical source may be reachable through more than one
        // compatibility root or directory symlink. That is one skill, not a
        // collision. Distinct sources declaring the same name remain visible.
        let mut seen_sources = HashSet::new();
        self.skills
            .retain(|skill| seen_sources.insert(skill.id.clone()));

        let mut name_counts = HashMap::new();
        for skill in &self.skills {
            *name_counts
                .entry(skill.name.to_ascii_lowercase())
                .or_insert(0usize) += 1;
        }
        let mut invocation_counts = HashMap::new();
        for skill in &mut self.skills {
            let has_collision = name_counts
                .get(&skill.name.to_ascii_lowercase())
                .copied()
                .unwrap_or_default()
                > 1;
            skill.has_name_collision = has_collision;
            let candidate = if has_collision {
                format!(
                    "{}:{}:{}",
                    skill.scope.label(),
                    skill.origin.invocation_label(),
                    skill.name
                )
            } else {
                skill.name.clone()
            };
            let candidate_key = candidate.to_ascii_lowercase();
            let count = invocation_counts.entry(candidate_key).or_insert(0usize);
            *count += 1;
            skill.invocation_name = if *count == 1 {
                candidate
            } else {
                format!("{candidate}:{}", short_id(&skill.id))
            };
        }
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
                || disabled.contains(skill.invocation_name.as_str())
                || disabled.contains(skill.id.as_str())
                || (skill.name == skill.base_name && disabled.contains(skill.base_name.as_str()));
            skill.missing_tools = skill
                .required_tools
                .iter()
                .filter(|required| !available_tools.contains(*required))
                .cloned()
                .collect();
            skill.disabled_reason = if configured_off {
                Some("disabled by project settings".to_string())
            } else if !skill.missing_tools.is_empty() {
                Some(format!(
                    "missing required tools: {}",
                    skill.missing_tools.join(", ")
                ))
            } else {
                None
            };
            skill.enabled = skill.disabled_reason.is_none();
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
        let invocation = self
            .enabled()
            .filter(|skill| skill.invocation_name.eq_ignore_ascii_case(requested))
            .collect::<Vec<_>>();
        if invocation.len() == 1 {
            return Ok(invocation[0]);
        }
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
                    .map(|skill| skill.invocation_name.as_str())
                    .collect::<Vec<_>>();
                names.sort_unstable();
                Err(format!(
                    "skill name `{requested}` is ambiguous; use one of: {}",
                    names.join(", ")
                ))
            }
        }
    }

    pub(crate) fn resolve_id(
        &self,
        requested: &str,
        revision: Option<&str>,
    ) -> Result<&Skill, String> {
        let skill = self
            .enabled()
            .find(|skill| skill.id == requested)
            .ok_or_else(|| format!("skill id `{requested}` is not available"))?;
        if let Some(revision) = revision {
            if skill.revision != revision {
                return Err(format!(
                    "skill `{}` changed from revision `{revision}` to `{}`; select it again before sending",
                    skill.name, skill.revision
                ));
            }
        }
        Ok(skill)
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
        Ok(contents)
    }

    pub(crate) fn name_counts(&self) -> HashMap<&str, usize> {
        let mut counts = HashMap::new();
        for skill in self.enabled() {
            *counts.entry(skill.base_name.as_str()).or_insert(0) += 1;
        }
        counts
    }

    pub(crate) fn snapshot(
        &self,
        environment_id: impl Into<String>,
        project_root: &Path,
    ) -> SkillCatalogSnapshot {
        let skills = self
            .skills
            .iter()
            .map(|skill| SkillCatalogEntry {
                id: skill.id.clone(),
                revision: skill.revision.clone(),
                name: skill.name.clone(),
                invocation_name: skill.invocation_name.clone(),
                description: skill.description.clone(),
                scope: skill.scope,
                origin: skill.origin,
                source: skill.locator(),
                required_tools: skill.required_tools.clone(),
                missing_tools: skill.missing_tools.clone(),
                allow_implicit_invocation: skill.allow_implicit_invocation,
                enabled: skill.enabled,
                disabled_reason: skill.disabled_reason.clone(),
                has_name_collision: skill.has_name_collision,
            })
            .collect::<Vec<_>>();
        let mut diagnostics = self.diagnostics.clone();
        diagnostics.extend(self.warnings.iter().map(|warning| {
            SkillDiagnostic::warning("discovery_warning", None::<String>, warning.clone())
        }));
        let environment_id = environment_id.into();
        let project_root = project_root.to_string_lossy().into_owned();
        let revision = catalog_revision(&skills, &diagnostics);
        SkillCatalogSnapshot {
            revision,
            environment_id,
            project_root,
            skills,
            diagnostics,
        }
    }
}

impl SkillOrigin {
    fn invocation_label(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Compatible => "compatible",
            Self::Claude => "claude",
            Self::Plugin => "plugin",
        }
    }
}

pub(crate) fn skill_identity(locator: &str) -> String {
    format!("skill_{}", &digest(&["skill-source", locator])[..24])
}

pub(crate) fn skill_revision(contents: &[u8]) -> String {
    format!("rev_{}", digest_bytes("skill-content", contents))
}

pub(crate) fn skill_revision_with_context(contents: &[u8], context: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"skill-content-v2\0");
    if let Some(context) = context {
        hasher.update(context.as_bytes());
    }
    hasher.update(b"\0");
    hasher.update(contents);
    format!("rev_{}", hex_digest(hasher.finalize().as_slice()))
}

fn catalog_revision(entries: &[SkillCatalogEntry], diagnostics: &[SkillDiagnostic]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"skill-catalog-v1\0");
    for entry in entries {
        for value in [
            entry.id.as_str(),
            entry.revision.as_str(),
            entry.invocation_name.as_str(),
            if entry.enabled { "enabled" } else { "disabled" },
        ] {
            hasher.update(value.as_bytes());
            hasher.update(b"\0");
        }
    }
    for diagnostic in diagnostics {
        hasher.update(diagnostic.code.as_bytes());
        hasher.update(b"\0");
        hasher.update(diagnostic.message.as_bytes());
        hasher.update(b"\0");
    }
    format!("catalog_{}", hex_digest(hasher.finalize().as_slice()))
}

fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hex_digest(hasher.finalize().as_slice())
}

fn digest_bytes(namespace: &str, contents: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update(b"\0");
    hasher.update(contents);
    hex_digest(hasher.finalize().as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn short_id(id: &str) -> &str {
    id.strip_prefix("skill_")
        .unwrap_or(id)
        .get(..8)
        .unwrap_or(id)
}

pub(crate) fn render_injection(skill: &Skill, contents: &str) -> String {
    format!(
        "[runtime skill — selected from the current skill catalog]\n<skill>\n<id>{}</id>\n<revision>{}</revision>\n<name>{}</name>\n{}\n</skill>",
        xml_escape(&skill.id),
        xml_escape(&skill.revision),
        xml_escape(&skill.invocation_name),
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
mod lossless_tests;
