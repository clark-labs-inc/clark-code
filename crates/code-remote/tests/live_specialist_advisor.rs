//! Explicit paid SSH acceptance for the Scientist -> Kimi advisor path.

use code_host::{Request, RequestCommand, Response, PROTOCOL_VERSION};
use code_remote::{RemoteWorker, RemoteWorkerSpec};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
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

#[tokio::test]
#[ignore = "requires explicit paid Clark credential, Linux worker, and SSH cpu access"]
async fn ssh_scientist_applies_kimi_advice_and_records_feedback() {
    if std::env::var("CLARK_ADVISOR_SPECIALIST_REMOTE").as_deref() != Ok("1") {
        eprintln!("set CLARK_ADVISOR_SPECIALIST_REMOTE=1 to authorize this paid SSH lane");
        return;
    }
    let api_key = std::env::var("CLARK_CODE_API_KEY")
        .expect("CLARK_CODE_API_KEY must contain an explicitly authorized paid credential");
    let api_base = std::env::var("CLARK_ADVISOR_SPECIALIST_API_BASE")
        .unwrap_or_else(|_| "https://api.dev.clarkslabs.com/v1".into());
    let organization_id = std::env::var("CLARK_ADVISOR_SPECIALIST_ORGANIZATION_ID")
        .expect("CLARK_ADVISOR_SPECIALIST_ORGANIZATION_ID is required");
    let worker_binary = PathBuf::from(
        std::env::var("CLARK_ADVISOR_SPECIALIST_LINUX_WORKER")
            .expect("CLARK_ADVISOR_SPECIALIST_LINUX_WORKER is required"),
    );
    let expected_worker_sha256 = format!(
        "{:x}",
        Sha256::digest(std::fs::read(&worker_binary).expect("read Linux worker"))
    );
    let receipt_path = PathBuf::from(
        std::env::var("CLARK_ADVISOR_SPECIALIST_REMOTE_RECEIPT")
            .expect("CLARK_ADVISOR_SPECIALIST_REMOTE_RECEIPT is required"),
    );
    let host = std::env::var("CLARK_ADVISOR_SPECIALIST_HOST").unwrap_or_else(|_| "cpu".into());
    let run = now_ms();
    let project_id = format!("advisor-ssh-{run}");
    let remote_root = PathBuf::from(format!("/tmp/{project_id}"));
    let trajectory_root = remote_root.join(".clark/specialist-trajectory");
    let worker_config = json!({
        "schema_version": 1,
        "projects": [{"id": project_id, "root": remote_root}],
        "trajectory_root": trajectory_root,
        "execution_residency": "remote_worker",
        "allowed_evaluator_commands": [],
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
    });
    let worker = RemoteWorker::connect_with_credentials(
        RemoteWorkerSpec {
            host: host.clone(),
            project_id: project_id.clone(),
            remote_root: remote_root.clone(),
            trajectory_root,
            worker_config,
            local_binary: Some(worker_binary),
            local_binaries: Default::default(),
            remote_binary: None,
            credential_envs: vec![CHILD_KEY_ENV.into()],
        },
        HashMap::from([(CHILD_KEY_ENV.into(), api_key)]),
    )
    .await
    .expect("connect verified remote worker");
    assert_eq!(worker.info().execution_residency, "remote_worker");
    assert_eq!(worker.info().binary_sha256, expected_worker_sha256);

    let response = worker
        .request(Request {
            schema_version: PROTOCOL_VERSION,
            request_id: format!("advisor-ssh-turn-{run}"),
            command: RequestCommand::Invoke {
                plugin: "scientist".into(),
                operation: "turn".into(),
                project_id: Some(project_id.clone()),
                input: json!({
                    "session_id": project_id,
                    "specialist": "scientist",
                    "workflow": "scientist:discover",
                    "project_id": project_id,
                    "message": "Propose one bounded falsifiable experiment proving that a cloud advisor improves specialist strategy without gaining execution authority. Treat this as intent, not evidence.",
                    "now_ms": run
                }),
            },
        })
        .await
        .expect("remote specialist turn");
    worker.disconnect().await.expect("remote worker shutdown");
    let data = match response {
        Response::Result { kind, data, .. } if kind == "invoke_result" => data,
        other => panic!("remote specialist turn failed: {other:?}"),
    };
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
    let file_count = data["cloudSync"]["file_count"].as_u64().unwrap_or(0);
    let verified_segments = data["cloudSync"]["verified_segment_count"]
        .as_u64()
        .unwrap_or(0);
    assert!(file_count == 0 || verified_segments > 0);
    write_private_json(
        &receipt_path,
        &json!({
            "schema_version": 1,
            "status": "passed",
            "host": host,
            "execution_residency": "remote_worker",
            "project_id": project_id,
            "worker": worker.info(),
            "cloud_advisor": data["cloudAdvisor"],
            "cloud_sync": data["cloudSync"],
            "credential_recorded": false,
            "generated_at_ms": now_ms()
        }),
    );
}
