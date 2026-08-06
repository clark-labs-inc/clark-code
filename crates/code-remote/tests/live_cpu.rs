//! Explicit live SSH receipt for the complete remote-worker path.
//!
//! This test intentionally does not run in normal CI. Set
//! `CLARK_REMOTE_CPU_LIVE=1` and provide a worker binary path to exercise the
//! real `cpu` host. It authenticates the worker's required Clark access
//! reconciliation, then proves residency/catalog correlation and shuts down.
//! It does not make a model call; a paid model turn is a separate opt-in test.

use code_host::{Request, RequestCommand, Response, PROTOCOL_VERSION};
use code_remote::{RemoteWorker, RemoteWorkerFrame, RemoteWorkerSpec};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn write_receipt(path: &Path, receipt: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {}: {error}", path.display()))
}

#[test]
fn paid_receipt_writer_creates_missing_parent_directories() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nested/receipt.json");
    write_receipt(&path, &json!({"status": "passed"})).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap(),
        json!({"status": "passed"})
    );
}

#[tokio::test]
#[ignore = "requires explicit CLARK_REMOTE_CPU_LIVE=1 and SSH access"]
async fn cpu_remote_worker_ping_catalog_shutdown() {
    if std::env::var("CLARK_REMOTE_CPU_LIVE").as_deref() != Ok("1") {
        eprintln!("set CLARK_REMOTE_CPU_LIVE=1 to run the live CPU receipt");
        return;
    }
    let host = std::env::var("CLARK_REMOTE_CPU_HOST").unwrap_or_else(|_| "cpu".into());
    let project_id = "clark-remote-smoke";
    let remote_root = PathBuf::from(
        std::env::var("CLARK_REMOTE_CPU_ROOT")
            .unwrap_or_else(|_| "/tmp/clark-code-remote-smoke".into()),
    );
    let trajectory_root = PathBuf::from(
        std::env::var("CLARK_REMOTE_CPU_TRAJECTORY")
            .unwrap_or_else(|_| "/tmp/clark-code-remote-trajectory".into()),
    );
    let worker_binary = PathBuf::from(
        std::env::var("CLARK_REMOTE_CPU_WORKER")
            .expect("CLARK_REMOTE_CPU_WORKER must point to a built Linux worker"),
    );
    let credential_env = std::env::var("CLARK_REMOTE_CPU_CREDENTIAL_ENV")
        .expect("CLARK_REMOTE_CPU_CREDENTIAL_ENV must name the Clark credential environment");
    let config = json!({
        "schema_version": 1,
        "projects": [{"id": project_id, "root": remote_root}],
        "trajectory_root": trajectory_root,
        "execution_residency": "remote_worker",
        "provider": {
            "base_url": "https://api.clarkslabs.com/v1",
            "api_key_env": credential_env,
            "model": "clark-code:free"
        }
    });
    let spec = RemoteWorkerSpec {
        host: host.clone(),
        project_id: project_id.into(),
        remote_root: remote_root.clone(),
        trajectory_root,
        worker_config: config,
        local_binary: Some(worker_binary),
        local_binaries: Default::default(),
        remote_binary: None,
        credential_envs: vec![std::env::var("CLARK_REMOTE_CPU_CREDENTIAL_ENV")
            .expect("credential env was read above")],
    };
    let total_started = Instant::now();
    let connect_started = Instant::now();
    let worker = RemoteWorker::connect(spec)
        .await
        .expect("remote worker connect");
    let connect_duration_ms = connect_started.elapsed().as_millis();
    assert_eq!(worker.info().execution_residency, "remote_worker");
    assert_eq!(worker.info().ssh_transport, "control_master");
    let worker_info = worker.info().clone();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let session_request_id = format!("session-open-live-{run}");
    let session_id = format!("session-live-{run}");
    let session_request = Request {
        schema_version: PROTOCOL_VERSION,
        request_id: session_request_id.clone(),
        command: RequestCommand::Invoke {
            plugin: "coding".into(),
            operation: "session.open".into(),
            project_id: Some(project_id.into()),
            input: json!({
                "session_id": session_id,
                "options": { "cwd": remote_root },
            }),
        },
    };
    let session = worker
        .request(session_request.clone())
        .await
        .expect("session-open response");
    assert!(
        matches!(&session, Response::Result { kind, .. } if kind == "plugin_result"),
        "session.open failed: {session:?}"
    );
    let replayed = match worker.request(session_request).await {
        Ok(response) => response,
        Err(error) => {
            let diagnostics = worker.stderr().await;
            panic!("idempotent session-open replay failed: {error}; worker stderr: {diagnostics}");
        }
    };
    assert_eq!(
        serde_json::to_value(replayed).unwrap(),
        serde_json::to_value(&session).unwrap(),
        "same request id must replay its terminal response"
    );
    let conflict_request = Request {
        schema_version: PROTOCOL_VERSION,
        request_id: session_request_id,
        command: RequestCommand::Invoke {
            plugin: "coding".into(),
            operation: "session.open".into(),
            project_id: Some(project_id.into()),
            input: json!({
                "session_id": format!("conflicting-session-{run}"),
                "options": { "cwd": remote_root },
            }),
        },
    };
    let conflict = match worker.request(conflict_request).await {
        Ok(response) => response,
        Err(error) => {
            let diagnostics = worker.stderr().await;
            panic!("conflicting request failed: {error}; worker stderr: {diagnostics}");
        }
    };
    assert!(matches!(
        conflict,
        Response::Error { ref code, .. } if code == "request_id_conflict"
    ));
    let catalog_started = Instant::now();
    let catalog = worker
        .request(Request {
            schema_version: PROTOCOL_VERSION,
            request_id: format!("catalog-live-{run}"),
            command: RequestCommand::Catalog,
        })
        .await
        .expect("catalog response");
    let catalog_duration_ms = catalog_started.elapsed().as_millis();
    assert!(matches!(catalog, Response::Result { kind, .. } if kind == "catalog"));
    let shutdown_started = Instant::now();
    worker.disconnect().await.expect("remote worker shutdown");
    let shutdown_duration_ms = shutdown_started.elapsed().as_millis();
    if let Ok(receipt_path) = std::env::var("CLARK_REMOTE_CPU_RECEIPT") {
        write_receipt(
            Path::new(&receipt_path),
            &json!({
                "schema_version": 1,
                "benchmark": "clark_code_remote_cpu_transport",
                "status": "passed",
                "generated_at_ms": SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after Unix epoch")
                    .as_millis(),
                "host": host,
                "worker": worker_info.worker,
                "worker_version": worker_info.worker_version,
                "arch": worker_info.arch,
                "binary_sha256": worker_info.binary_sha256,
                "execution_residency": worker_info.execution_residency,
                "ssh_transport": worker_info.ssh_transport,
                "connect_duration_ms": connect_duration_ms,
                "catalog_duration_ms": catalog_duration_ms,
                "shutdown_duration_ms": shutdown_duration_ms,
                "total_duration_ms": total_started.elapsed().as_millis(),
                "idempotent_replay_verified": true,
                "conflicting_request_rejected": true,
                "credential_recorded": false,
                "model_called": false,
            }),
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }
}

#[tokio::test]
#[ignore = "requires explicit paid model, credential env, and CLARK_REMOTE_CPU_PAID=1"]
async fn cpu_remote_worker_paid_coding_turn() {
    if std::env::var("CLARK_REMOTE_CPU_PAID").as_deref() != Ok("1") {
        eprintln!("set CLARK_REMOTE_CPU_PAID=1 to run the paid CPU turn");
        return;
    }
    let model = std::env::var("CLARK_REMOTE_CPU_MODEL")
        .expect("CLARK_REMOTE_CPU_MODEL must name the paid route explicitly");
    let receipt_path = PathBuf::from(
        std::env::var("CLARK_REMOTE_CPU_RECEIPT")
            .expect("CLARK_REMOTE_CPU_RECEIPT must retain the paid run receipt"),
    );
    let credential_env = std::env::var("CLARK_REMOTE_CPU_CREDENTIAL_ENV")
        .expect("CLARK_REMOTE_CPU_CREDENTIAL_ENV must name the local key variable");
    if std::env::var(&credential_env)
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        panic!("the configured paid credential environment variable is unset or empty");
    }
    let host = std::env::var("CLARK_REMOTE_CPU_HOST").unwrap_or_else(|_| "cpu".into());
    let base_url = std::env::var("CLARK_REMOTE_CPU_BASE_URL")
        .unwrap_or_else(|_| "https://api.clarkslabs.com/v1".into());
    let project_id = "clark-paid-remote-smoke";
    let remote_root = PathBuf::from(
        std::env::var("CLARK_REMOTE_CPU_ROOT")
            .unwrap_or_else(|_| "/tmp/clark-code-paid-remote-smoke".into()),
    );
    let trajectory_root = PathBuf::from(
        std::env::var("CLARK_REMOTE_CPU_TRAJECTORY")
            .unwrap_or_else(|_| "/tmp/clark-code-paid-trajectory".into()),
    );
    let worker_binary = PathBuf::from(
        std::env::var("CLARK_REMOTE_CPU_WORKER")
            .expect("CLARK_REMOTE_CPU_WORKER must point to a built Linux worker"),
    );
    let config = json!({
        "schema_version": 1,
        "projects": [{"id": project_id, "root": remote_root}],
        "trajectory_root": trajectory_root,
        "execution_residency": "remote_worker",
        "provider": {
            "base_url": base_url,
            "api_key_env": credential_env,
            "model": model,
            "max_iterations": 2
        }
    });
    let spec = RemoteWorkerSpec {
        host: host.clone(),
        project_id: project_id.into(),
        remote_root,
        trajectory_root,
        worker_config: config,
        local_binary: Some(worker_binary),
        local_binaries: Default::default(),
        remote_binary: None,
        credential_envs: vec![std::env::var("CLARK_REMOTE_CPU_CREDENTIAL_ENV")
            .expect("credential env was read above")],
    };
    let worker = RemoteWorker::connect(spec)
        .await
        .expect("paid worker connect");
    let opened = worker
        .request(Request {
            schema_version: PROTOCOL_VERSION,
            request_id: "paid-open".into(),
            command: RequestCommand::Invoke {
                plugin: "coding".into(),
                operation: "session.open".into(),
                project_id: Some(project_id.into()),
                input: json!({}),
            },
        })
        .await
        .expect("paid session.open response");
    let session_id = match opened {
        Response::Result { data, .. } => data["session_id"]
            .as_str()
            .expect("session.open returned session_id")
            .to_owned(),
        other => panic!("session.open failed: {other:?}"),
    };
    let mut prompted = worker
        .start_request(Request {
            schema_version: PROTOCOL_VERSION,
            request_id: "paid-prompt".into(),
            command: RequestCommand::Invoke {
                plugin: "coding".into(),
                operation: "session.prompt".into(),
                project_id: Some(project_id.into()),
                input: json!({
                    "session_id": session_id,
                    "input": {
                        "blocks": [{
                            "type": "text",
                            "text": "Reply with exactly REMOTE_CPU_PAID_OK and do not use tools."
                        }]
                    }
                }),
            },
        })
        .await
        .expect("paid session.prompt request");
    let mut streamed_events = Vec::new();
    let terminal = loop {
        match prompted.next().await.expect("paid session.prompt frame") {
            RemoteWorkerFrame::Progress(progress) if progress.kind == "agent_event" => {
                streamed_events.push(progress.data);
            }
            RemoteWorkerFrame::Progress(_) => {}
            RemoteWorkerFrame::Terminal(response) => break response,
        }
    };
    let serialized = serde_json::to_string(&streamed_events).expect("events serialize");
    assert!(
        serialized.contains("REMOTE_CPU_PAID_OK"),
        "paid response did not contain marker"
    );
    assert!(matches!(terminal, Response::Result { .. }));
    let usage = streamed_events
        .iter()
        .rev()
        .find_map(|event| (event["event"] == "run_usage_updated").then(|| event["usage"].clone()))
        .expect("paid response retained cumulative usage");
    assert!(
        usage["cost_usd"].as_f64().is_some_and(|cost| cost > 0.0),
        "paid response must report positive cost: {usage}"
    );
    worker.disconnect().await.expect("paid worker shutdown");
    let generated_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after Unix epoch")
        .as_millis();
    write_receipt(
        &receipt_path,
        &json!({
            "schema_version": 1,
            "benchmark": "clark_code_remote_cpu_paid",
            "status": "passed",
            "generated_at_ms": generated_at_ms,
            "host": host,
            "execution_residency": "remote_worker",
            "provider": "clark-platform",
            "base_url": base_url,
            "model": model,
            "credential_env": credential_env,
            "credential_recorded": false,
            "first_failure": null,
            "usage": usage,
            "terminal": terminal,
            "events": streamed_events,
        }),
    )
    .unwrap_or_else(|error| panic!("{error}"));
}
