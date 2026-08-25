use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// Deny networking to sandboxed child processes. Brokered host
    /// capabilities, such as brokered cloud, remain available through their
    /// typed application tools and never inherit shell access.
    Restricted,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxPreset {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxPreset {
    pub fn for_session_mode(mode: Option<&str>) -> Self {
        match mode {
            Some("plan" | "read-only") => Self::ReadOnly,
            Some("full" | "danger-full-access") => Self::DangerFullAccess,
            _ => Self::WorkspaceWrite,
        }
    }
}

/// Resolved policy for one local session. Empty read roots mean full-disk read;
/// writes always require an explicit root. Paths are canonicalized when the
/// policy is constructed, before any platform compiler sees them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub deny_read: Vec<PathBuf>,
    pub deny_write: Vec<PathBuf>,
    pub network: NetworkPolicy,
    /// Private per-session temporary directory advertised to child processes.
    /// It is also an allowed write root, avoiding a broad `/tmp` exemption.
    #[serde(default)]
    pub process_temp_root: Option<PathBuf>,
}

impl SandboxPolicy {
    /// Read-only policy: reads remain available across the host so
    /// compilers, package managers, and inspection commands can resolve their
    /// inputs, while every filesystem write is denied except separately
    /// attached product roots (for example the private process temp dir).
    pub fn read_only() -> Self {
        Self {
            read_roots: Vec::new(),
            write_roots: Vec::new(),
            deny_read: Vec::new(),
            deny_write: Vec::new(),
            network: NetworkPolicy::Restricted,
            process_temp_root: None,
        }
    }

    pub fn workspace_write(root: PathBuf, additional_write_roots: Vec<PathBuf>) -> Self {
        let root = canonical_or_normalized(&root);
        let mut write_roots = vec![root.clone()];
        write_roots.extend(
            additional_write_roots
                .iter()
                .map(|path| canonical_or_normalized(path)),
        );
        dedupe_roots(&mut write_roots);

        // Protect Git metadata even when it does not exist yet. Otherwise a
        // sandboxed command could create `.git` after session startup and gain
        // write access through the workspace allowance.
        let dot_git = root.join(".git");
        let mut deny_write = vec![dot_git.clone()];
        if dot_git.exists() {
            deny_write.push(canonical_or_normalized(&dot_git));
            if dot_git.is_file() {
                if let Ok(contents) = std::fs::read_to_string(&dot_git) {
                    if let Some(target) = contents.strip_prefix("gitdir:") {
                        let target = target.trim();
                        let resolved = if Path::new(target).is_absolute() {
                            PathBuf::from(target)
                        } else {
                            root.join(target)
                        };
                        deny_write.push(canonical_or_normalized(&resolved));
                    }
                }
            }
        }
        dedupe_roots(&mut deny_write);

        Self {
            read_roots: Vec::new(),
            write_roots,
            deny_read: Vec::new(),
            deny_write,
            network: NetworkPolicy::Restricted,
            process_temp_root: None,
        }
    }

    pub fn with_process_temp_root(mut self, root: PathBuf) -> Self {
        let root = canonical_or_normalized(&root);
        self.write_roots.push(root.clone());
        dedupe_roots(&mut self.write_roots);
        self.process_temp_root = Some(root);
        self
    }

    pub fn with_write_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        self.write_roots
            .extend(roots.into_iter().map(|root| canonical_or_normalized(&root)));
        dedupe_roots(&mut self.write_roots);
        self
    }

    pub fn check_read(&self, path: &Path) -> Result<PathBuf, String> {
        let resolved = path
            .canonicalize()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if inside_any(&resolved, &self.deny_read) {
            return Err(format!("sandbox denied read: {}", resolved.display()));
        }
        if self.read_roots.is_empty() || inside_any(&resolved, &self.read_roots) {
            Ok(resolved)
        } else {
            Err(format!("sandbox denied read: {}", resolved.display()))
        }
    }

    pub fn check_write(&self, path: &Path) -> Result<PathBuf, String> {
        if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(format!(
                "sandbox refused write through symlink: {}",
                path.display()
            ));
        }
        let resolved = resolve_write_target(path)?;
        if inside_any(&resolved, &self.deny_write) || inside_any(path, &self.deny_write) {
            return Err(format!("sandbox denied write: {}", path.display()));
        }
        if inside_any(&resolved, &self.write_roots) {
            Ok(resolved)
        } else {
            Err(format!("sandbox denied write: {}", path.display()))
        }
    }
}

fn resolve_write_target(path: &Path) -> Result<PathBuf, String> {
    let normalized = lexical_normalize(path);
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| format!("{} has no existing ancestor", path.display()))?;
        suffix.push(name.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| format!("{} has no existing ancestor", path.display()))?;
    }
    let mut resolved = ancestor
        .canonicalize()
        .map_err(|error| format!("{}: {error}", ancestor.display()))?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

pub(crate) fn canonical_or_normalized(path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|_| lexical_normalize(path))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn inside_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn dedupe_roots(roots: &mut Vec<PathBuf>) {
    roots.sort();
    roots.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_policy_allows_inside_and_denies_outside_writes() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let policy = SandboxPolicy::workspace_write(workspace.path().to_path_buf(), Vec::new());
        assert!(policy
            .check_write(&workspace.path().join("new.txt"))
            .is_ok());
        assert!(policy.check_write(&outside.path().join("new.txt")).is_err());
    }

    #[test]
    fn product_modes_map_to_explicit_sandbox_presets() {
        assert_eq!(
            SandboxPreset::for_session_mode(Some("plan")),
            SandboxPreset::ReadOnly
        );
        assert_eq!(
            SandboxPreset::for_session_mode(Some("auto")),
            SandboxPreset::WorkspaceWrite
        );
        assert_eq!(
            SandboxPreset::for_session_mode(Some("full")),
            SandboxPreset::DangerFullAccess
        );
    }

    #[test]
    fn read_only_keeps_host_reads_available_and_denies_writes() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("dependency.txt");
        std::fs::write(&outside_file, b"dependency").unwrap();
        let policy = SandboxPolicy::read_only();

        assert!(policy.check_read(&outside_file).is_ok());
        assert!(policy
            .check_write(&workspace.path().join("mutation.txt"))
            .is_err());
    }

    #[test]
    fn workspace_policy_protects_git_metadata_created_after_startup() {
        let workspace = tempfile::tempdir().unwrap();
        let policy = SandboxPolicy::workspace_write(workspace.path().to_path_buf(), Vec::new());

        assert!(policy
            .check_write(&workspace.path().join(".git/config"))
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn write_through_parent_symlink_is_denied() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("escape")).unwrap();
        let policy = SandboxPolicy::workspace_write(workspace.path().to_path_buf(), Vec::new());
        assert!(policy
            .check_write(&workspace.path().join("escape/new.txt"))
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_shared_cache_writes_are_allowed_inside_an_explicit_extra_root() {
        // The shared-build-cache shape: an in-project `target/` symlink whose
        // resolved target lives outside the workspace. Writes stay denied while
        // the cache root is ungranted and succeed once it is an explicit extra
        // write root (`.agent/settings.json` → policy wiring).
        let workspace = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(cache.path(), workspace.path().join("target")).unwrap();

        let granted = SandboxPolicy::workspace_write(
            workspace.path().to_path_buf(),
            vec![cache.path().to_path_buf()],
        );
        assert!(granted
            .check_write(&workspace.path().join("target/debug/blob"))
            .is_ok());

        let ungranted = SandboxPolicy::workspace_write(workspace.path().to_path_buf(), Vec::new());
        assert!(ungranted
            .check_write(&workspace.path().join("target/debug/blob"))
            .is_err());
    }
}
