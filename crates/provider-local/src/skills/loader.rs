use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::bundled;
use super::{Skill, SkillCatalog, SkillOrigin, SkillResource, SkillScope};
use crate::exec::Executor;
use crate::markdown_frontmatter::{frontmatter, read_text, resolve_home};

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
    };

    let mut roots = ordinary_roots(project_root, home);
    roots.extend(super::plugin::roots(exec, project_root, home, &mut catalog.warnings).await);
    let mut seen_roots = HashSet::new();
    roots.retain(|root| seen_roots.insert(root.path.clone()));

    for root in roots {
        scan_root(exec, &root, &mut catalog).await;
    }
    catalog.sort_and_dedupe();
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
    }
}

async fn scan_root(exec: &dyn Executor, root: &SkillRoot, catalog: &mut SkillCatalog) {
    let mut stack = vec![(root.path.clone(), 0usize)];
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
            if entry.is_dir {
                if !entry.name.starts_with('.') && depth < MAX_SCAN_DEPTH {
                    stack.push((directory.join(entry.name), depth + 1));
                }
                continue;
            }
            if entry.name != "SKILL.md" || budget.skills >= MAX_SKILLS_PER_ROOT {
                truncated |= budget.skills >= MAX_SKILLS_PER_ROOT;
                continue;
            }
            let path = directory.join(entry.name);
            match load_skill(exec, root, path).await {
                Ok(skill) => {
                    catalog.skills.push(skill);
                    budget.skills += 1;
                }
                Err(message) => catalog.warnings.push(message),
            }
        }
        if truncated {
            break;
        }
    }
    if truncated {
        catalog.warnings.push(format!(
            "{}: skills scan reached its traversal limit",
            root.path.display()
        ));
    }
}

async fn load_skill(exec: &dyn Executor, root: &SkillRoot, path: PathBuf) -> Result<Skill, String> {
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
    let metadata = load_metadata(exec, &path).await;
    Ok(Skill {
        name,
        base_name,
        description,
        scope: root.scope,
        origin: root.origin,
        resource: SkillResource::ExecutorFile(path),
        required_tools: metadata.required_tools,
        allow_implicit_invocation: metadata.allow_implicit_invocation,
        enabled: true,
    })
}

struct LoadedMetadata {
    required_tools: Vec<String>,
    allow_implicit_invocation: bool,
}

async fn load_metadata(exec: &dyn Executor, skill_path: &Path) -> LoadedMetadata {
    let default = || LoadedMetadata {
        required_tools: Vec::new(),
        allow_implicit_invocation: true,
    };
    let Some(directory) = skill_path.parent() else {
        return default();
    };
    let Some(text) = read_text(exec, &directory.join("agents/openai.yaml")).await else {
        return default();
    };
    let Ok(metadata) = serde_yaml::from_str::<SkillMetadataFile>(&text) else {
        return default();
    };
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
    LoadedMetadata {
        required_tools,
        allow_implicit_invocation: metadata.policy.allow_implicit_invocation.unwrap_or(true),
    }
}

pub(super) fn parse_bundled_skill(
    namespace: &str,
    locator: &'static str,
    contents: &'static str,
) -> Skill {
    let frontmatter = frontmatter(contents).expect("bundled skill must have frontmatter");
    let parsed = parse_frontmatter(frontmatter).expect("bundled skill frontmatter must be valid");
    let base_name = parsed.name.as_deref().unwrap_or("skill").trim().to_string();
    assert!(
        valid_skill_name(&base_name),
        "bundled skill name must be valid"
    );
    Skill {
        name: format!("{namespace}:{base_name}"),
        base_name,
        description: sanitize(&parsed.description, MAX_DESCRIPTION_CHARS),
        scope: SkillScope::Bundled,
        origin: SkillOrigin::Clark,
        resource: SkillResource::Embedded { locator, contents },
        required_tools: vec!["bash".to_string()],
        allow_implicit_invocation: true,
        enabled: true,
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
