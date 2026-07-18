use std::collections::HashSet;
use std::path::Path;

use toml::Value;

use super::{MigratedInstruction, MigratedSkill, MigrationSource};
use crate::exec::Executor;
use crate::markdown_frontmatter::{fm_field, frontmatter, read_text};
use crate::mcp::McpServerConfig;

fn parse_mcp_entry(name: &str, value: &Value) -> Option<McpServerConfig> {
    let table = value.as_table()?;
    if table.get("enabled").and_then(Value::as_bool) == Some(false)
        || table.contains_key("url")
        || table.contains_key("cwd")
    {
        return None;
    }
    let command = table.get("command")?.as_str()?.trim();
    if command.is_empty() {
        return None;
    }
    let args = table
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(|arg| arg.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let env = table
        .get("env")
        .and_then(Value::as_table)
        .map(|env| {
            env.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
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

fn collect_mcp(value: &Value, out: &mut Vec<McpServerConfig>, seen: &mut HashSet<String>) {
    let Some(servers) = value.get("mcp_servers").and_then(Value::as_table) else {
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

async fn collect_config(
    exec: &dyn Executor,
    path: &Path,
    out: &mut Vec<McpServerConfig>,
    seen: &mut HashSet<String>,
) {
    let Some(text) = read_text(exec, path).await else {
        return;
    };
    let Ok(value) = toml::from_str::<Value>(&text) else {
        return;
    };
    collect_mcp(&value, out, seen);
}

pub(super) async fn discover_mcp_servers(
    exec: &dyn Executor,
    project_root: &Path,
    home: Option<&Path>,
) -> Vec<McpServerConfig> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect_config(
        exec,
        &project_root.join(".codex/config.toml"),
        &mut out,
        &mut seen,
    )
    .await;
    if let Some(home) = home {
        collect_config(exec, &home.join(".codex/config.toml"), &mut out, &mut seen).await;
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
        source: MigrationSource::Codex,
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
        &project_root.join(".agents/skills"),
        "project",
        &mut skills,
    )
    .await;
    if let Some(home) = home {
        skills_in(exec, &home.join(".agents/skills"), "personal", &mut skills).await;
        skills_in(exec, &home.join(".codex/skills"), "personal", &mut skills).await;
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
    let path = project_root.join("AGENTS.md");
    match read_text(exec, &path)
        .await
        .filter(|text| !text.trim().is_empty())
    {
        Some(_) => vec![MigratedInstruction {
            path: path.to_string_lossy().to_string(),
            scope: "project",
            source: MigrationSource::Codex,
        }],
        None => Vec::new(),
    }
}
