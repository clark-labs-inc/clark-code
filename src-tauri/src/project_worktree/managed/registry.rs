use std::{
    fs,
    fs::OpenOptions,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::time::{sleep, Instant};
use uuid::Uuid;

use super::super::git_output;
use super::types::{registry_version, ManagedWorktreeRegistry};

const MANAGED_REGISTRY_FILE: &str = "agent-managed-worktrees-v1.json";
const LEGACY_MANAGED_REGISTRY_FILE: &str = "clark-managed-worktrees-v1.json";
const REGISTRY_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const REGISTRY_LOCK_RETRY: Duration = Duration::from_millis(25);
const STALE_REGISTRY_LOCK_AGE: Duration = Duration::from_secs(10 * 60);

pub(super) struct RegistryLock {
    path: PathBuf,
    /// Random token written into the lock file at creation. Release must prove
    /// the file is still ours: after a staleness steal, the original holder's
    /// unconditional `remove_file` was deleting the *stealer's* live lock,
    /// letting a third writer in and losing registry entries.
    owner: String,
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).is_ok_and(|content| content == self.owner) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) async fn common_git_dir(repo_root: &Path) -> Result<PathBuf, String> {
    let common = git_output(
        repo_root,
        vec![
            "rev-parse".into(),
            "--path-format=absolute".into(),
            "--git-common-dir".into(),
        ],
        "Find shared Git directory",
    )
    .await?;
    PathBuf::from(common)
        .canonicalize()
        .map_err(|error| format!("Shared Git directory is unavailable: {error}"))
}

pub(super) fn registry_path(common_git_dir: &Path) -> PathBuf {
    common_git_dir.join(MANAGED_REGISTRY_FILE)
}

pub(super) async fn is_registered_managed_worktree(root: &Path) -> Result<bool, String> {
    let registry_path = registry_path(&common_git_dir(root).await?);
    let _registry_lock = acquire_registry_lock(&registry_path).await?;
    let registry = read_registry(&registry_path)?;
    Ok(registry.entries.iter().any(|entry| {
        PathBuf::from(&entry.path)
            .canonicalize()
            .is_ok_and(|path| path == root)
    }))
}

pub(super) async fn acquire_registry_lock(registry_path: &Path) -> Result<RegistryLock, String> {
    let lock_path = registry_lock_path(registry_path)?;
    let deadline = Instant::now() + REGISTRY_LOCK_TIMEOUT;
    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                let owner = Uuid::new_v4().to_string();
                if let Err(error) = std::io::Write::write_all(&mut file, owner.as_bytes()) {
                    drop(file);
                    let _ = fs::remove_file(&lock_path);
                    return Err(format!(
                        "Write managed-worktree lifecycle lock {}: {error}",
                        lock_path.display()
                    ));
                }
                drop(file);
                return Ok(RegistryLock {
                    path: lock_path,
                    owner,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                if registry_lock_is_stale(&lock_path) {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "Another managed-worktree lifecycle action is already running for {}.",
                        registry_path.display()
                    ));
                }
                sleep(REGISTRY_LOCK_RETRY).await;
            }
            Err(error) => {
                return Err(format!(
                    "Create managed-worktree lifecycle lock {}: {error}",
                    lock_path.display()
                ));
            }
        }
    }
}

fn registry_lock_path(registry_path: &Path) -> Result<PathBuf, String> {
    let name = registry_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Managed-worktree registry has no usable filename.".to_string())?;
    Ok(registry_path.with_file_name(format!("{name}.lock")))
}

fn registry_lock_is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
        .is_ok_and(|age| age > STALE_REGISTRY_LOCK_AGE)
}

pub(super) fn read_registry(path: &Path) -> Result<ManagedWorktreeRegistry, String> {
    let mut registry = read_registry_file(path)?;
    if path.file_name().and_then(|name| name.to_str()) == Some(MANAGED_REGISTRY_FILE) {
        let legacy_path = path.with_file_name(LEGACY_MANAGED_REGISTRY_FILE);
        let legacy = read_registry_file(&legacy_path)?;
        for entry in legacy.entries {
            let duplicate = registry
                .entries
                .iter()
                .any(|current| current.id == entry.id || current.path == entry.path);
            if !duplicate {
                registry.entries.push(entry);
            }
        }
    }
    Ok(registry)
}

fn read_registry_file(path: &Path) -> Result<ManagedWorktreeRegistry, String> {
    let Ok(bytes) = fs::read(path) else {
        return if path.exists() {
            Err(format!(
                "Read managed-worktree registry {} failed.",
                path.display()
            ))
        } else {
            Ok(ManagedWorktreeRegistry {
                version: registry_version(),
                entries: Vec::new(),
            })
        };
    };
    let registry: ManagedWorktreeRegistry = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "Managed-worktree registry {} is unreadable: {error}",
            path.display()
        )
    })?;
    if registry.version != registry_version() {
        return Err(format!(
            "Managed-worktree registry {} has unsupported version {}.",
            path.display(),
            registry.version
        ));
    }
    Ok(registry)
}

pub(super) fn write_registry(
    path: &Path,
    registry: &ManagedWorktreeRegistry,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("Serialize managed-worktree registry: {error}"))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Managed-worktree registry has no usable filename.".to_string())?;
    let temporary = path.with_file_name(format!("{filename}.{}.tmp", Uuid::new_v4().simple()));
    fs::write(&temporary, bytes).map_err(|error| {
        format!(
            "Write temporary managed-worktree registry {}: {error}",
            temporary.display()
        )
    })?;
    if let Err(first_error) = fs::rename(&temporary, path) {
        if first_error.kind() != ErrorKind::AlreadyExists {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "Replace managed-worktree registry {}: {first_error}",
                path.display()
            ));
        }
        // Windows does not replace an existing destination with rename. Every
        // caller holds the lifecycle lock, so this brief replacement is still
        // serialized with all Clark Code reads and writes of the registry.
        fs::remove_file(path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            format!(
                "Replace managed-worktree registry {} after {first_error}: {error}",
                path.display()
            )
        })?;
        fs::rename(&temporary, path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            format!(
                "Replace managed-worktree registry {}: {error}",
                path.display()
            )
        })?;
    }
    if path.file_name().and_then(|name| name.to_str()) == Some(MANAGED_REGISTRY_FILE) {
        let legacy_path = path.with_file_name(LEGACY_MANAGED_REGISTRY_FILE);
        if let Err(error) = fs::remove_file(&legacy_path) {
            if error.kind() != ErrorKind::NotFound {
                return Err(format!(
                    "Remove migrated managed-worktree registry {}: {error}",
                    legacy_path.display()
                ));
            }
        }
    }
    Ok(())
}

pub(super) async fn attached_worktree_paths(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = git_output(
        repo_root,
        vec![
            "worktree".into(),
            "list".into(),
            "--porcelain".into(),
            "-z".into(),
        ],
        "Inspect worktrees",
    )
    .await?;
    Ok(output
        .split('\0')
        .filter_map(|field| field.strip_prefix("worktree "))
        .filter_map(|path| PathBuf::from(path).canonicalize().ok())
        .collect())
}
