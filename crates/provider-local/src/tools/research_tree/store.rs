//! Local artifact storage. The journal is planning data, never authorization or
//! proof. Every load replays validated events; no serialized state is trusted.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use research_tree::{Event, Identity, Limits, ResearchTree, Status};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

const MAX_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub version: u32,
    pub tree_id: String,
    pub workspace: String,
    pub objective: String,
    pub scope: String,
    pub sources: Vec<String>,
    pub identity: Identity,
    pub limits: Limits,
    pub events: Vec<Event>,
}

impl Journal {
    pub fn revision(&self) -> usize {
        self.events.len() + 1
    }

    pub fn replay(&self) -> Result<ResearchTree, String> {
        ResearchTree::replay(self.identity.clone(), self.limits, self.events.clone())
            .map_err(|error| error.to_string())
    }
}

pub(super) struct Store {
    root: PathBuf,
    directory: PathBuf,
    tree_id: String,
}

impl Store {
    pub fn new(root: &Path, tree_id: &str) -> Result<Self, String> {
        if tree_id.is_empty()
            || tree_id.len() > 80
            || !tree_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(
                "tree_id must be 1–80 ASCII letters, digits, hyphens or underscores".into(),
            );
        }
        let root = root.canonicalize().map_err(err)?;
        let directory = root.join(".agent/research-trees");
        let store = Self {
            root,
            directory,
            tree_id: tree_id.into(),
        };
        store.check_paths()?;
        Ok(store)
    }

    fn check_paths(&self) -> Result<(), String> {
        for path in [
            self.root.join(".agent"),
            self.directory.clone(),
            self.path(),
            self.directory.join(format!("{}.lock", self.tree_id)),
        ] {
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err("research tree storage must not contain symlinks".into());
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(err(error))
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn path(&self) -> PathBuf {
        self.directory.join(format!("{}.json", self.tree_id))
    }

    pub fn lock(&self) -> Result<File, String> {
        self.check_paths()?;
        fs::create_dir_all(&self.directory).map_err(err)?;
        self.check_paths()?;
        let lock_path = self.directory.join(format!("{}.lock", self.tree_id));
        if let Ok(metadata) = fs::symlink_metadata(&lock_path) {
            if !metadata.is_file() {
                return Err("research lock must be a regular file".into());
            }
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        harden_open(&mut options);
        let file = options.open(lock_path).map_err(err)?;
        if !file.metadata().map_err(err)?.is_file() {
            return Err("research lock must be a regular file".into());
        }
        file.try_lock()
            .map_err(|_| "research tree is busy; retry after its current update".to_string())?;
        Ok(file)
    }

    pub fn load(&self) -> Result<Journal, String> {
        self.check_paths()?;
        let bytes = bounded_read(&self.path(), MAX_JOURNAL_BYTES)?;
        let journal: Journal = serde_json::from_slice(&bytes).map_err(err)?;
        if journal.version != 1
            || journal.tree_id != self.tree_id
            || journal.workspace != self.root.to_string_lossy()
            || journal.identity.scope_id != scope_id(&self.root, &journal.scope)
        {
            return Err("research journal identity does not match this workspace/scope".into());
        }
        bounded_text(&journal.objective, 2048)?;
        bounded_text(&journal.scope, 2048)?;
        journal.replay()?;
        Ok(journal)
    }

    pub fn create(
        &self,
        objective: String,
        scope: String,
        sources: Vec<String>,
        limits: Limits,
    ) -> Result<Journal, String> {
        if self.path().exists() {
            return Err(
                "tree_id already exists; inspect it with research_tree_status or choose a new id"
                    .into(),
            );
        }
        bounded_text(&objective, 2048)?;
        bounded_text(&scope, 2048)?;
        let identity = Identity {
            scope_id: scope_id(&self.root, &scope),
            source_id: source_id(&self.root, &sources)?,
        };
        ResearchTree::new(identity.clone(), limits).map_err(|e| e.to_string())?;
        Ok(Journal {
            version: 1,
            tree_id: self.tree_id.clone(),
            workspace: self.root.to_string_lossy().into_owned(),
            objective,
            scope,
            sources,
            identity,
            limits,
            events: Vec::new(),
        })
    }

    pub fn snapshot_current(&self, journal: &Journal) -> Result<bool, String> {
        Ok(source_id(&self.root, &journal.sources)? == journal.identity.source_id)
    }

    pub fn save(&self, journal: &Journal, cancel: &CancellationToken) -> Result<(), String> {
        let bytes = serde_json::to_vec(journal).map_err(err)?;
        let active = journal
            .replay()?
            .nodes()
            .values()
            .filter(|node| node.status == Status::Active)
            .count();
        check_byte_budget(bytes.len() as u64, active)?;
        self.check_paths()?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory).map_err(err)?;
        temporary.write_all(&bytes).map_err(err)?;
        temporary.as_file().sync_all().map_err(err)?;
        if cancel.is_cancelled() {
            return Err("research tree update canceled before commit".into());
        }
        temporary.persist(self.path()).map_err(err)?;
        // Flush the directory entry where supported (Windows cannot open directories).
        #[cfg(unix)]
        File::open(&self.directory)
            .and_then(|f| f.sync_all())
            .map_err(err)?;
        Ok(())
    }
}

fn bounded_text(value: &str, limit: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > limit {
        Err(format!(
            "objective/scope must be nonempty and at most {limit} bytes"
        ))
    } else {
        Ok(())
    }
}

fn scope_id(root: &Path, scope: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(root.to_string_lossy().as_bytes());
    hash.update([0]);
    hash.update(scope.as_bytes());
    format!("{:x}", hash.finalize())
}

fn source_id(root: &Path, sources: &[String]) -> Result<String, String> {
    if sources.len() > 32 {
        return Err("at most 32 source files may be snapshotted".into());
    }
    let mut sources = sources.to_vec();
    sources.sort();
    if sources.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("source paths must be unique".into());
    }
    let mut hash = Sha256::new();
    let mut remaining = MAX_SOURCE_BYTES;
    for source in sources {
        if source.is_empty()
            || source.len() > 1024
            || Path::new(&source)
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || Path::new(&source)
                .components()
                .any(|part| part.as_os_str() == ".agent")
        {
            return Err(
                "sources must be relative file paths without traversal or .agent state".into(),
            );
        }
        let path = root.join(&source).canonicalize().map_err(err)?;
        if !path.starts_with(root) {
            return Err("source file escapes the investigation workspace".into());
        }
        if path
            .strip_prefix(root)
            .map_err(err)?
            .components()
            .any(|part| part.as_os_str() == ".agent")
        {
            return Err("source file resolves into generated agent state".into());
        }
        let bytes = bounded_read(&path, remaining)?;
        remaining -= bytes.len() as u64;
        hash.update((source.len() as u64).to_le_bytes());
        hash.update(source.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path).map_err(err)?.is_file() {
        return Err("research state and sources must be regular files".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    harden_open(&mut options);
    let file = options.open(path).map_err(err)?;
    if !file.metadata().map_err(err)?.is_file() {
        return Err("research state and sources must be regular files".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(err)?;
    if bytes.len() as u64 > limit {
        return Err("research state/source byte limit exceeded".into());
    }
    Ok(bytes)
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn harden_open(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Never hang on a substituted FIFO or follow a replaced leaf symlink.
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    let _ = options;
}

// Each tool input is at most 64 KiB. Reserve one full terminal event per active
// attempt so a large journal cannot strand already-selected work.
fn check_byte_budget(bytes: u64, active: usize) -> Result<(), String> {
    if bytes + active as u64 * 64 * 1024 > MAX_JOURNAL_BYTES {
        Err("research journal exceeds its 4 MiB budget including reserved active outcomes; start a separate bounded investigation".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_outcomes_keep_serialized_byte_headroom() {
        assert!(check_byte_budget(MAX_JOURNAL_BYTES - 65536, 1).is_ok());
        assert!(check_byte_budget(MAX_JOURNAL_BYTES - 65535, 1).is_err());
        assert!(check_byte_budget(MAX_JOURNAL_BYTES, 0).is_ok());
    }
}
