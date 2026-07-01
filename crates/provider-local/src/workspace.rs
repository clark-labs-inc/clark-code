//! App-managed workspace for agent-authored documents.
//!
//! When the local agent produces a user-facing written deliverable (a report,
//! summary, plan…), it saves it as Markdown into a per-session folder under
//! `~/.clark/workspace/<session>/` rather than into the project — keeping the
//! repo clean and giving the UI a stable place to read the document from for its
//! inline viewer. The sandbox is extended (see [`crate::sandbox::Sandbox`]) to
//! permit writes here in addition to the project root.

use std::path::{Path, PathBuf};

/// `~/.clark/workspace` — the root of the app-managed document workspace.
pub const WORKSPACE_SUBDIR: &str = ".clark/workspace";

/// The workspace root on this machine, or `None` if the home dir can't resolve.
pub fn workspace_root() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(WORKSPACE_SUBDIR))
}

/// The workspace directory for one session (`~/.clark/workspace/<session>`).
pub fn session_workspace(session_id: &str) -> Option<PathBuf> {
    workspace_root().map(|root| root.join(session_id))
}

/// True for a path that names a Markdown document.
pub fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("md") | Some("markdown")
    )
}

/// The system-prompt section that tells the agent where to write documents. The
/// path is the absolute (canonical) workspace directory for this session.
pub fn prompt_section(docs_dir: &Path) -> String {
    format!(
        "\n# Documents\n\
         When you produce a substantial written deliverable for the user (a report, \
         summary, plan, design doc, or notes), save it as a Markdown (`.md`) file in your \
         workspace directory:\n\n    {dir}\n\n\
         Write with an absolute path under that directory (e.g. `{dir}/report.md`). These \
         files render inline for the user as a document viewer, so prefer well-structured \
         Markdown with clear headings; put `---` on its own line between blank lines to \
         separate slides/sections (the user can page through them). Keep code and project \
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
}
