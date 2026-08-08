use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;

use super::{MigratedInstruction, MigratedSkill, MigrationSource};
use crate::exec::Executor;
use crate::markdown_frontmatter::{fm_field, frontmatter, read_json, read_text};
use crate::mcp::McpServerConfig;

fn parse_mcp_entry(name: &str, value: &Value) -> Option<McpServerConfig> {
    if let Some(transport) = value.get("type").and_then(Value::as_str) {
        if !transport.eq_ignore_ascii_case("stdio") {
            return None;
        }
    }
    let command = value.get("command")?.as_str()?.trim();
    if command.is_empty() {
        return None;
    }
    let args = value
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(|arg| arg.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let env = value
        .get("env")
        .and_then(Value::as_object)
        .map(|env| {
            env.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(McpServerConfig {
        credential_ref: None,
        name: name.to_string(),
        command: command.to_string(),
        args,
        env,
    })
}

fn collect_mcp(value: &Value, out: &mut Vec<McpServerConfig>, seen: &mut HashSet<String>) {
    let Some(servers) = value.get("mcpServers").and_then(Value::as_object) else {
        return;
    };
    for (name, value) in servers {
        if seen.contains(name) {
            continue;
        }
        if let Some(config) = parse_mcp_entry(name, value) {
            seen.insert(name.clone());
            out.push(config);
        }
    }
}

pub(super) async fn discover_mcp_servers(
    exec: &dyn Executor,
    cwd: &Path,
    home: Option<&Path>,
) -> Vec<McpServerConfig> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Some(value) = read_json(exec, &cwd.join(".mcp.json")).await {
        collect_mcp(&value, &mut out, &mut seen);
    }
    for path in [".claude/settings.json", ".claude/settings.local.json"] {
        if let Some(value) = read_json(exec, &cwd.join(path)).await {
            collect_mcp(&value, &mut out, &mut seen);
        }
    }
    if let Some(home) = home {
        if let Some(value) = read_json(exec, &home.join(".claude.json")).await {
            if let Some(project) = value
                .get("projects")
                .and_then(|projects| projects.get(cwd.to_string_lossy().as_ref()))
            {
                collect_mcp(project, &mut out, &mut seen);
            }
            collect_mcp(&value, &mut out, &mut seen);
        }
    }
    out.sort_by_key(|server| server.name.to_lowercase());
    out
}

fn parse_skill(text: &str, path: &Path, scope: &'static str) -> Option<MigratedSkill> {
    let frontmatter = frontmatter(text).unwrap_or("");
    let name = fm_field(frontmatter, "name").or_else(|| {
        path.parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().to_string())
    })?;
    (!name.is_empty()).then(|| MigratedSkill {
        name,
        description: fm_field(frontmatter, "description").unwrap_or_default(),
        path: path.to_string_lossy().to_string(),
        scope,
        source: MigrationSource::Claude,
    })
}

async fn skills_in(
    exec: &dyn Executor,
    directory: &Path,
    scope: &'static str,
    out: &mut Vec<MigratedSkill>,
) {
    let Ok(entries) = exec.read_dir(directory).await else {
        return;
    };
    for entry in entries.into_iter().filter(|entry| entry.is_dir) {
        let path = directory.join(entry.name).join("SKILL.md");
        if let Some(text) = read_text(exec, &path).await {
            if let Some(skill) = parse_skill(&text, &path, scope) {
                out.push(skill);
            }
        }
    }
}

pub(super) async fn discover_skills(
    exec: &dyn Executor,
    project_root: &Path,
    home: Option<&Path>,
) -> Vec<MigratedSkill> {
    let mut skills = Vec::new();
    skills_in(
        exec,
        &project_root.join(".claude/skills"),
        "project",
        &mut skills,
    )
    .await;
    if let Some(home) = home {
        skills_in(exec, &home.join(".claude/skills"), "personal", &mut skills).await;
    }
    let mut seen = HashSet::new();
    skills.retain(|skill| seen.insert(skill.name.clone()));
    skills.sort_by_key(|skill| skill.name.to_lowercase());
    skills
}

pub(super) async fn discover_instructions(
    exec: &dyn Executor,
    project_root: &Path,
) -> Vec<MigratedInstruction> {
    let path = project_root.join("CLAUDE.md");
    match read_text(exec, &path)
        .await
        .filter(|text| !text.trim().is_empty())
    {
        Some(_) => vec![MigratedInstruction {
            path: path.to_string_lossy().to_string(),
            scope: "project",
            source: MigrationSource::Claude,
        }],
        None => Vec::new(),
    }
}
