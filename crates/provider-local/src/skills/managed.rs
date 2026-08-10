use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::loader::{self, valid_namespace, SkillRoot};
use super::{SkillCatalog, SkillDiagnostic, SkillDiagnosticSeverity, SkillOrigin, SkillScope};
use crate::exec::Executor;
use crate::markdown_frontmatter::resolve_home;

const REGISTRY_VERSION: u32 = 1;
const MAX_PACK_FILES: usize = 5_000;
const MAX_PACK_DIRECTORIES: usize = 2_000;
const MAX_PACK_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillPackScope {
    Project,
    User,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSkillPackRequest {
    pub pack_id: String,
    pub source_path: String,
    pub scope: SkillPackScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillPackAction {
    Installed,
    Updated,
    Unchanged,
    Uninstalled,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackReceipt {
    pub action: SkillPackAction,
    pub pack_id: String,
    pub revision: Option<String>,
    pub previous_revision: Option<String>,
    pub skill_count: usize,
    pub scope: SkillPackScope,
    pub install_root: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkillPack {
    pub pack_id: String,
    pub revision: String,
    pub source: String,
    pub skill_count: usize,
    pub scope: SkillPackScope,
    pub install_root: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct PackRegistry {
    version: u32,
    packs: BTreeMap<String, PackRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PackRecord {
    revision: String,
    source: String,
    skill_count: usize,
}

struct SourceFile {
    relative: PathBuf,
    bytes: Vec<u8>,
}

pub async fn install_skill_pack(
    exec: &dyn Executor,
    project_root: &Path,
    request: InstallSkillPackRequest,
) -> Result<SkillPackReceipt, String> {
    let pack_id = request.pack_id.trim().to_ascii_lowercase();
    if !valid_namespace(&pack_id) {
        return Err(
            "pack id must be lowercase hyphen-case, with no repeated or edge hyphens".into(),
        );
    }
    let source = exec
        .canonicalize(Path::new(request.source_path.trim()))
        .await
        .map_err(|error| format!("cannot open skill pack source: {error}"))?;
    let source = source_skills_root(exec, &source).await;
    let base = pack_storage_root(exec, project_root, request.scope).await?;
    if source.starts_with(&base) {
        return Err("skill pack source cannot be inside Clark Code's managed pack storage".into());
    }

    let files = collect_source_files(exec, &source).await?;
    let revision = pack_revision(&files);
    let pack_root = base.join("packs").join(&pack_id);
    let final_root = pack_root.join("revisions").join(&revision);
    let staging = pack_root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    let final_exists = exec
        .metadata(&final_root)
        .await
        .is_ok_and(|meta| meta.is_dir);

    if !final_exists {
        if let Err(error) = write_staging(exec, &staging, &files).await {
            let _ = exec.remove_dir_all(&staging).await;
            return Err(error);
        }
        let validation = validate_pack(exec, &staging, request.scope, &pack_id, &revision).await;
        if let Err(error) = validation {
            let _ = exec.remove_dir_all(&staging).await;
            return Err(error);
        }
        if let Err(error) = exec.rename(&staging, &final_root).await {
            let _ = exec.remove_dir_all(&staging).await;
            return Err(format!(
                "could not activate validated skill pack files: {error}"
            ));
        }
    }

    let validation = validate_pack(exec, &final_root, request.scope, &pack_id, &revision).await?;
    let mut registry = read_registry(exec, &base).await?;
    let previous = registry.packs.get(&pack_id).cloned();
    registry.version = REGISTRY_VERSION;
    registry.packs.insert(
        pack_id.clone(),
        PackRecord {
            revision: revision.clone(),
            source: source.to_string_lossy().into_owned(),
            skill_count: validation.skills.len(),
        },
    );
    write_registry(exec, &base, &registry).await?;

    let action = match previous.as_ref() {
        None => SkillPackAction::Installed,
        Some(record) if record.revision == revision => SkillPackAction::Unchanged,
        Some(_) => SkillPackAction::Updated,
    };
    Ok(SkillPackReceipt {
        action,
        pack_id,
        revision: Some(revision),
        previous_revision: previous.map(|record| record.revision),
        skill_count: validation.skills.len(),
        scope: request.scope,
        install_root: final_root.to_string_lossy().into_owned(),
        warnings: Vec::new(),
    })
}

pub async fn uninstall_skill_pack(
    exec: &dyn Executor,
    project_root: &Path,
    pack_id: &str,
    scope: SkillPackScope,
) -> Result<SkillPackReceipt, String> {
    let pack_id = pack_id.trim().to_ascii_lowercase();
    if !valid_namespace(&pack_id) {
        return Err("invalid skill pack id".into());
    }
    let base = pack_storage_root(exec, project_root, scope).await?;
    let mut registry = read_registry(exec, &base).await?;
    let previous = registry
        .packs
        .remove(&pack_id)
        .ok_or_else(|| format!("skill pack `{pack_id}` is not installed in this scope"))?;
    registry.version = REGISTRY_VERSION;
    write_registry(exec, &base, &registry).await?;

    let pack_root = base.join("packs").join(&pack_id);
    let mut warnings = Vec::new();
    if let Err(error) = exec.remove_dir_all(&pack_root).await {
        warnings.push(format!(
            "Pack is inactive, but old version files could not be removed: {error}"
        ));
    }
    Ok(SkillPackReceipt {
        action: SkillPackAction::Uninstalled,
        pack_id,
        revision: None,
        previous_revision: Some(previous.revision),
        skill_count: previous.skill_count,
        scope,
        install_root: pack_root.to_string_lossy().into_owned(),
        warnings,
    })
}

pub async fn list_skill_packs(
    exec: &dyn Executor,
    project_root: &Path,
) -> Result<Vec<InstalledSkillPack>, String> {
    let home = resolve_home(exec, project_root).await;
    let mut packs = Vec::new();
    for (scope, base) in pack_bases(project_root, home.as_deref()) {
        let registry = read_registry(exec, &base).await?;
        for (pack_id, record) in registry.packs {
            packs.push(InstalledSkillPack {
                install_root: active_root(&base, &pack_id, &record.revision)
                    .to_string_lossy()
                    .into_owned(),
                pack_id,
                revision: record.revision,
                source: record.source,
                skill_count: record.skill_count,
                scope,
            });
        }
    }
    packs.sort_by(|left, right| {
        scope_rank(left.scope)
            .cmp(&scope_rank(right.scope))
            .then_with(|| left.pack_id.cmp(&right.pack_id))
    });
    Ok(packs)
}

pub(super) async fn active_roots(
    exec: &dyn Executor,
    project_root: &Path,
    home: Option<&Path>,
    diagnostics: &mut Vec<SkillDiagnostic>,
) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    for (scope, base) in pack_bases(project_root, home) {
        match read_registry(exec, &base).await {
            Ok(registry) => {
                for (pack_id, record) in registry.packs {
                    roots.push(SkillRoot {
                        path: active_root(&base, &pack_id, &record.revision),
                        scope: match scope {
                            SkillPackScope::Project => SkillScope::Project,
                            SkillPackScope::User => SkillScope::User,
                        },
                        origin: SkillOrigin::Bundled,
                        namespace: None,
                        identity_namespace: Some(format!(
                            "managed:{}:{pack_id}",
                            scope_label(scope)
                        )),
                        revision_context: Some(record.revision),
                    });
                }
            }
            Err(error) => diagnostics.push(SkillDiagnostic {
                severity: SkillDiagnosticSeverity::Error,
                code: "managed_pack_registry".into(),
                message: error,
                source: Some(registry_path(&base).to_string_lossy().into_owned()),
            }),
        }
    }
    roots
}

async fn validate_pack(
    exec: &dyn Executor,
    root: &Path,
    scope: SkillPackScope,
    pack_id: &str,
    revision: &str,
) -> Result<SkillCatalog, String> {
    let catalog = loader::discover_root(
        exec,
        SkillRoot {
            path: root.to_path_buf(),
            scope: match scope {
                SkillPackScope::Project => SkillScope::Project,
                SkillPackScope::User => SkillScope::User,
            },
            origin: SkillOrigin::Bundled,
            namespace: None,
            identity_namespace: Some(format!("managed:{}:{pack_id}", scope_label(scope))),
            revision_context: Some(revision.to_string()),
        },
    )
    .await;
    if catalog.skills.is_empty() {
        return Err("skill pack contains no valid SKILL.md files".into());
    }
    if !catalog.diagnostics.is_empty() || !catalog.warnings.is_empty() {
        let messages = catalog
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .chain(catalog.warnings.iter().map(String::as_str))
            .collect::<Vec<_>>();
        return Err(format!(
            "skill pack validation failed: {}",
            messages.join("; ")
        ));
    }
    Ok(catalog)
}

async fn source_skills_root(exec: &dyn Executor, source: &Path) -> PathBuf {
    let skills = source.join("skills");
    if exec.metadata(&skills).await.is_ok_and(|meta| meta.is_dir) {
        exec.canonicalize(&skills)
            .await
            .unwrap_or_else(|_| source.to_path_buf())
    } else {
        source.to_path_buf()
    }
}

async fn collect_source_files(
    exec: &dyn Executor,
    source: &Path,
) -> Result<Vec<SourceFile>, String> {
    let mut stack = vec![source.to_path_buf()];
    let mut paths = Vec::new();
    let mut directories = 0usize;
    while let Some(directory) = stack.pop() {
        directories += 1;
        if directories > MAX_PACK_DIRECTORIES {
            return Err("skill pack exceeds the directory safety limit".into());
        }
        let mut entries = exec
            .read_dir(&directory)
            .await
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        for entry in entries {
            if entry.name == ".git" {
                continue;
            }
            let path = directory.join(&entry.name);
            if entry.is_symlink {
                return Err(format!(
                    "{}: managed skill packs cannot contain symlinks",
                    path.display()
                ));
            }
            if entry.is_dir {
                stack.push(path);
            } else {
                paths.push(path);
                if paths.len() > MAX_PACK_FILES {
                    return Err("skill pack exceeds the file safety limit".into());
                }
            }
        }
    }
    paths.sort();
    let mut total = 0usize;
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = exec
            .read(&path)
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        total = total.saturating_add(bytes.len());
        if total > MAX_PACK_BYTES {
            return Err("skill pack exceeds the 32 MiB safety limit".into());
        }
        let relative = path
            .strip_prefix(source)
            .map_err(|_| "skill pack source identity changed during import")?
            .to_path_buf();
        files.push(SourceFile { relative, bytes });
    }
    if !files.iter().any(|file| {
        file.relative
            .file_name()
            .is_some_and(|name| name == "SKILL.md")
    }) {
        return Err("skill pack contains no SKILL.md files".into());
    }
    Ok(files)
}

async fn write_staging(
    exec: &dyn Executor,
    staging: &Path,
    files: &[SourceFile],
) -> Result<(), String> {
    exec.create_dir_all(staging).await?;
    for file in files {
        exec.write(&staging.join(&file.relative), &file.bytes)
            .await
            .map_err(|error| format!("{}: {error}", file.relative.display()))?;
    }
    Ok(())
}

fn pack_revision(files: &[SourceFile]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"agent-skill-pack-v1\0");
    for file in files {
        hasher.update(file.relative.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        hasher.update(&file.bytes);
        hasher.update(b"\0");
    }
    format!("pack_{}", hex_digest(hasher.finalize().as_slice()))
}

async fn pack_storage_root(
    exec: &dyn Executor,
    project_root: &Path,
    scope: SkillPackScope,
) -> Result<PathBuf, String> {
    Ok(match scope {
        SkillPackScope::Project => project_root.join(".agent/skill-packs"),
        SkillPackScope::User => resolve_home(exec, project_root)
            .await
            .ok_or_else(|| "could not resolve the target environment home directory".to_string())?
            .join(".agent/skill-packs"),
    })
}

fn pack_bases(project_root: &Path, home: Option<&Path>) -> Vec<(SkillPackScope, PathBuf)> {
    let mut bases = vec![(
        SkillPackScope::Project,
        project_root.join(".agent/skill-packs"),
    )];
    if let Some(home) = home {
        bases.push((SkillPackScope::User, home.join(".agent/skill-packs")));
    }
    bases
}

fn registry_path(base: &Path) -> PathBuf {
    base.join("installed.json")
}

fn active_root(base: &Path, pack_id: &str, revision: &str) -> PathBuf {
    base.join("packs")
        .join(pack_id)
        .join("revisions")
        .join(revision)
}

async fn read_registry(exec: &dyn Executor, base: &Path) -> Result<PackRegistry, String> {
    let path = registry_path(base);
    let bytes = match exec.read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return Ok(PackRegistry::default()),
    };
    let registry: PackRegistry = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{}: invalid managed pack registry: {error}", path.display()))?;
    if registry.version != 0 && registry.version != REGISTRY_VERSION {
        return Err(format!(
            "{}: unsupported managed pack registry version {}",
            path.display(),
            registry.version
        ));
    }
    Ok(registry)
}

async fn write_registry(
    exec: &dyn Executor,
    base: &Path,
    registry: &PackRegistry,
) -> Result<(), String> {
    exec.create_dir_all(base).await?;
    let path = registry_path(base);
    let temporary = base.join(format!(".installed-{}.json", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("could not serialize managed pack registry: {error}"))?;
    exec.write(&temporary, &bytes).await?;
    if let Err(error) = exec.rename(&temporary, &path).await {
        let _ = exec.remove_file(&temporary).await;
        return Err(format!(
            "could not atomically publish managed pack registry: {error}"
        ));
    }
    Ok(())
}

fn scope_label(scope: SkillPackScope) -> &'static str {
    match scope {
        SkillPackScope::Project => "project",
        SkillPackScope::User => "user",
    }
}

fn scope_rank(scope: SkillPackScope) -> u8 {
    match scope {
        SkillPackScope::Project => 0,
        SkillPackScope::User => 1,
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
