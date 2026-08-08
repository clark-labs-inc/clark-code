use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use code_host::{
    HeadlessHost, HeadlessPlugin, PluginContext, PluginError, PluginManifest, ProjectRegistration,
    ProjectRegistry, Request, RequestCommand, Response, PROTOCOL_VERSION,
};
use serde_json::{json, Value};

struct CountingPlugin {
    calls: Arc<AtomicUsize>,
    manifest: PluginManifest,
}

#[async_trait::async_trait]
impl HeadlessPlugin for CountingPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        context: PluginContext,
        _operation: &str,
        input: Value,
    ) -> Result<Value, PluginError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        context.progress.emit("step", json!({"index": 0})).await?;
        context.progress.emit("step", json!({"index": 1})).await?;
        Ok(json!({"echo": input}))
    }
}

fn host(root: &std::path::Path, calls: Arc<AtomicUsize>) -> HeadlessHost {
    let projects = ProjectRegistry::new([ProjectRegistration {
        id: "fixture".into(),
        root: root.to_path_buf(),
    }])
    .unwrap();
    let mut host = HeadlessHost::new(projects, root.join("trajectory"));
    host.register_plugin(CountingPlugin {
        calls,
        manifest: PluginManifest {
            id: "counting".into(),
            version: "1.0.0".into(),
            description: "idempotency test plugin".into(),
            operations: BTreeSet::from(["run".into()]),
            capabilities: BTreeSet::new(),
        },
    })
    .unwrap();
    host
}

fn request(input: Value) -> Request {
    Request {
        schema_version: PROTOCOL_VERSION,
        request_id: "durable-request-1".into(),
        command: RequestCommand::Invoke {
            plugin: "counting".into(),
            operation: "run".into(),
            project_id: Some("fixture".into()),
            input,
        },
    }
}

async fn invoke(host: &HeadlessHost, input: Value) -> (Response, Vec<Response>) {
    let (output, mut progress) = tokio::sync::mpsc::channel(8);
    let terminal = host.handle_stream(request(input), output).await;
    let mut frames = Vec::new();
    while let Ok(frame) = progress.try_recv() {
        frames.push(frame);
    }
    (terminal, frames)
}

#[tokio::test]
async fn completed_request_replays_after_worker_restart_without_reexecution() {
    let root = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let first_host = host(root.path(), calls.clone());
    let (first_terminal, first_progress) = invoke(&first_host, json!({"value": 7})).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first_progress.len(), 2);
    drop(first_host);

    let restarted_host = host(root.path(), calls.clone());
    let (replayed_terminal, replayed_progress) = invoke(&restarted_host, json!({"value": 7})).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(replayed_progress.len(), 2);
    assert_eq!(
        serde_json::to_value(replayed_terminal).unwrap(),
        serde_json::to_value(first_terminal).unwrap()
    );
    assert_eq!(
        serde_json::to_value(replayed_progress).unwrap(),
        serde_json::to_value(first_progress).unwrap()
    );
}

#[tokio::test]
async fn reused_request_id_with_different_work_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let host = host(root.path(), calls.clone());
    let (first, _) = invoke(&host, json!({"value": 7})).await;
    assert!(matches!(first, Response::Result { .. }));

    let (conflict, progress) = invoke(&host, json!({"value": 8})).await;
    assert!(progress.is_empty());
    assert!(matches!(
        conflict,
        Response::Error { ref code, .. } if code == "request_id_conflict"
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn interrupted_request_receipt_is_ambiguous_and_never_reexecutes() {
    let root = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let first_host = host(root.path(), calls.clone());
    let (first, _) = invoke(&first_host, json!({"value": 7})).await;
    assert!(matches!(first, Response::Result { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(first_host);

    let receipts = root.path().join("trajectory/request-receipts");
    let receipt = std::fs::read_dir(&receipts)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let contents = std::fs::read_to_string(&receipt).unwrap();
    let started = contents.lines().next().unwrap();
    std::fs::write(&receipt, format!("{started}\n")).unwrap();

    let restarted_host = host(root.path(), calls.clone());
    let (ambiguous, progress) = invoke(&restarted_host, json!({"value": 7})).await;
    assert!(progress.is_empty());
    assert!(matches!(
        ambiguous,
        Response::Error { ref code, .. } if code == "ambiguous_request"
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
