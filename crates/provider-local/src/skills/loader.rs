use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::bundled;
use super::{
    skill_identity, skill_revision, skill_revision_with_context, Skill, SkillCatalog, SkillOrigin,
    SkillResource, SkillScope,
};
use crate::exec::Executor;
use crate::markdown_frontmatter::{frontmatter, resolve_home};

const MAX_SCAN_DEPTH: usize = 6;
const MAX_DIRECTORIES_PER_ROOT: usize = 2_000;
const MAX_ENTRIES_PER_ROOT: usize = 20_000;
const MAX_SKILLS_PER_ROOT: usize = 512;
const MAX_SKILL_FILE_BYTES: usize = 256 * 1024;
pub(super) const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 1_024;

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: Option<String>,
    description: String,
}

#[derive(Debug, Default, Deserialize)]
struct SkillMetadataFile {
    #[serde(default)]
    dependencies: Dependencies,
    #[serde(default)]
    policy: Policy,
}

#[derive(Debug, Default, Deserialize)]
struct Dependencies {
    #[serde(default)]
    tools: Vec<ToolDependency>,
}

#[derive(Debug, Deserialize)]
struct ToolDependency {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    value: String,
}

#[derive(Debug, Default, Deserialize)]
struct Policy {
    #[serde(default)]
    allow_implicit_invocation: Option<bool>,
}

#[derive(Clone)]
pub(super) struct SkillRoot {
    pub(super) path: PathBuf,
    pub(super) scope: SkillScope,
    pub(super) origin: SkillOrigin,
    pub(super) namespace: Option<String>,
    pub(super) identity_namespace: Option<String>,
    pub(super) revision_context: Option<String>,
}

struct ScanBudget {
    directories: usize,
    entries: usize,
    skills: usize,
}

pub(crate) async fn discover_catalog(exec: &dyn Executor, project_root: &Path) -> SkillCatalog {
    let home = resolve_home(exec, project_root).await;
    discover_catalog_with_home(exec, project_root, home.as_deref()).await
}

pub(crate) async fn discover_catalog_with_home(
    exec: &dyn Executor,
    project_root: &Path,
    home: Option<&Path>,
) -> SkillCatalog {
    let mut catalog = SkillCatalog {
        skills: bundled::skills(),
        warnings: Vec::new(),
        diagnostics: Vec::new(),
    };

    let mut roots = ordinary_roots(project_root, home);
    roots.extend(super::plugin::roots(exec, project_root, home, &mut catalog.warnings).await);
    roots.extend(
        super::managed::active_roots(exec, project_root, home, &mut catalog.diagnostics).await,
    );
    let mut seen_roots = HashSet::new();
    roots.retain(|root| seen_roots.insert(root.path.clone()));

    for root in roots {
        scan_root(exec, &root, &mut catalog).await;
    }
    catalog.sort_and_finalize();
    catalog
}

fn ordinary_roots(project_root: &Path, home: Option<&Path>) -> Vec<SkillRoot> {
    let mut roots = vec![
        root(
            project_root.join(".clark/skills"),
            SkillScope::Project,
            SkillOrigin::Clark,
        ),
        root(
            project_root.join(".agents/skills"),
            SkillScope::Project,
            SkillOrigin::Compatible,
        ),
        root(
            project_root.join(".codex/skills"),
            SkillScope::Project,
            SkillOrigin::Compatible,
        ),
        root(
            project_root.join(".claude/skills"),
            SkillScope::Project,
            SkillOrigin::Claude,
        ),
    ];
    if let Some(home) = home {
        roots.extend([
            root(
                home.join(".clark/skills"),
                SkillScope::User,
                SkillOrigin::Clark,
            ),
            root(
                home.join(".agents/skills"),
                SkillScope::User,
                SkillOrigin::Compatible,
            ),
            root(
                home.join(".codex/skills"),
                SkillScope::User,
                SkillOrigin::Compatible,
            ),
            root(
                home.join(".claude/skills"),
                SkillScope::User,
                SkillOrigin::Claude,
            ),
        ]);
    }
    roots
}

fn root(path: PathBuf, scope: SkillScope, origin: SkillOrigin) -> SkillRoot {
    SkillRoot {
        path,
        scope,
        origin,
        namespace: None,
        identity_namespace: None,
        revision_context: None,
    }
}

async fn scan_root(exec: &dyn Executor, root: &SkillRoot, catalog: &mut SkillCatalog) {
    let Ok(canonical_root) = exec.canonicalize(&root.path).await else {
        // Most conventional roots do not exist.
        return;
    };
    let mut stack = vec![(canonical_root.clone(), 0usize)];
    let mut seen_directories = HashSet::new();
    let mut budget = ScanBudget {
        directories: 0,
        entries: 0,
        skills: 0,
    };
    let mut truncated = false;
    while let Some((directory, depth)) = stack.pop() {
        if depth > MAX_SCAN_DEPTH || budget.directories >= MAX_DIRECTORIES_PER_ROOT {
            truncated = true;
            break;
        }
        let directory = match exec.canonicalize(&directory).await {
            Ok(directory) => directory,
            Err(error) => {
                catalog.warning(
                    "directory_unreadable",
                    Some(directory.to_string_lossy()),
                    format!("Cannot follow skill directory: {error}"),
                );
                continue;
            }
        };
        if !seen_directories.insert(directory.clone()) {
            continue;
        }
        budget.directories += 1;
        let Ok(mut entries) = exec.read_dir(&directory).await else {
            continue;
        };
        entries.sort_by(|left, right| right.name.cmp(&left.name));
        for entry in entries {
            if budget.entries >= MAX_ENTRIES_PER_ROOT {
                truncated = true;
                break;
            }
            budget.entries += 1;
            let path = directory.join(&entry.name);
            if entry.is_dir || entry.is_symlink {
                if !entry.name.starts_with('.') && depth < MAX_SCAN_DEPTH {
                    match exec.canonicalize(&path).await {
                        Ok(target)
                            if exec
                                .metadata(&target)
                                .await
                                .is_ok_and(|metadata| metadata.is_dir) =>
                        {
                            stack.push((target, depth + 1));
                        }
                        Ok(_) if entry.is_symlink && entry.name == "SKILL.md" => {
                            catalog.warning(
                                "symlinked_skill_file",
                                Some(path.to_string_lossy()),
                                "Symlinked skill files are ignored; link the containing directory instead",
                            );
                        }
                        Ok(_) => {}
                        Err(error) if entry.is_symlink => catalog.warning(
                            "symlink_target_unavailable",
                            Some(path.to_string_lossy()),
                            format!("Cannot follow skill directory symlink: {error}"),
                        ),
                        Err(_) => {}
                    }
                }
                continue;
            }
            if entry.name != "SKILL.md" || budget.skills >= MAX_SKILLS_PER_ROOT {
                truncated |= budget.skills >= MAX_SKILLS_PER_ROOT;
                continue;
            }
            let path = match exec.canonicalize(&path).await {
                Ok(path) => path,
                Err(error) => {
                    catalog.warning(
                        "source_unavailable",
                        Some(path.to_string_lossy()),
                        format!("Cannot resolve skill source: {error}"),
                    );
                    continue;
                }
            };
            match load_skill(exec, root, &canonical_root, path).await {
                Ok(skill) => {
                    catalog.skills.push(skill);
                    budget.skills += 1;
                }
                Err(message) => catalog.warning("invalid_skill", None::<String>, message),
            }
        }
        if truncated {
            break;
        }
    }
    if truncated {
        catalog.warning(
            "traversal_limit",
            Some(root.path.to_string_lossy()),
            "Skills scan reached its traversal limit",
        );
    }
}

pub(super) async fn discover_root(exec: &dyn Executor, root: SkillRoot) -> SkillCatalog {
    let mut catalog = SkillCatalog::default();
    scan_root(exec, &root, &mut catalog).await;
    catalog.sort_and_finalize();
    catalog
}

async fn load_skill(
    exec: &dyn Executor,
    root: &SkillRoot,
    canonical_root: &Path,
    path: PathBuf,
) -> Result<Skill, String> {
    let bytes = exec
        .read(&path)
        .await
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() > MAX_SKILL_FILE_BYTES {
        return Err(format!(
            "{}: skill file exceeds {MAX_SKILL_FILE_BYTES} bytes",
            path.display()
        ));
    }
    let revision = skill_revision_with_context(&bytes, root.revision_context.as_deref());
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("{}: skill file is not UTF-8", path.display()))?;
    let frontmatter = frontmatter(&text)
        .ok_or_else(|| format!("{}: missing YAML frontmatter", path.display()))?;
    let parsed = parse_frontmatter(frontmatter)
        .map_err(|error| format!("{}: invalid skill frontmatter: {error}", path.display()))?;
    let fallback_name = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "skill".to_string());
    let base_name = parsed
        .name
        .as_deref()
        .unwrap_or(&fallback_name)
        .trim()
        .to_string();
    let description = sanitize(&parsed.description, MAX_DESCRIPTION_CHARS);
    if !valid_skill_name(&base_name) {
        return Err(format!(
            "{}: skill name must contain only letters, digits, hyphens, or underscores and be at most {MAX_NAME_CHARS} characters",
            path.display()
        ));
    }
    if description.is_empty() {
        return Err(format!("{}: skill description is empty", path.display()));
    }
    let name = root
        .namespace
        .as_ref()
        .map(|namespace| format!("{namespace}:{base_name}"))
        .unwrap_or_else(|| base_name.clone());
    let metadata = load_metadata(exec, &path)
        .await
        .map_err(|error| format!("{}: invalid agents/openai.yaml: {error}", path.display()))?;
    let locator = path.to_string_lossy().into_owned();
    let identity_locator = root
        .identity_namespace
        .as_ref()
        .and_then(|namespace| {
            path.strip_prefix(canonical_root)
                .ok()
                .map(|relative| format!("{namespace}:{}", relative.display()))
        })
        .unwrap_or_else(|| locator.clone());
    let id = skill_identity(&identity_locator);
    Ok(Skill {
        id,
        revision,
        invocation_name: name.clone(),
        name,
        base_name,
        description,
        scope: root.scope,
        origin: root.origin,
        resource: SkillResource::ExecutorFile(path),
        required_tools: metadata.required_tools,
        allow_implicit_invocation: metadata.allow_implicit_invocation,
        enabled: true,
        missing_tools: Vec::new(),
        disabled_reason: None,
        has_name_collision: false,
    })
}

struct LoadedMetadata {
    required_tools: Vec<String>,
    allow_implicit_invocation: bool,
}

async fn load_metadata(exec: &dyn Executor, skill_path: &Path) -> Result<LoadedMetadata, String> {
    let default = || LoadedMetadata {
        required_tools: Vec::new(),
        allow_implicit_invocation: true,
    };
    let Some(directory) = skill_path.parent() else {
        return Ok(default());
    };
    let path = directory.join("agents/openai.yaml");
    if exec.metadata(&path).await.is_err() {
        return Ok(default());
    }
    let bytes = exec.read(&path).await?;
    let text = String::from_utf8(bytes).map_err(|_| "metadata is not UTF-8".to_string())?;
    let metadata =
        serde_yaml::from_str::<SkillMetadataFile>(&text).map_err(|error| error.to_string())?;
    let mut required_tools = metadata
        .dependencies
        .tools
        .into_iter()
        .filter(|tool| tool.kind.is_empty() || tool.kind == "tool")
        .map(|tool| tool.value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    required_tools.sort_unstable();
    required_tools.dedup();
    Ok(LoadedMetadata {
        required_tools,
        allow_implicit_invocation: metadata.policy.allow_implicit_invocation.unwrap_or(true),
    })
}

pub(super) struct BundledSkillSpec {
    pub namespace: &'static str,
    pub locator: &'static str,
    pub contents: &'static str,
    pub required_tools: &'static [&'static str],
    pub allow_implicit_invocation: bool,
}

pub(super) fn parse_bundled_skill(spec: BundledSkillSpec) -> Skill {
    let frontmatter = frontmatter(spec.contents).expect("bundled skill must have frontmatter");
    let parsed = parse_frontmatter(frontmatter).expect("bundled skill frontmatter must be valid");
    let base_name = parsed.name.as_deref().unwrap_or("skill").trim().to_string();
    assert!(
        valid_skill_name(&base_name),
        "bundled skill name must be valid"
    );
    let name = format!("{}:{base_name}", spec.namespace);
    Skill {
        id: skill_identity(spec.locator),
        revision: skill_revision(spec.contents.as_bytes()),
        invocation_name: name.clone(),
        name,
        base_name,
        description: sanitize(&parsed.description, MAX_DESCRIPTION_CHARS),
        scope: SkillScope::Bundled,
        origin: SkillOrigin::Clark,
        resource: SkillResource::Embedded {
            locator: spec.locator,
            contents: spec.contents,
        },
        required_tools: spec
            .required_tools
            .iter()
            .map(|tool| (*tool).to_string())
            .collect(),
        allow_implicit_invocation: spec.allow_implicit_invocation,
        enabled: true,
        missing_tools: Vec::new(),
        disabled_reason: None,
        has_name_collision: false,
    }
}

fn sanitize(value: &str, max_chars: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn valid_skill_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_NAME_CHARS
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

pub(super) fn valid_namespace(value: &str) -> bool {
    valid_skill_name(value)
        && value == value.to_ascii_lowercase()
        && !value.contains('_')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
}

fn parse_frontmatter(value: &str) -> Result<SkillFrontmatter, String> {
    match serde_yaml::from_str(value) {
        Ok(parsed) => Ok(parsed),
        Err(yaml_error) => {
            // Repair common third-party scalar prose such as
            // `description: Deploy to AWS: ECS`. Clark's narrow fallback reads
            // only the two supported fields and leaves unrelated invalid YAML
            // rejected.
            let repaired = repair_scalar_fields(value).ok_or_else(|| yaml_error.to_string())?;
            serde_yaml::from_str(&repaired).map_err(|_| yaml_error.to_string())
        }
    }
}

fn repair_scalar_fields(value: &str) -> Option<String> {
    let mut changed = false;
    let lines = value
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let indentation = &line[..line.len() - trimmed.len()];
            for field in ["name", "description"] {
                let Some(raw) = trimmed.strip_prefix(&format!("{field}:")) else {
                    continue;
                };
                let raw = raw.trim();
                if raw.is_empty() {
                    return line.to_string();
                }
                changed = true;
                let quoted = serde_json::to_string(raw).expect("a string is JSON serializable");
                return format!("{indentation}{field}: {quoted}");
            }
            line.to_string()
        })
        .collect::<Vec<_>>();
    changed.then(|| lines.join("\n"))
}
