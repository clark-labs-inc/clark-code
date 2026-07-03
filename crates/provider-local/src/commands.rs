//! Custom user-authored slash commands: `.claude/commands/*.md` (project) and
//! `~/.claude/commands/*.md` (personal) — the same convention Claude Code
//! itself uses, so an existing setup works with zero migration. Frontmatter
//! carries `description`; the body is inserted into the composer when the
//! command is picked (see the frontend `slashCommands.ts`/`Composer.tsx`).
//!
//! Commands are a frontend-only concern (composer autocomplete) — unlike
//! skills, which fold into the system prompt — so discovery is exposed via a
//! dedicated Tauri command (`list_commands`) rather than hooked into
//! `new_session`.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::exec::Executor;
use crate::markdown_frontmatter::{
    body_after_frontmatter, fm_field, frontmatter, read_text, resolve_home,
};

/// A user-authored slash command discovered from a markdown file.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CustomCommand {
    /// The `/name` the composer matches on — the file's stem (`test.md` ->
    /// `test`), or `name` from frontmatter if set.
    pub name: String,
    pub description: String,
    /// The markdown body (frontmatter stripped) inserted into the composer.
    pub body: String,
    /// `project` or `personal`.
    pub scope: &'static str,
}

fn parse_command(text: &str, stem: &str, scope: &'static str) -> Option<CustomCommand> {
    let fm = frontmatter(text).unwrap_or("");
    let name = fm_field(fm, "name").unwrap_or_else(|| stem.to_string());
    if name.is_empty() {
        return None;
    }
    let body = body_after_frontmatter(text);
    if body.is_empty() {
        return None;
    }
    Some(CustomCommand {
        name,
        description: fm_field(fm, "description").unwrap_or_default(),
        body: body.to_string(),
        scope,
    })
}

async fn commands_in(
    exec: &dyn Executor,
    dir: &Path,
    scope: &'static str,
    out: &mut Vec<CustomCommand>,
) {
    let Ok(entries) = exec.read_dir(dir).await else {
        return;
    };
    for e in entries {
        if e.is_dir || !e.name.ends_with(".md") {
            continue;
        }
        let path = dir.join(&e.name);
        let Some(text) = read_text(exec, &path).await else {
            continue;
        };
        let stem = e.name.trim_end_matches(".md");
        if let Some(cmd) = parse_command(&text, stem, scope) {
            out.push(cmd);
        }
    }
}

/// Discover custom commands: project (`<root>/.claude/commands`) and personal
/// (`~/.claude/commands`), via `exec` (works for remote/SSH projects too).
/// Project commands win over personal by name.
pub async fn discover_commands(exec: &dyn Executor, project_root: &Path) -> Vec<CustomCommand> {
    let mut all = Vec::new();
    commands_in(
        exec,
        &project_root.join(".claude/commands"),
        "project",
        &mut all,
    )
    .await;
    if let Some(home) = resolve_home(exec, project_root).await {
        commands_in(exec, &home.join(".claude/commands"), "personal", &mut all).await;
    }
    let mut seen = HashSet::new();
    all.retain(|c| seen.insert(c.name.clone()));
    all.sort_by_key(|c| c.name.to_lowercase());
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    #[test]
    fn parses_frontmatter_name_and_description() {
        let text = "---\nname: test\ndescription: Run the test suite.\n---\n\nRun the tests and summarize failures.";
        let cmd = parse_command(text, "unused-stem", "project").unwrap();
        assert_eq!(cmd.name, "test");
        assert_eq!(cmd.description, "Run the test suite.");
        assert_eq!(cmd.body, "Run the tests and summarize failures.");
    }

    #[test]
    fn falls_back_to_file_stem_when_no_frontmatter_name() {
        let cmd = parse_command("Just do the thing.", "do-thing", "project").unwrap();
        assert_eq!(cmd.name, "do-thing");
        assert_eq!(cmd.body, "Just do the thing.");
    }

    #[test]
    fn empty_body_is_rejected() {
        assert!(parse_command("---\nname: empty\n---\n\n", "empty", "project").is_none());
    }

    #[tokio::test]
    async fn discovers_project_commands_via_executor_and_dedupes_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cmd_dir = root.join(".claude/commands");
        std::fs::create_dir_all(&cmd_dir).unwrap();
        std::fs::write(
            cmd_dir.join("review.md"),
            "---\ndescription: Review the current diff.\n---\n\nReview the current diff for bugs.",
        )
        .unwrap();

        let exec = LocalExecutor;
        let cmds = discover_commands(&exec, root).await;
        let review = cmds
            .iter()
            .find(|c| c.name == "review")
            .expect("review command");
        assert_eq!(review.description, "Review the current diff.");
        assert_eq!(review.scope, "project");
    }

    #[tokio::test]
    async fn no_commands_dir_yields_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let cmds = discover_commands(&LocalExecutor, dir.path()).await;
        assert!(cmds.is_empty());
    }
}
