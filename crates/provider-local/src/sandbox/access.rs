use super::Sandbox;
use std::path::{Path, PathBuf};

impl Sandbox {
    /// Add existing read-only roots. Invalid or non-directory paths are not
    /// admitted; the host validates them before displaying a durable receipt.
    pub fn with_read_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        self.read_roots.extend(
            roots
                .into_iter()
                .filter_map(|root| root.canonicalize().ok().filter(|path| path.is_dir())),
        );
        self.read_roots.sort();
        self.read_roots.dedup();
        self
    }

    /// Replace the complete host-approved read-only set while preserving the
    /// writable checkout, task scope, and document workspace.
    pub fn replacing_read_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        self.read_roots.clear();
        self.with_read_roots(roots)
    }

    pub fn read_roots(&self) -> &[PathBuf] {
        &self.read_roots
    }

    fn writable(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
            || self
                .docs
                .as_deref()
                .is_some_and(|root| path.starts_with(root))
    }

    fn readable(&self, path: &Path) -> bool {
        self.writable(path) || self.read_roots.iter().any(|root| path.starts_with(root))
    }

    pub(super) fn ensure_read_contained(&self, path: &Path) -> Result<(), String> {
        if self.host_trusted {
            return Ok(());
        }
        ensure(self.readable(path), "readable", path, &self.root)
    }

    pub(super) fn ensure_write_contained(&self, path: &Path) -> Result<(), String> {
        if self.host_trusted {
            return Ok(());
        }
        ensure(self.writable(path), "writable", path, &self.root)
    }

    pub(super) fn ensure_write_contained_lexical(&self, path: &Path) -> Result<(), String> {
        self.ensure_write_contained(path)
    }
}

fn ensure(allowed: bool, kind: &str, path: &Path, project: &Path) -> Result<(), String> {
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "path escapes {kind} roots: {} is outside {}",
            path.display(),
            project.display()
        ))
    }
}
