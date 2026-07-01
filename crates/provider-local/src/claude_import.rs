//! Migrate from Claude Code. Discover the MCP servers and skills a user already
//! configured under `.mcp.json` / `.claude/`, so bringing an existing setup into
//! Clark Code is one click (MCP) or automatic (skills).
//!
//! All reads go through the session's [`Executor`], so discovery works the same
//! for a **local** project (reads the local disk) and a **remote** one (reads
//! the remote host's `.claude` over the exec-server tunnel). `$HOME` is resolved
//! on whichever machine the executor targets.
//!
//! MCP: Clark's [`McpServerConfig`] is the same `{command, args, env}` shape as a
//! Claude `.mcp.json` entry, so imported servers run unchanged. Skills: Claude's
//! `SKILL.md` files are surfaced to the agent in the system prompt (name +
//! description), and the agent reads the full file on demand — the same
//! progressive-disclosure approach Claude Code and Codex use.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::exec::Executor;
use crate::mcp::McpServerConfig;

async fn read_text(exec: &dyn Executor, path: &Path) -> Option<String> {
    String::from_utf8(exec.read(path).await.ok()?).ok()
}

async fn read_json(exec: &dyn Executor, path: &Path) -> Option<Value> {
    serde_json::from_str(&read_text(exec, path).await?).ok()
}

/// `$HOME` on the executor's target machine (local or remote).
async fn resolve_home(exec: &dyn Executor, cwd: &Path) -> Option<PathBuf> {
    let out = exec
        .exec(
            "printf %s \"$HOME\"",
            cwd,
            Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .await
        .ok()?;
    let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!home.is_empty()).then(|| PathBuf::from(home))
}

// ---- MCP servers -----------------------------------------------------------

/// Parse one Claude `mcpServers` entry into Clark's config. stdio only — Clark
/// spawns local processes; remote `http`/`sse` servers are skipped.
fn parse_mcp_entry(name: &str, v: &Value) -> Option<McpServerConfig> {
    if let Some(t) = v.get("type").and_then(Value::as_str) {
        if !t.eq_ignore_ascii_case("stdio") {
            return None;
        }
    }
    let command = v
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if command.is_empty() {
        return None;
    }
    let args = v
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let env = v
        .get("env")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Some(McpServerConfig {
        name: name.to_string(),
        command: command.to_string(),
        args,
        env,
    })
}

/// Pull `obj.mcpServers` into `out`, first-name-wins (earlier sources override).
fn collect_mcp(obj: &Value, out: &mut Vec<McpServerConfig>, seen: &mut HashSet<String>) {
    let Some(map) = obj.get("mcpServers").and_then(Value::as_object) else {
        return;
    };
    for (name, v) in map {
        if seen.contains(name) {
            continue;
        }
        if let Some(cfg) = parse_mcp_entry(name, v) {
            seen.insert(name.clone());
            out.push(cfg);
        }
    }
}

/// Discover Claude Code MCP servers for `cwd`, most-specific first (project
/// `.mcp.json` and `.claude/settings*.json`, then the project-scoped and global
/// entries in `~/.claude.json`). A name is only taken from the first source that
/// defines it. Reads through `exec`, so it works local or remote.
pub async fn discover_mcp_servers(exec: &dyn Executor, cwd: &Path) -> Vec<McpServerConfig> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    if let Some(v) = read_json(exec, &cwd.join(".mcp.json")).await {
        collect_mcp(&v, &mut out, &mut seen);
    }
    for f in [".claude/settings.json", ".claude/settings.local.json"] {
        if let Some(v) = read_json(exec, &cwd.join(f)).await {
            collect_mcp(&v, &mut out, &mut seen);
        }
    }
    if let Some(home) = resolve_home(exec, cwd).await {
        if let Some(v) = read_json(exec, &home.join(".claude.json")).await {
            if let Some(proj) = v
                .get("projects")
                .and_then(|p| p.get(cwd.to_string_lossy().as_ref()))
            {
                collect_mcp(proj, &mut out, &mut seen);
            }
            collect_mcp(&v, &mut out, &mut seen);
        }
    }
    out
}

// ---- Skills ----------------------------------------------------------------

/// A Claude Code skill discovered from a `SKILL.md`.
#[derive(Clone, Debug, Serialize)]
pub struct ClaudeSkill {
    pub name: String,
    pub description: String,
    /// Absolute path to the `SKILL.md` (on the executor's machine).
    pub path: String,
    /// `project` or `personal`.
    pub scope: &'static str,
}

/// The YAML-ish frontmatter between the leading `---` fences.
fn frontmatter(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix("---")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

fn fm_field(fm: &str, key: &str) -> Option<String> {
    for line in fm.lines() {
        if let Some(v) = line.trim().strip_prefix(&format!("{key}:")) {
            let v = v.trim().trim_matches(['"', '\'']).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn parse_skill(text: &str, skill_md: &Path, scope: &'static str) -> Option<ClaudeSkill> {
    let fm = frontmatter(text).unwrap_or("");
    let name = fm_field(fm, "name").or_else(|| {
        skill_md
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
    })?;
    if name.is_empty() {
        return None;
    }
    Some(ClaudeSkill {
        name,
        description: fm_field(fm, "description").unwrap_or_default(),
        path: skill_md.to_string_lossy().to_string(),
        scope,
    })
}

async fn skills_in(
    exec: &dyn Executor,
    dir: &Path,
    scope: &'static str,
    out: &mut Vec<ClaudeSkill>,
) {
    let Ok(entries) = exec.read_dir(dir).await else {
        return;
    };
    for e in entries {
        if !e.is_dir {
            continue;
        }
        let skill_md = dir.join(&e.name).join("SKILL.md");
        if let Some(text) = read_text(exec, &skill_md).await {
            if let Some(s) = parse_skill(&text, &skill_md, scope) {
                out.push(s);
            }
        }
    }
}

/// Discover Claude Code skills: personal (`~/.claude/skills`) and project
/// (`<root>/.claude/skills`), via `exec`. Project skills win over personal by
/// name.
pub async fn discover_skills(exec: &dyn Executor, project_root: &Path) -> Vec<ClaudeSkill> {
    let mut all = Vec::new();
    skills_in(
        exec,
        &project_root.join(".claude/skills"),
        "project",
        &mut all,
    )
    .await;
    if let Some(home) = resolve_home(exec, project_root).await {
        skills_in(exec, &home.join(".claude/skills"), "personal", &mut all).await;
    }
    // De-dup by name; project (added first) wins.
    let mut seen = HashSet::new();
    all.retain(|s| seen.insert(s.name.clone()));
    all.sort_by_key(|s| s.name.to_lowercase());
    all
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// A compact `# Skills` block for the system prompt, or `None` if there are no
/// skills. Lists each skill's name + description and points the agent at the
/// full `SKILL.md`, which it reads with `read_file` when a task matches.
pub async fn skills_prompt_section(exec: &dyn Executor, project_root: &Path) -> Option<String> {
    let skills = discover_skills(exec, project_root).await;
    if skills.is_empty() {
        return None;
    }
    let mut s = String::from(
        "\n# Skills\n\
         Reusable skills from the user's Claude setup are available. When a task \
         matches one, read its `SKILL.md` with `read_file` and follow it.\n",
    );
    for sk in &skills {
        s.push_str(&format!(
            "- **{}** — {} (read `{}`)\n",
            sk.name,
            truncate(&sk.description, 200),
            sk.path
        ));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    #[test]
    fn parses_stdio_mcp_and_skips_remote() {
        let v: Value = serde_json::json!({
            "mcpServers": {
                "fs": { "command": "npx", "args": ["-y", "server-fs", "."], "env": { "K": "v" } },
                "web": { "type": "http", "url": "https://x" }
            }
        });
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        collect_mcp(&v, &mut out, &mut seen);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "fs");
        assert_eq!(out[0].args, vec!["-y", "server-fs", "."]);
        assert_eq!(out[0].env.get("K").map(String::as_str), Some("v"));
    }

    #[tokio::test]
    async fn discovers_project_mcp_and_skills_via_executor() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers":{"github":{"command":"npx","args":["-y","server-github"]}}}"#,
        )
        .unwrap();
        let skill_dir = root.join(".claude/skills/pdf-tools");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: pdf-tools\ndescription: Fill and read PDF forms.\n---\n\nBody.",
        )
        .unwrap();

        let exec = LocalExecutor;
        let mcp = discover_mcp_servers(&exec, root).await;
        assert!(mcp.iter().any(|m| m.name == "github" && m.command == "npx"));

        let skills = discover_skills(&exec, root).await;
        let pdf = skills
            .iter()
            .find(|s| s.name == "pdf-tools")
            .expect("skill");
        assert_eq!(pdf.description, "Fill and read PDF forms.");
        assert_eq!(pdf.scope, "project");

        let section = skills_prompt_section(&exec, root).await.unwrap();
        assert!(section.contains("pdf-tools"));
        assert!(section.contains("Fill and read PDF forms."));
    }
}
