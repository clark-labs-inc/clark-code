//! Project-root containment for local file tools.
//!
//! The model is *told* the project root in the system prompt, but enforcement
//! lives here in code, not trust: every path a tool touches is resolved against
//! the canonical root and rejected if it escapes (via `..` or a symlink). `bash`
//! is the deliberate hole in this fence — it runs with `cwd = root` but can do
//! anything, which is why it defaults to requiring confirmation.

use std::path::{Component, Path, PathBuf};

/// Whether the root lives on this machine or on a remote host (reached through
/// the exec-server). Remote roots can't be canonicalized — the filesystem isn't
/// here — so containment is purely lexical for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Local,
    Remote,
}

/// A project root that file paths are resolved against. Local roots are
/// canonicalized (symlink-aware); remote roots are normalized lexically.
#[derive(Clone, Debug)]
pub struct Sandbox {
    root: PathBuf,
    mode: Mode,
    /// An additional allowed root (the app-managed document workspace, outside
    /// the project). Writes/reads are permitted here as well as under `root`.
    /// Canonical; `None` unless attached via [`Sandbox::with_docs`].
    docs: Option<PathBuf>,
}

impl Sandbox {
    /// Canonicalize a **local** `root`. Fails if it doesn't exist / isn't a dir.
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
        Ok(Self {
            root: canon,
            mode: Mode::Local,
            docs: None,
        })
    }

    /// A **remote** project root: an absolute path on the remote host. The local
    /// filesystem can't be consulted, so we only normalize lexically and enforce
    /// containment lexically. The real enforcement is layered: the exec-server's
    /// own `--root` confinement and the local safety gate both still apply.
    pub fn new_remote(root: &str) -> Result<Self, String> {
        let p = PathBuf::from(root);
        if !remote_path_is_absolute(root) {
            return Err(format!(
                "remote project root must be an absolute path: {root}"
            ));
        }
        Ok(Self {
            root: lexically_normalize(&p),
            mode: Mode::Remote,
            docs: None,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Attach an additional writable/readable root — the app-managed document
    /// workspace, which lives outside the project. The directory must exist; if
    /// it can't be canonicalized the sandbox is returned unchanged. Only
    /// meaningful for local sandboxes (a remote executor can't reach a local
    /// path).
    pub fn with_docs(mut self, dir: PathBuf) -> Self {
        if self.mode == Mode::Local {
            if let Ok(canon) = dir.canonicalize() {
                if canon.is_dir() {
                    self.docs = Some(canon);
                }
            }
        }
        self
    }

    /// The attached document-workspace root, if any (canonical).
    pub fn docs_root(&self) -> Option<&Path> {
        self.docs.as_deref()
    }

    /// Resolve a (possibly relative) path for **reading**. Local: the target must
    /// exist and, after symlink resolution, lie within the root. Remote: the
    /// lexically-normalized path must lie within the root.
    pub fn resolve_existing(&self, path: &str) -> Result<PathBuf, String> {
        let joined = self.join(path);
        if is_host_private(&lexically_normalize(&joined)) {
            return Err(format!(
                "{path}: path is reserved for host-private Clark state"
            ));
        }
        match self.mode {
            Mode::Local => {
                let canon = joined.canonicalize().map_err(|e| format!("{path}: {e}"))?;
                self.ensure_contained(&canon)?;
                Ok(canon)
            }
            Mode::Remote => {
                let normalized = lexically_normalize(&joined);
                self.ensure_contained_lexical(&normalized)?;
                Ok(normalized)
            }
        }
    }

    /// Resolve a (possibly relative) path for **writing**. The file need not
    /// exist. Local: its nearest existing ancestor must resolve within the root,
    /// so a new file can't be planted outside via `..` or a symlinked parent.
    /// Remote: lexical containment only (no filesystem to stat).
    pub fn resolve_for_write(&self, path: &str) -> Result<PathBuf, String> {
        let joined = self.join(path);
        let normalized = lexically_normalize(&joined);
        if is_host_private(&normalized) {
            return Err(format!(
                "{path}: path is reserved for host-private Clark state"
            ));
        }
        if self.mode == Mode::Local {
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
            // The leaf itself must not be a symlink. The executor's write follows
            // symlinks, so writing through one — even a *dangling* link to an
            // absolute path outside the root — would escape containment that the
            // ancestor check (which only stats the parent) can't catch.
            if let Ok(meta) = std::fs::symlink_metadata(&normalized) {
                if meta.file_type().is_symlink() {
                    return Err(format!("{path}: refusing to write through a symlink"));
                }
            }
        }
        self.ensure_contained_lexical(&normalized)?;
        Ok(normalized)
    }

    /// Resolve a Clark-owned path for an internal host tool. Model-facing file
    /// tools must use `resolve_for_write`, which rejects this namespace.
    pub(crate) fn resolve_host_managed(&self, path: &str) -> Result<PathBuf, String> {
        let joined = self.join(path);
        let normalized = lexically_normalize(&joined);
        if !is_host_private(&normalized) {
            return Err(format!("{path}: path is not in host-managed Clark state"));
        }
        if self.mode == Mode::Local {
            let mut ancestor = normalized.as_path();
            loop {
                match ancestor.parent() {
                    Some(parent) if parent.exists() => {
                        let canonical = parent
                            .canonicalize()
                            .map_err(|error| format!("{}: {error}", parent.display()))?;
                        self.ensure_contained(&canonical)?;
                        break;
                    }
                    Some(parent) => ancestor = parent,
                    None => return Err(format!("{path}: no existing parent directory")),
                }
            }
            if std::fs::symlink_metadata(&normalized)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(format!("{path}: refusing a host-managed symlink"));
            }
        }
        self.ensure_contained_lexical(&normalized)?;
        Ok(normalized)
    }

    fn join(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() || (self.mode == Mode::Remote && remote_path_is_absolute(path)) {
            p.to_path_buf()
        } else {
            self.root.join(p)
        }
    }

    /// Whether `p` lies within the project root or the attached docs workspace.
    fn allowed(&self, p: &Path) -> bool {
        p.starts_with(&self.root) || self.docs.as_deref().is_some_and(|d| p.starts_with(d))
    }

    fn ensure_contained(&self, canon: &Path) -> Result<(), String> {
        if self.allowed(canon) {
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
        if self.allowed(normalized) {
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
        model_path(
            path.strip_prefix(&self.root)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string()),
        )
    }
}

/// Remote execution currently targets POSIX SSH hosts. A leading slash must
/// therefore remain absolute even when Clark Desktop itself runs on Windows,
/// where `Path::is_absolute("/home/...")` is false without a drive prefix.
fn remote_path_is_absolute(path: &str) -> bool {
    path.starts_with('/') || Path::new(path).is_absolute()
}

/// Paths shown to the model and sent back through tool arguments use `/`
/// consistently. Windows filesystem APIs still receive native `PathBuf`s.
pub(crate) fn model_path(path: String) -> String {
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path
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

fn is_host_private(path: &Path) -> bool {
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    components
        .windows(3)
        .any(|window| window == [".clark", "scout", "enterprises"])
        || components
            .windows(4)
            .any(|window| window == [".clark", "scout", "adapters", "private"])
        || components
            .windows(4)
            .any(|window| window == [".clark", "scout", "capsules", "private"])
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
    fn host_private_scout_state_is_not_model_readable_or_writable() {
        let dir = temp_root();
        let private = dir.path().join(".clark/scout/enterprises/v3-test/private");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(private.join("signing-bootstrap"), b"secret").unwrap();
        let trust = dir.path().join(".clark/scout/enterprises/v3-test/trust");
        std::fs::create_dir_all(&trust).unwrap();
        std::fs::write(trust.join("chain.json"), b"{}").unwrap();
        let adapter_private = dir.path().join(".clark/scout/adapters/private");
        std::fs::create_dir_all(&adapter_private).unwrap();
        std::fs::write(adapter_private.join("vault.key"), b"secret").unwrap();
        let capsule_private = dir.path().join(".clark/scout/capsules/private");
        std::fs::create_dir_all(&capsule_private).unwrap();
        std::fs::write(capsule_private.join("registry-v1.json"), b"secret").unwrap();
        let sandbox = Sandbox::new(dir.path()).unwrap();

        assert!(sandbox
            .resolve_existing(".clark/scout/enterprises/v3-test/private/signing-bootstrap")
            .is_err());
        assert!(sandbox
            .resolve_for_write(".clark/scout/enterprises/v3-test/private/replacement")
            .is_err());
        assert!(sandbox
            .resolve_existing(".clark/scout/enterprises/v3-test/trust/chain.json")
            .is_err());
        assert!(sandbox
            .resolve_for_write(".clark/scout/enterprises/v3-test/batches/forged.json")
            .is_err());
        assert!(sandbox
            .resolve_existing(".clark/scout/adapters/private/vault.key")
            .is_err());
        assert!(sandbox
            .resolve_existing(".clark/scout/capsules/private/registry-v1.json")
            .is_err());
        assert!(sandbox
            .resolve_for_write(".clark/scout/capsules/private/replacement")
            .is_err());
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

    #[cfg(unix)]
    #[test]
    fn rejects_write_through_a_dangling_symlink_leaf() {
        let dir = temp_root();
        let sb = Sandbox::new(dir.path()).unwrap();
        // A symlink inside the root pointing at an absolute path outside it, with
        // a non-existent target — the executor's write would follow it and plant
        // a file at /tmp/…, escaping the root. resolve_for_write must refuse.
        let link = dir.path().join("evil");
        std::os::unix::fs::symlink("/tmp/clark-sandbox-escape-test", &link).unwrap();
        let err = sb.resolve_for_write("evil").unwrap_err();
        assert!(err.contains("symlink"), "{err}");
    }

    #[test]
    fn allows_new_file_in_new_nested_dir() {
        let dir = temp_root();
        let sb = Sandbox::new(dir.path()).unwrap();
        let p = sb.resolve_for_write("a/b/c.txt").unwrap();
        assert!(p.ends_with("a/b/c.txt"));
    }

    #[test]
    fn remote_resolves_lexically_without_touching_disk() {
        // The remote root need not exist locally.
        let sb = Sandbox::new_remote("/home/me/project").unwrap();
        assert_eq!(sb.root(), Path::new("/home/me/project"));

        // Relative + absolute paths inside the root resolve.
        let r = sb.resolve_existing("src/main.rs").unwrap();
        assert_eq!(r, Path::new("/home/me/project/src/main.rs"));
        let w = sb.resolve_for_write("a/b/c.txt").unwrap();
        assert_eq!(w, Path::new("/home/me/project/a/b/c.txt"));

        // `..` escapes are refused lexically (read and write).
        assert!(sb
            .resolve_existing("../secret")
            .unwrap_err()
            .contains("escapes"));
        assert!(sb
            .resolve_for_write("../../etc/x")
            .unwrap_err()
            .contains("escapes"));
    }

    #[test]
    fn remote_requires_absolute_root() {
        assert!(Sandbox::new_remote("relative/path").is_err());
    }

    #[test]
    fn docs_root_permits_reads_and_writes_outside_project() {
        let proj = temp_root();
        let docs = tempfile::tempdir().unwrap();
        let sb = Sandbox::new(proj.path())
            .unwrap()
            .with_docs(docs.path().to_path_buf());
        let docs_canon = sb.docs_root().expect("docs attached").to_path_buf();

        // A new doc under the workspace resolves for writing.
        let target = docs_canon.join("report.md");
        let w = sb.resolve_for_write(target.to_str().unwrap()).unwrap();
        assert!(w.ends_with("report.md"));

        // An existing doc under the workspace resolves for reading.
        std::fs::write(&target, "# Hi").unwrap();
        let r = sb.resolve_existing(target.to_str().unwrap()).unwrap();
        assert!(r.ends_with("report.md"));

        // The project root still works, and escapes are still refused.
        assert!(sb.resolve_for_write("src/new.rs").is_ok());
        assert!(sb.resolve_for_write("/etc/evil.md").is_err());
    }

    #[test]
    fn remote_ignores_docs_attachment() {
        // A local docs path is meaningless for a remote executor.
        let docs = tempfile::tempdir().unwrap();
        let sb = Sandbox::new_remote("/home/me/project")
            .unwrap()
            .with_docs(docs.path().to_path_buf());
        assert!(sb.docs_root().is_none());
    }
}
