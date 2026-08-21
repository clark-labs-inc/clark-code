//! App-managed workspace for agent-authored documents.
//!
//! When the local agent produces a user-facing written deliverable (a report,
//! summary or design note…), it saves it as Markdown into a per-session folder under
//! `~/.agent/workspace/<session>/` rather than into the project — keeping the
//! repo clean and giving the UI a stable place to read the document from for its
//! inline viewer. The sandbox is extended (see [`crate::sandbox::Sandbox`]) to
//! permit writes here in addition to the project root.

use std::path::{Path, PathBuf};

/// `~/.agent/workspace` — the root of the app-managed document workspace.
pub const WORKSPACE_SUBDIR: &str = ".agent/workspace";
pub const QUICK_CHAT_MARKER: &str = ".agent-quick-chat";

/// The workspace root on this machine, or `None` if the home dir can't resolve.
pub fn workspace_root() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(WORKSPACE_SUBDIR))
}

/// The workspace directory for one session (`~/.agent/workspace/<session>`).
pub fn session_workspace(session_id: &str) -> Option<PathBuf> {
    if !is_safe_session_id(session_id) {
        return None;
    }
    workspace_root().map(|root| root.join(session_id))
}

/// Session workspace names are path components, not paths. This is also the
/// boundary that lets the host bind a public conversation id to a local
/// document workspace without allowing traversal through that binding.
pub fn is_safe_session_id(session_id: &str) -> bool {
    let path = Path::new(session_id);
    !session_id.is_empty()
        && !session_id.contains(['/', '\\'])
        && matches!(
            path.components().next(),
            Some(std::path::Component::Normal(_))
        )
        && path.components().count() == 1
}

/// Whether `path` is exactly one per-session directory directly beneath the
/// app-managed workspace root. Quick Chat uses this directory as both its
/// checkout and document root.
pub fn is_session_workspace(path: &Path) -> bool {
    let Some(root) = workspace_root() else {
        return false;
    };
    is_session_workspace_under(&root, path)
}

/// Mark a newly allocated directory as a repository-free Quick Chat checkout.
pub fn initialize_quick_chat_workspace(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::write(path.join(QUICK_CHAT_MARKER), b"agent-quick-chat-v1\n")
}

pub fn is_quick_chat_workspace(path: &Path) -> bool {
    path.is_dir() && path.join(QUICK_CHAT_MARKER).is_file()
}

fn is_session_workspace_under(root: &Path, path: &Path) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    path.parent() == Some(root.as_path())
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok())
}

/// True for a path that names a Markdown document.
pub fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md") | Some("markdown")
    )
}

/// The system-prompt section that tells the agent where to write documents. The
/// path is the absolute (canonical) workspace directory for this session.
pub fn prompt_section(docs_dir: &Path) -> String {
    format!(
        "\n# Documents\n\
         When you produce a substantial written deliverable for the user (a report, \
         summary, design doc, or notes), save it as a Markdown (`.md`) file in your \
         workspace directory:\n\n    {dir}\n\n\
         Write with an absolute path under that directory (e.g. `{dir}/report.md`). These \
         files render inline for the user as a document viewer, so prefer well-structured \
         Markdown with clear headings; put `---` on its own line between blank lines to \
         separate slides/sections (the user can page through them). For PDF, DOCX, and other \
         office deliverables, activate `document_convert` with `tool_search` and use the \
         bundled pure-Rust libreoffice-rs converter; do not improvise with `textutil`, \
         `soffice`, Pandoc, or Python document generators. Keep code and project \
         changes in the project itself — the workspace is only for user-facing documents.\n",
        dir = docs_dir.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_markdown_by_extension() {
        assert!(is_markdown(Path::new("/w/report.md")));
        assert!(is_markdown(Path::new("/w/NOTES.MARKDOWN")));
        assert!(!is_markdown(Path::new("/w/main.rs")));
        assert!(!is_markdown(Path::new("/w/readme")));
    }

    #[test]
    fn session_workspace_sits_under_the_root() {
        // Only assert when a home dir resolves (always true in CI/dev).
        if let (Some(root), Some(ws)) = (workspace_root(), session_workspace("sess-1")) {
            assert!(ws.starts_with(&root));
            assert!(ws.ends_with("sess-1"));
        }
    }

    #[test]
    fn session_workspace_rejects_path_traversal() {
        assert!(is_safe_session_id("conversation-1"));
        assert!(!is_safe_session_id("../outside"));
        assert!(!is_safe_session_id("nested/conversation-1"));
        assert!(!is_safe_session_id(""));
        assert!(session_workspace("../outside").is_none());
    }

    #[test]
    fn recognizes_only_uuid_named_direct_session_directories() {
        let root = tempfile::tempdir().unwrap();
        let id = "912a9700-7f5f-4f18-9785-b5d9315a41b4";
        let session = root.path().join(id);
        let nested = session.join("nested");
        let named = root.path().join("not-a-session");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir(&named).unwrap();

        assert!(is_session_workspace_under(root.path(), &session));
        assert!(!is_session_workspace_under(root.path(), &nested));
        assert!(!is_session_workspace_under(root.path(), &named));
    }

    #[test]
    fn marker_identifies_a_repository_free_workspace() {
        let root = tempfile::tempdir().unwrap();
        assert!(!is_quick_chat_workspace(root.path()));
        initialize_quick_chat_workspace(root.path()).unwrap();
        assert!(is_quick_chat_workspace(root.path()));
    }

    #[test]
    fn document_deliverables_use_the_bundled_rust_converter() {
        let prompt = prompt_section(Path::new("/workspace/session"));
        assert!(prompt.contains("`document_convert`"));
        assert!(prompt.contains("pure-Rust libreoffice-rs"));
        assert!(prompt.contains("do not improvise with `textutil`"));
    }
}
