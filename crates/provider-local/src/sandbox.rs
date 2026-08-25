//! Project-root containment for local file tools.
//!
//! The model is *told* the project root in the system prompt, but enforcement
//! lives here in code, not trust: every path a tool touches is resolved against
//! the canonical root and rejected if it escapes (via `..` or a symlink). `bash`
//! is the deliberate hole in this fence — it runs with `cwd = root` but can do
//! anything, which is why it defaults to requiring confirmation.

use std::path::{Component, Path, PathBuf};

mod access;

/// Whether this (mode, preset) pair runs tools on the bare host — the same
/// condition under which the provider's executor skips the sandbox. Kept next
/// to `Sandbox` so the trust flag and its trigger stay in one place.
pub(super) fn preset_runs_on_bare_host(
    mode: crate::config::LocalSandboxMode,
    preset: exec_sandbox::SandboxPreset,
) -> bool {
    mode == crate::config::LocalSandboxMode::Disabled
        || preset == exec_sandbox::SandboxPreset::DangerFullAccess
}

/// A project root resolved on the same machine as this provider process. A
/// durable remote worker runs the provider there, so containment is always
/// canonical and symlink-aware instead of emulating another filesystem.
#[derive(Clone, Debug)]
pub struct Sandbox {
    root: PathBuf,
    /// An unambiguous user-named subdirectory for the current turn. This is a
    /// narrower project boundary, not another allowed root: project file and
    /// shell tools may operate only below it until the turn ends.
    task_scope: Option<PathBuf>,
    /// An additional allowed root (the app-managed document workspace, outside
    /// the project). Writes/reads are permitted here as well as under `root`.
    /// Canonical; `None` unless attached via [`Sandbox::with_docs`].
    docs: Option<PathBuf>,
    /// Read-only roots explicitly approved by the host. These extend reads
    /// without becoming writable roots.
    read_roots: Vec<PathBuf>,
    /// Set when this session's executor runs on the bare host (Full Access /
    /// disabled sandbox): process containment is already lifted, so file-tool
    /// path walls would be inconsistent friction, not security. Only the
    /// Clark-internal private-state guard still applies.
    pub(super) host_trusted: bool,
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
            task_scope: None,
            docs: None,
            read_roots: Vec::new(),
            host_trusted: false,
        })
    }

    /// Mark this sandbox as running beside an unsandboxed host executor (Full
    /// Access): file-tool containment stops enforcing project walls so the
    /// model-facing tools match what shell commands can already do. The
    /// Clark-internal private-state guard stays active in every mode.
    pub fn with_host_trust(mut self) -> Self {
        self.host_trusted = true;
        self
    }

    /// Whether containment is lifted because the session's executor is the
    /// bare host executor (Full Access / disabled sandbox).
    pub fn host_trusted(&self) -> bool {
        self.host_trusted
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn task_scope(&self) -> Option<&Path> {
        self.task_scope.as_deref()
    }

    /// Narrow this cloned sandbox to an existing project subdirectory. The
    /// session's canonical root and document workspace remain stable; only
    /// model-facing project access is narrowed for this turn.
    pub fn with_task_scope(mut self, path: &str) -> Result<Self, String> {
        let joined = self.join(path);
        let canonical = joined
            .canonicalize()
            .map_err(|error| format!("task scope {path}: {error}"))?;
        if !canonical.is_dir() {
            return Err(format!(
                "task scope {} is not a directory",
                canonical.display()
            ));
        }
        self.ensure_write_contained(&canonical)?;
        self.task_scope = Some(canonical);
        Ok(self)
    }

    /// Attach an additional writable/readable root — the app-managed document
    /// workspace, which lives outside the project. The directory must exist; if
    /// it can't be canonicalized the sandbox is returned unchanged. Only
    /// meaningful for local sandboxes (a remote executor can't reach a local
    /// path).
    pub fn with_docs(mut self, dir: PathBuf) -> Self {
        if let Ok(canon) = dir.canonicalize() {
            if canon.is_dir() {
                self.docs = Some(canon);
            }
        }
        self
    }

    /// The attached document-workspace root, if any (canonical).
    pub fn docs_root(&self) -> Option<&Path> {
        self.docs.as_deref()
    }

    /// Resolve a (possibly relative) path for **reading**. The target must exist
    /// and, after symlink resolution, lie within an approved root.
    pub fn resolve_existing(&self, path: &str) -> Result<PathBuf, String> {
        let joined = self.join(path);
        if is_host_private(&lexically_normalize(&joined)) {
            return Err(format!(
                "{path}: path is reserved for host-private Clark Code state"
            ));
        }
        let canon = joined.canonicalize().map_err(|e| format!("{path}: {e}"))?;
        self.ensure_read_contained(&canon)?;
        self.ensure_task_contained(&canon, path)?;
        Ok(canon)
    }

    /// Resolve a (possibly relative) path for **writing**. The file need not
    /// exist. Local: its nearest existing ancestor must resolve within the root,
    /// so a new file can't be planted outside via `..` or a symlinked parent.
    pub fn resolve_for_write(&self, path: &str) -> Result<PathBuf, String> {
        let joined = self.join(path);
        let normalized = lexically_normalize(&joined);
        if is_host_private(&normalized) {
            return Err(format!(
                "{path}: path is reserved for host-private Clark Code state"
            ));
        }
        // Walk up to the first existing ancestor and canonicalize it.
        let mut ancestor = normalized.as_path();
        loop {
            match ancestor.parent() {
                Some(parent) => {
                    if parent.exists() {
                        let canon_parent = parent
                            .canonicalize()
                            .map_err(|e| format!("{}: {e}", parent.display()))?;
                        self.ensure_write_contained(&canon_parent)?;
                        break;
                    }
                    ancestor = parent;
                }
                None => return Err(format!("{path}: no existing parent directory")),
            }
        }
        // The leaf itself must not be a symlink. The executor's write follows
        // symlinks, so writing through one — even a dangling link — could
        // escape containment that the ancestor check cannot see.
        if let Ok(meta) = std::fs::symlink_metadata(&normalized) {
            if meta.file_type().is_symlink() {
                return Err(format!("{path}: refusing to write through a symlink"));
            }
        }
        self.ensure_write_contained_lexical(&normalized)?;
        self.ensure_task_contained(&normalized, path)?;
        Ok(normalized)
    }

    /// Resolve an Clark Code-owned path for an internal host tool. Model-facing file
    /// tools must use `resolve_for_write`, which rejects this namespace.
    pub(crate) fn resolve_host_managed(&self, path: &str) -> Result<PathBuf, String> {
        let joined = self.join(path);
        let normalized = lexically_normalize(&joined);
        if !is_host_private(&normalized) {
            return Err(format!("{path}: path is not in host-managed desktop state"));
        }
        let mut ancestor = normalized.as_path();
        loop {
            match ancestor.parent() {
                Some(parent) if parent.exists() => {
                    let canonical = parent
                        .canonicalize()
                        .map_err(|error| format!("{}: {error}", parent.display()))?;
                    self.ensure_write_contained(&canonical)?;
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
        self.ensure_write_contained_lexical(&normalized)?;
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

    fn ensure_task_contained(&self, path: &Path, display: &str) -> Result<(), String> {
        let Some(scope) = &self.task_scope else {
            return Ok(());
        };
        // Product-managed documents are a separate explicit root and remain
        // available to document-first specialist flows. The narrower boundary
        // applies only to paths inside the selected project.
        if !path.starts_with(&self.root) || path.starts_with(scope) {
            return Ok(());
        }
        Err(format!(
            "{display}: outside the explicit task scope {}. Work inside that directory or ask the user before widening scope",
            self.display(scope),
        ))
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
        .any(|window| window == [".agent", "scout", "enterprises"])
        || components
            .windows(4)
            .any(|window| window == [".agent", "scout", "adapters", "private"])
        || components
            .windows(4)
            .any(|window| window == [".agent", "scout", "capsules", "private"])
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
        let private = dir.path().join(".agent/scout/enterprises/v3-test/private");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(private.join("signing-bootstrap"), b"secret").unwrap();
        let trust = dir.path().join(".agent/scout/enterprises/v3-test/trust");
        std::fs::create_dir_all(&trust).unwrap();
        std::fs::write(trust.join("chain.json"), b"{}").unwrap();
        let adapter_private = dir.path().join(".agent/scout/adapters/private");
        std::fs::create_dir_all(&adapter_private).unwrap();
        std::fs::write(adapter_private.join("vault.key"), b"secret").unwrap();
        let capsule_private = dir.path().join(".agent/scout/capsules/private");
        std::fs::create_dir_all(&capsule_private).unwrap();
        std::fs::write(capsule_private.join("registry-v1.json"), b"secret").unwrap();
        let sandbox = Sandbox::new(dir.path()).unwrap();

        assert!(sandbox
            .resolve_existing(".agent/scout/enterprises/v3-test/private/signing-bootstrap")
            .is_err());
        assert!(sandbox
            .resolve_for_write(".agent/scout/enterprises/v3-test/private/replacement")
            .is_err());
        assert!(sandbox
            .resolve_existing(".agent/scout/enterprises/v3-test/trust/chain.json")
            .is_err());
        assert!(sandbox
            .resolve_for_write(".agent/scout/enterprises/v3-test/batches/forged.json")
            .is_err());
        assert!(sandbox
            .resolve_existing(".agent/scout/adapters/private/vault.key")
            .is_err());
        assert!(sandbox
            .resolve_existing(".agent/scout/capsules/private/registry-v1.json")
            .is_err());
        assert!(sandbox
            .resolve_for_write(".agent/scout/capsules/private/replacement")
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

    #[test]
    fn task_scope_rejects_project_siblings_but_allows_scoped_reads_and_writes() {
        let dir = temp_root();
        std::fs::create_dir(dir.path().join("sibling")).unwrap();
        std::fs::write(dir.path().join("sibling/example.txt"), "outside").unwrap();
        let sandbox = Sandbox::new(dir.path())
            .unwrap()
            .with_task_scope("src")
            .unwrap();

        assert!(sandbox.resolve_existing("src/main.rs").is_ok());
        assert!(sandbox.resolve_for_write("src/new.rs").is_ok());
        assert!(sandbox
            .resolve_existing(".")
            .unwrap_err()
            .contains("explicit task scope"));
        assert!(sandbox
            .resolve_existing("sibling/example.txt")
            .unwrap_err()
            .contains("explicit task scope"));
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
        std::os::unix::fs::symlink("/tmp/agent-sandbox-escape-test", &link).unwrap();
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
    fn approved_read_root_never_becomes_a_write_root() {
        let project = temp_root();
        let extra = tempfile::tempdir().unwrap();
        let evidence = extra.path().join("evidence.txt");
        std::fs::write(&evidence, "read only").unwrap();
        let sandbox = Sandbox::new(project.path())
            .unwrap()
            .with_read_roots([extra.path().to_path_buf()]);

        assert_eq!(
            sandbox
                .resolve_existing(evidence.to_str().unwrap())
                .unwrap(),
            evidence.canonicalize().unwrap()
        );
        assert!(sandbox
            .resolve_for_write(extra.path().join("changed.txt").to_str().unwrap())
            .unwrap_err()
            .contains("writable roots"));
    }

    #[test]
    fn full_access_host_trust_lifts_containment_but_keeps_clark_private_guard() {
        let project = temp_root();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("notes.txt");
        std::fs::write(&outside_file, "host file").unwrap();
        let private_dir = project.path().join(".agent/scout/enterprises/e1/private");
        std::fs::create_dir_all(&private_dir).unwrap();

        let trusted = Sandbox::new(project.path()).unwrap().with_host_trust();
        assert!(trusted.host_trusted());

        // Reads and writes anywhere on the host now resolve — matching what an
        // unsandboxed shell executor can already do.
        assert!(trusted
            .resolve_existing(outside_file.to_str().unwrap())
            .is_ok());
        assert!(trusted
            .resolve_for_write(outside.path().join("new.txt").to_str().unwrap())
            .is_ok());
        assert!(trusted.resolve_for_write("src/new.rs").is_ok());

        // Clark's own signing/identity material stays refused even here.
        assert!(trusted
            .resolve_existing(private_dir.join("signing-bootstrap").to_str().unwrap())
            .is_err());

        // The default sandbox keeps every wall.
        let contained = Sandbox::new(project.path()).unwrap();
        assert!(!contained.host_trusted());
        assert!(contained
            .resolve_existing(outside_file.to_str().unwrap())
            .is_err());
        assert!(contained
            .resolve_for_write(outside.path().join("new.txt").to_str().unwrap())
            .is_err());
    }
}
