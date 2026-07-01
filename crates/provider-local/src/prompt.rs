//! System-prompt assembly for the local coding agent.
//!
//! Kept stable across a session so a prompt-caching prefix holds. Volatile,
//! per-turn facts (changed files, new git state) belong in turn messages, not
//! here.

use std::path::Path;

use crate::sandbox::Sandbox;

/// Build the one system message for a session rooted at `sandbox`.
pub fn system_prompt(sandbox: &Sandbox, research_available: bool) -> String {
    let root = sandbox.root().display();
    let mut p = String::new();

    p.push_str(
        "You are a coding agent operating directly on the user's local machine and codebase. \
You write and modify real files and run real commands on their computer.\n\n",
    );

    p.push_str("# Behavior\n");
    p.push_str("- Be concise and direct. Prefer using tools over guessing or describing what you would do.\n");
    p.push_str("- Read a file before you edit it. Make minimal, targeted changes that match the surrounding code style.\n");
    p.push_str("- For `edit_file`, choose an `old_string` with enough surrounding context to match exactly once.\n");
    p.push_str("- Use `grep`/`glob`/`list_dir` to locate code instead of reading entire trees.\n");
    p.push_str(
        "- Don't add comments or documentation unless asked. Don't commit or push unless asked.\n",
    );
    p.push_str("- After making changes, verify them (build/tests) with `bash` when appropriate.\n");
    if research_available {
        p.push_str(
            "- For anything requiring up-to-date external information — latest API/library docs, \
web search, browsing, or broader multi-step research — call `clark_research`. It runs remotely \
in Clark's sandbox; never try to reach the network with `bash`.\n",
        );
    }
    p.push('\n');

    p.push_str("# Environment\n");
    p.push_str(&format!("- Project root: {root}\n"));
    p.push_str(&format!("- OS: {}\n", std::env::consts::OS));
    p.push_str("- All file paths you pass to tools are resolved relative to the project root and cannot escape it.\n");
    p.push_str("- The shell runs with the project root as its working directory.\n");

    p.push_str("\n# Project memory\n");
    p.push_str(&crate::memory::system_prompt_section(sandbox.root()));

    if let Some(ctx) = project_context(sandbox.root()) {
        p.push_str("\n# Project context\n");
        p.push_str(&ctx);
    }

    // Note: the `# Skills` section (from the user's Claude setup) is appended in
    // `new_session`, which has the session's `Executor` to read `.claude` — local
    // or remote — asynchronously.

    p
}

/// Concatenate short excerpts of well-known guidance files if present.
fn project_context(root: &Path) -> Option<String> {
    const FILES: &[&str] = &["AGENTS.md", "CLAUDE.md", "README.md"];
    const MAX_PER_FILE: usize = 4_000;
    let mut out = String::new();
    for name in FILES {
        let path = root.join(name);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let excerpt: String = text.chars().take(MAX_PER_FILE).collect();
            out.push_str(&format!("\n## {name}\n{excerpt}\n"));
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_root_and_research_note_when_available() {
        let dir = tempfile::tempdir().unwrap();
        let sb = Sandbox::new(dir.path()).unwrap();
        let p = system_prompt(&sb, true);
        assert!(p.contains("Project root:"));
        assert!(p.contains("clark_research"));
    }

    #[test]
    fn omits_research_note_when_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let sb = Sandbox::new(dir.path()).unwrap();
        let p = system_prompt(&sb, false);
        assert!(!p.contains("clark_research"));
    }

    #[test]
    fn pulls_in_agents_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Special rule: be terse.").unwrap();
        let sb = Sandbox::new(dir.path()).unwrap();
        let p = system_prompt(&sb, false);
        assert!(p.contains("Special rule: be terse."));
    }
}
