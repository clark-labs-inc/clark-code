use std::process::Stdio;

use code_host::PROTOCOL_VERSION;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[tokio::test]
async fn worker_exposes_strict_ping_catalog_and_shutdown_protocol() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let trajectory = temp.path().join("trajectory");
    tokio::fs::create_dir_all(&project).await.unwrap();
    let config = serde_json::json!({
        "schema_version": 1,
        "worker_name": "test-worker",
        "projects": [{"id": "fixture", "root": project}],
        "trajectory_root": trajectory,
        "enabled_plugins": ["coding"],
        "max_concurrent_requests": 1,
        "provider": {
            "base_url": "http://127.0.0.1:11434/v1",
            "model": "local-model",
            "allowed_tools": []
        }
    });
    let config_path = temp.path().join("worker.json");
    tokio::fs::write(&config_path, serde_json::to_vec(&config).unwrap())
        .await
        .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-code-worker"))
        .arg("--config")
        .arg(&config_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();

    let ping_request = format!(
        "{{\"schema_version\":{PROTOCOL_VERSION},\"request_id\":\"ping-1\",\"command\":\"ping\"}}\n"
    );
    stdin.write_all(ping_request.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
    let ping: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(ping["type"], "result");
    assert_eq!(ping["request_id"], "ping-1");
    assert_eq!(ping["data"]["worker"], "test-worker");

    let catalog_request = format!(
        "{{\"schema_version\":{PROTOCOL_VERSION},\"request_id\":\"catalog-1\",\"command\":\"catalog\"}}\n"
    );
    stdin.write_all(catalog_request.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
    let catalog: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(catalog["type"], "result");
    assert_eq!(catalog["data"]["plugins"][0]["id"], "coding");

    let invalid_request = format!(
        "{{\"schema_version\":{PROTOCOL_VERSION},\"request_id\":\"bad-1\",\"command\":\"ping\",\"surprise\":true}}\n"
    );
    stdin.write_all(invalid_request.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
    let invalid: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(invalid["type"], "error");
    assert_eq!(invalid["code"], "invalid_request");

    let shutdown_request = format!(
        "{{\"schema_version\":{PROTOCOL_VERSION},\"request_id\":\"shutdown-1\",\"command\":\"shutdown\"}}\n"
    );
    stdin.write_all(shutdown_request.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
    let shutdown: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(shutdown["type"], "result");
    assert_eq!(shutdown["kind"], "shutdown");
    drop(stdin);
    assert!(child.wait().await.unwrap().success());
}
