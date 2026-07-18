use std::fs;
use std::path::{Path, PathBuf};

use provider_local::{
    discover_agent_setups_with_home, AgentMigrationDiscovery, LocalExecutor, RemoteExecutor,
};

const TOKEN: &str = "migration-eval-token";

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("parent")).unwrap();
    fs::write(path, contents).unwrap();
}

fn build_fixture(root: &Path, home: &Path) {
    write(
        root.join("CLAUDE.md").as_path(),
        "Claude fixture instructions.",
    );
    write(
        root.join("AGENTS.md").as_path(),
        "Codex fixture instructions.",
    );
    write(
        root.join(".mcp.json").as_path(),
        r#"{"mcpServers":{"claude-fs":{"command":"claude-fixture","args":["--stdio"]}}}"#,
    );
    write(
        root.join(".codex/config.toml").as_path(),
        r#"[mcp_servers.codex_docs]
command = "codex-fixture"
args = ["--stdio"]

[mcp_servers.unsupported_http]
url = "https://example.test/mcp"
"#,
    );
    write(
        root.join(".claude/skills/claude-review/SKILL.md").as_path(),
        "---\nname: claude-review\ndescription: Review with Claude conventions.\n---\n",
    );
    write(
        root.join(".agents/skills/codex-review/SKILL.md").as_path(),
        "---\nname: codex-review\ndescription: Review with Codex conventions.\n---\n",
    );
    write(
        home.join(".agents/skills/personal/SKILL.md").as_path(),
        "---\nname: personal\ndescription: Personal fixture.\n---\n",
    );
}

async fn start_server(root: PathBuf) -> String {
    let server = exec_server::bind(exec_server::Config {
        token: TOKEN.to_string(),
        root: Some(root),
        addr: "127.0.0.1:0".to_string(),
    })
    .await
    .expect("bind exec server");
    let address = server.local_addr().expect("server address");
    tokio::spawn(server.serve());
    format!("ws://{address}")
}

fn assert_fixture(discoveries: &[AgentMigrationDiscovery]) {
    assert_eq!(discoveries.len(), 2);
    assert_eq!(discoveries[0].mcp.len(), 1);
    assert_eq!(discoveries[0].skills.len(), 1);
    assert_eq!(discoveries[0].instructions.len(), 1);
    assert_eq!(discoveries[1].mcp.len(), 1);
    assert_eq!(discoveries[1].skills.len(), 2);
    assert_eq!(discoveries[1].instructions.len(), 1);
}

#[tokio::test]
async fn synthetic_claude_and_codex_migration_matches_local_and_remote() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    build_fixture(&root, &home);

    let local = discover_agent_setups_with_home(&LocalExecutor, &root, Some(&home)).await;
    assert_fixture(&local);

    let remote = RemoteExecutor::connect(&start_server(temp.path().to_path_buf()).await, TOKEN)
        .await
        .expect("connect remote executor");
    let remote = discover_agent_setups_with_home(&remote, &root, Some(&home)).await;
    assert_fixture(&remote);
    assert_eq!(remote, local);

    println!(
        "{}",
        serde_json::json!({
            "eval": "agent_setup_migration",
            "status": "pass",
            "sources": ["claude", "codex"],
            "modes": ["local", "remote_transport"],
            "mcp_servers": 2,
            "skills": 3,
            "instructions": 2,
            "checks": 8,
        })
    );
}
