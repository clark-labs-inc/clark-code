use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::*;
use crate::exec::LocalExecutor;

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("parent")).unwrap();
    fs::write(path, contents).unwrap();
}

fn skill(root: &Path, name: &str, description: &str) {
    write(
        &root.join(name).join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: {description}\n---\n\nBody."),
    );
}

fn mcp(name: &str, command: &str) -> McpServerConfig {
    McpServerConfig {
        credential_ref: None,
        name: name.to_string(),
        command: command.to_string(),
        args: Vec::new(),
        env: HashMap::new(),
    }
}

#[tokio::test]
async fn synthetic_claude_and_openai_setups_are_detected_without_source_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&root).unwrap();

    write(
        &root.join(".mcp.json"),
        r#"{"mcpServers":{"shared":{"command":"claude-project"},"claude-project":{"command":"claude-only"},"remote":{"type":"http","url":"https://example.test"}}}"#,
    );
    write(
        &home.join(".claude.json"),
        r#"{"mcpServers":{"claude-global":{"command":"claude-global"},"shared":{"command":"claude-global-shared"}}}"#,
    );
    skill(
        &root.join(".claude/skills"),
        "shared-skill",
        "Claude project",
    );
    skill(&root.join(".claude/skills"), "claude-skill", "Claude only");
    skill(
        &home.join(".claude/skills"),
        "personal-claude",
        "Personal Claude",
    );
    write(&root.join("CLAUDE.md"), "Use the Claude fixture rules.");

    write(
        &root.join(".codex/config.toml"),
        r#"[mcp_servers.shared]
command = "codex-project"

[mcp_servers.codex-project]
command = "codex-only"
args = ["--fixture"]

[mcp_servers.http]
url = "https://example.test/mcp"

[mcp_servers.disabled]
command = "disabled"
enabled = false

[mcp_servers.custom-cwd]
command = "cwd-server"
cwd = "/tmp"
"#,
    );
    write(
        &home.join(".codex/config.toml"),
        r#"[mcp_servers.codex-global]
command = "codex-global"

[mcp_servers.shared]
command = "codex-global-shared"
"#,
    );
    skill(
        &root.join(".agents/skills"),
        "shared-skill",
        "OpenAI project",
    );
    skill(&root.join(".agents/skills"), "openai-skill", "OpenAI only");
    skill(
        &home.join(".agents/skills"),
        "personal-codex",
        "Personal OpenAI",
    );
    write(&root.join("AGENTS.md"), "Use the external fixture rules.");

    let before = walkdir::WalkDir::new(temp.path())
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.path().to_path_buf())
        .collect::<Vec<_>>();
    let actual = discover_agent_setups_with_home(&LocalExecutor, &root, Some(&home)).await;
    let after = walkdir::WalkDir::new(temp.path())
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.path().to_path_buf())
        .collect::<Vec<_>>();

    assert_eq!(before, after, "discovery must be read-only");
    assert_eq!(actual.len(), 2);
    assert_eq!(actual[0].source, MigrationSource::Claude);
    assert_eq!(
        actual[0].mcp,
        vec![
            mcp("claude-global", "claude-global"),
            mcp("claude-project", "claude-only"),
            mcp("shared", "claude-project"),
        ]
    );
    assert_eq!(
        actual[0]
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>(),
        vec!["claude-skill", "personal-claude", "shared-skill"]
    );
    assert_eq!(actual[0].instructions.len(), 1);

    assert_eq!(actual[1].source, MigrationSource::Openai);
    assert_eq!(
        actual[1].mcp,
        vec![
            mcp("codex-global", "codex-global"),
            McpServerConfig {
                credential_ref: None,
                name: "codex-project".to_string(),
                command: "codex-only".to_string(),
                args: vec!["--fixture".to_string()],
                env: HashMap::new(),
            },
            mcp("shared", "codex-project"),
        ]
    );
    assert_eq!(
        actual[1]
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>(),
        vec!["openai-skill", "personal-codex", "shared-skill"]
    );
    assert_eq!(actual[1].instructions.len(), 1);
}
