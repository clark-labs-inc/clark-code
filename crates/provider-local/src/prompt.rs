//! System-prompt assembly for the local coding agent.
//!
//! Kept stable across a session so a prompt-caching prefix holds. Volatile,
//! per-turn facts (changed files, new git state) belong in turn messages, not
//! here.

use std::path::Path;

use crate::sandbox::Sandbox;

/// One selectable output style/persona.
pub struct OutputStyle {
    pub id: &'static str,
    // `label`/`description` mirror the frontend's own style list for parity
    // and documentation; only `id` and `instructions` are read Rust-side.
    #[allow(dead_code)]
    pub label: &'static str,
    #[allow(dead_code)]
    pub description: &'static str,
    /// Per-turn instruction block; empty for `default` (no change from the
    /// base system prompt's own voice).
    pub instructions: &'static str,
}

/// Fixed set of built-in output styles (mirrors `REASONING_EFFORTS`'s shape
/// on the frontend — a small fixed enum, not a markdown-file convention, for
/// this first version). Selected via `Provider::set_output_style`, applied
/// per-turn in `LocalAgentProvider::prompt()` — never baked into the cached
/// system-prompt prefix.
pub const OUTPUT_STYLES: &[OutputStyle] = &[
    OutputStyle {
        id: "default",
        label: "Default",
        description: "Clark's normal voice.",
        instructions: "",
    },
    OutputStyle {
        id: "terse",
        label: "Terse",
        description: "Minimal narration — just the work and the result.",
        instructions: "Output style: Terse. Skip preamble and restating what you're about to do. \
No summaries unless asked. One-line status updates at most.",
    },
    OutputStyle {
        id: "teaching",
        label: "Teaching",
        description: "Explains reasoning and trade-offs as it works.",
        instructions:
            "Output style: Teaching. Briefly explain *why* behind non-obvious choices as \
you make them — the trade-off you weighed, not just what you did. Keep it to a sentence or two per \
choice, woven into the normal flow, not a lecture.",
    },
];

/// The instruction block for `style_id`, or empty for `default`/unknown ids.
pub fn output_style_instructions(style_id: &str) -> &'static str {
    OUTPUT_STYLES
        .iter()
        .find(|s| s.id == style_id)
        .map(|s| s.instructions)
        .unwrap_or("")
}

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
    p.push_str(
        "- Never fetch URLs with `bash` (`curl`/`wget`). For a single page/doc lookup, use \
`web_fetch` — it's local, fast, and returns markdown.",
    );
    if research_available {
        p.push_str(
            " For anything needing search, JS-rendered pages, or broader multi-step research, \
call `clark_research` instead — it runs remotely in Clark's sandbox.",
        );
    }
    p.push('\n');
    p.push('\n');

    p.push_str("# Planning\n");
    p.push_str("- You have an `update_plan` tool that shows the user a live checklist of steps. Use it for non-trivial multi-step work — not for simple or single-step requests you can just do.\n");
    p.push_str("- Keep at most one step `in_progress` at a time; move a step to `in_progress` before marking it `completed` (don't jump straight to completed).\n");
    p.push_str("- Don't restate the plan in your reply after calling `update_plan` — the checklist is already shown to the user; just summarize what changed.\n");
    p.push_str("- If the project has a check_command configured (.clark/settings.json), call `check_diagnostics` after non-trivial changes — it reports only new problems since your last call.\n");
    p.push('\n');

    p.push_str("# Environment\n");
    p.push_str(&format!("- Project root: {root}\n"));
    p.push_str(&format!("- OS: {}\n", std::env::consts::OS));
    p.push_str("- All file paths you pass to tools are resolved relative to the project root and cannot escape it.\n");
    p.push_str("- The shell runs with the project root as its working directory.\n");

    // Note: durable memory (project + global) is injected in `new_session`,
    // gated by the memories setting and read through the session executor.

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
    fn includes_planning_guidance() {
        let dir = tempfile::tempdir().unwrap();
        let sb = Sandbox::new(dir.path()).unwrap();
        let p = system_prompt(&sb, false);
        assert!(p.contains("update_plan"));
    }

    #[test]
    fn output_style_instructions_are_empty_for_default_and_unknown() {
        assert_eq!(output_style_instructions("default"), "");
        assert_eq!(output_style_instructions("nonexistent"), "");
        assert!(output_style_instructions("terse").contains("Terse"));
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
