//! Explicit paid laptop acceptance for the Scientist -> Kimi advisor path.

use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const CHILD_KEY_ENV: &str = "CLARK_SPECIALIST_MODEL_KEY";

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after Unix epoch")
        .as_millis()
}

fn write_private_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    serde_json::to_writer_pretty(&mut file, value).unwrap();
    file.write_all(b"\n").unwrap();
}

fn contains_secret(root: &Path, secret: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            contains_secret(&path, secret)
        } else {
            std::fs::read(&path).is_ok_and(|bytes| {
                bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes())
            })
        }
    })
}

#[test]
#[ignore = "requires explicit paid Clark credential and native Scientist worker"]
fn laptop_scientist_applies_kimi_advice_and_records_feedback() {
    if std::env::var("CLARK_ADVISOR_SPECIALIST_LOCAL").as_deref() != Ok("1") {
        eprintln!("set CLARK_ADVISOR_SPECIALIST_LOCAL=1 to authorize this paid laptop lane");
        return;
    }
    let api_key = std::env::var("CLARK_CODE_API_KEY")
        .expect("CLARK_CODE_API_KEY must contain an explicitly authorized paid credential");
    let api_base = std::env::var("CLARK_ADVISOR_SPECIALIST_API_BASE")
        .unwrap_or_else(|_| "https://api.dev.clarkslabs.com/v1".into());
    let organization_id = std::env::var("CLARK_ADVISOR_SPECIALIST_ORGANIZATION_ID")
        .expect("CLARK_ADVISOR_SPECIALIST_ORGANIZATION_ID is required");
    let worker = PathBuf::from(
        std::env::var("CLARK_ADVISOR_SPECIALIST_LOCAL_WORKER")
            .expect("CLARK_ADVISOR_SPECIALIST_LOCAL_WORKER is required"),
    );
    let receipt_path = PathBuf::from(
        std::env::var("CLARK_ADVISOR_SPECIALIST_LOCAL_RECEIPT")
            .expect("CLARK_ADVISOR_SPECIALIST_LOCAL_RECEIPT is required"),
    );
    let project_root = PathBuf::from(
        std::env::var("CLARK_ADVISOR_SPECIALIST_PROJECT_ROOT")
            .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").into()),
    )
    .canonicalize()
    .unwrap();
    let run_root = receipt_path.parent().unwrap();
    std::fs::create_dir_all(run_root).unwrap();
    let trajectory_root = run_root.join("trajectories");
    let run = now_ms();
    let project_id = format!("advisor-laptop-{run}");
    let config_path = run_root.join("worker.json");
    write_private_json(
        &config_path,
        &json!({
            "schema_version": 1,
            "projects": [{"id": project_id, "root": project_root}],
            "trajectory_root": trajectory_root,
            "execution_residency": "local_only",
            "allowed_evaluator_commands": [],
            "allow_paid_models": true,
            "advisor_training_enabled": false,
            "cloud_sync": {
                "api_base_url": api_base,
                "organization_id": organization_id,
                "scope_id": project_id,
                "api_key_env": CHILD_KEY_ENV
            },
            "provider": {
                "base_url": api_base,
                "model": "clark-code:deepseek_v4_flash_latest",
                "api_key_env": CHILD_KEY_ENV,
                "reasoning_effort": "max",
                "structured_output_mode": "json_object",
                "max_iterations": 3
            }
        }),
    );
    let mut child = Command::new(worker)
        .args(["--config", config_path.to_str().unwrap()])
        .env(CHILD_KEY_ENV, &api_key)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start laptop specialist worker");
    let mut stdin = child.stdin.take().unwrap();
    for request in [
        json!({"schema_version":1,"request_id":"advisor-laptop-ping","command":"ping"}),
        json!({
            "schema_version": 1,
            "request_id": format!("advisor-laptop-turn-{run}"),
            "command": "specialist_turn",
            "session_id": project_id,
            "specialist": "scientist",
            "workflow": "scientist:discover",
            "project_id": project_id,
            "message": "Propose one bounded falsifiable experiment proving that a cloud advisor improves specialist strategy without gaining execution authority. Treat this as intent, not evidence.",
            "now_ms": run
        }),
        json!({"schema_version":1,"request_id":"advisor-laptop-shutdown","command":"shutdown"}),
    ] {
        serde_json::to_writer(&mut stdin, &request).unwrap();
        stdin.write_all(b"\n").unwrap();
    }
    drop(stdin);
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "worker failed: {stderr}");
    let responses = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let data = &responses
        .iter()
        .find(|response| response["kind"] == "specialist_turn")
        .expect("specialist turn response")["data"];
    assert_eq!(data["cloudAdvisor"]["status"], "applied");
    assert_eq!(
        data["cloudAdvisor"]["receipt"]["semanticIndexStatus"],
        "indexed"
    );
    assert_eq!(data["cloudAdvisor"]["receipt"]["semanticRecordCount"], 6);
    assert_eq!(data["cloudAdvisor"]["feedback"]["status"], "recorded");
    assert!(data["cloudAdvisor"]["usage"]["cost"]
        .as_f64()
        .is_some_and(|cost| cost > 0.0));
    assert!(!contains_secret(run_root, &api_key));
    write_private_json(
        &receipt_path,
        &json!({
            "schema_version": 1,
            "status": "passed",
            "execution_residency": "local_only",
            "project_id": project_id,
            "cloud_advisor": data["cloudAdvisor"],
            "cloud_sync": data["cloudSync"],
            "credential_recorded": false,
            "generated_at_ms": now_ms()
        }),
    );
}
