//! Project-root containment for local file tools.
//!
//! The model is *told* the project root in the system prompt, but enforcement
//! lives here in code, not trust: every path a tool touches is resolved against
//! the canonical root and rejected if it escapes (via `..` or a symlink). `bash`
//! is the deliberate hole in this fence — it runs with `cwd = root` but can do
//! anything, which is why it defaults to requiring confirmation.

use std::path::{Component, Path, PathBuf};

/// A canonicalized project root that file paths are resolved against.
#[derive(Clone, Debug)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// Canonicalize `root`. Fails if it doesn't exist / isn't a directory.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref();
        let canon = root
            .canonicalize()
            .map_err(|e| format!("project root {}: {e}", root.display()))?;
        if !canon.is_dir() {
            return Err(format!(
                "project root {} is not a directory",
                canon.display()
            ));
        }
        Ok(Self { root: canon })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a (possibly relative) path for **reading**. The target must exist
    /// and, after symlink resolution, lie within the root.
    pub fn resolve_existing(&self, path: &str) -> Result<PathBuf, String> {
        let joined = self.join(path);
        let canon = joined.canonicalize().map_err(|e| format!("{path}: {e}"))?;
        self.ensure_contained(&canon)?;
        Ok(canon)
    }

    /// Resolve a (possibly relative) path for **writing**. The file need not
    /// exist, but its nearest existing ancestor must resolve within the root, so
    /// a new file can't be planted outside via `..` or a symlinked parent.
    pub fn resolve_for_write(&self, path: &str) -> Result<PathBuf, String> {
        let joined = self.join(path);
        let normalized = lexically_normalize(&joined);
        // Walk up to the first existing ancestor and canonicalize it.
        let mut ancestor = normalized.as_path();
        loop {
            match ancestor.parent() {
                Some(parent) => {
                    if parent.exists() {
                        let canon_parent = parent
                            .canonicalize()
                            .map_err(|e| format!("{}: {e}", parent.display()))?;
                        self.ensure_contained(&canon_parent)?;
                        break;
                    }
                    ancestor = parent;
                }
                None => return Err(format!("{path}: no existing parent directory")),
            }
        }
        self.ensure_contained_lexical(&normalized)?;
        Ok(normalized)
    }

    fn join(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        }
    }

    fn ensure_contained(&self, canon: &Path) -> Result<(), String> {
        if canon.starts_with(&self.root) {
            Ok(())
        } else {
            Err(format!(
                "path escapes project root: {} is outside {}",
                canon.display(),
                self.root.display()
            ))
        }
    }

    fn ensure_contained_lexical(&self, normalized: &Path) -> Result<(), String> {
        if normalized.starts_with(&self.root) {
            Ok(())
        } else {
            Err(format!(
                "path escapes project root: {} is outside {}",
                normalized.display(),
                self.root.display()
            ))
        }
    }

    /// Render a path relative to the root for display, falling back to absolute.
    pub fn display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    }
}

/// Resolve `.`/`..` lexically without touching the filesystem (no symlink
/// resolution). Used to normalize a write target before the ancestor check.
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        dir
    }

    #[test]
    fn resolves_relative_existing_file() {
        let dir = temp_root();
        let sb = Sandbox::new(dir.path()).unwrap();
        let p = sb.resolve_existing("src/main.rs").unwrap();
        assert!(p.ends_with("src/main.rs"));
        assert_eq!(sb.display(&p), "src/main.rs");
    }

    #[test]
    fn rejects_parent_escape_on_read() {
        let dir = temp_root();
        let sb = Sandbox::new(dir.path()).unwrap();
        let err = sb.resolve_existing("../etc/passwd").unwrap_err();
        assert!(err.contains("escapes") || err.contains("No such") || err.contains("passwd"));
    }

    #[test]
    fn rejects_parent_escape_on_write() {
        let dir = temp_root();
        let sb = Sandbox::new(dir.path()).unwrap();
        let err = sb.resolve_for_write("../escape.txt").unwrap_err();
        assert!(err.contains("escapes"));
    }

    #[test]
    fn allows_new_file_in_existing_dir() {
        let dir = temp_root();
        let sb = Sandbox::new(dir.path()).unwrap();
        let p = sb.resolve_for_write("src/new_mod.rs").unwrap();
        assert!(p.ends_with("src/new_mod.rs"));
    }

    #[test]
    fn allows_new_file_in_new_nested_dir() {
        let dir = temp_root();
        let sb = Sandbox::new(dir.path()).unwrap();
        let p = sb.resolve_for_write("a/b/c.txt").unwrap();
        assert!(p.ends_with("a/b/c.txt"));
    }
}
