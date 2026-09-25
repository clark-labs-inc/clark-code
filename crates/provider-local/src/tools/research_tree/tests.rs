use super::*;
use crate::tools::{ReadTracker, ToolPermissionClass, ToolRegistry};
use std::sync::{Arc, Mutex};

fn context(root: &std::path::Path) -> ToolCtx {
    ToolCtx {
        sandbox: Arc::new(crate::sandbox::Sandbox::new(root).unwrap()),
        executor: Arc::new(crate::exec::LocalExecutor),
        reads: Arc::new(Mutex::new(ReadTracker::default())),
        cancel: tokio_util::sync::CancellationToken::new(),
        background: Arc::new(crate::background::BackgroundTasks::default()),
        session: Arc::new(tokio::sync::Mutex::new(
            crate::loop_state::SessionState::default(),
        )),
        progress: None,
        agent_progress: None,
        call_progress: None,
        model_override: None,
    }
}

fn begin() -> Value {
    json!({"action":"begin","tree_id":"test","objective":"Check ownership",
        "scope":"Local fixture only","sources":["source.rs"],"max_nodes":16,"max_steps":4})
}

fn proposal() -> Value {
    json!({"type":"proposed","key":"ownership","parent":null,"links":[],"dependencies":[],
        "evidence_refs":["source.rs:1"],"rationale":"Trace ownership check",
        "assessment":{"surface":"hypothesis","in_scope":true,"requires_validation":true,
            "priority":0.7,"novelty":0.5,"confidence":0.4,"impact":0.8}})
}

async fn update(ctx: &ToolCtx, revision: usize, event: Value) -> ToolOutcome {
    ResearchTreeTool
        .invoke(
            json!({"action":"apply","tree_id":"test",
        "expected_revision":revision,"event":event}),
            ctx,
        )
        .await
}

#[tokio::test]
async fn tree_storage_enforces_revisions_sources_and_read_only_status() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("source.rs"), "initial").unwrap();
    let ctx = context(temp.path());
    assert!(!ResearchTreeTool.invoke(begin(), &ctx).await.is_error);
    let result = update(&ctx, 1, proposal()).await;
    assert!(!result.is_error, "{}", result.content);
    let path = temp.path().join(".agent/research-trees/test.json");
    let before = std::fs::read(&path).unwrap();
    assert!(update(&ctx, 1, proposal()).await.is_error);
    assert_eq!(std::fs::read(&path).unwrap(), before);

    std::fs::write(temp.path().join("source.rs"), "changed").unwrap();
    let blocked = ResearchTreeTool
        .invoke(
            json!({"action":"select","tree_id":"test","expected_revision":2}),
            &ctx,
        )
        .await;
    assert!(blocked.is_error);
    assert!(blocked.content.contains("snapshot changed"));
    let status = ResearchTreeStatus
        .invoke(json!({"tree_id":"test"}), &ctx)
        .await;
    assert!(!status.is_error);
    assert_eq!(status.details["snapshot_current"], false);
    assert_eq!(status.details["revision"], 2);
    assert_eq!(std::fs::read(&path).unwrap(), before);

    std::fs::remove_file(temp.path().join("source.rs")).unwrap();
    let status = ResearchTreeStatus
        .invoke(json!({"tree_id":"test"}), &ctx)
        .await;
    assert!(!status.is_error);
    assert_eq!(status.details["snapshot_current"], false);
    assert!(status.details["snapshot_error"].is_string());
}

#[tokio::test]
async fn canceled_and_direct_started_updates_leave_journal_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("source.rs"), "source").unwrap();
    let ctx = context(temp.path());
    assert!(!ResearchTreeTool.invoke(begin(), &ctx).await.is_error);
    let path = temp.path().join(".agent/research-trees/test.json");
    let before = std::fs::read(&path).unwrap();
    assert!(
        update(&ctx, 1, json!({"type":"started","key":"ownership"}))
            .await
            .is_error
    );
    ctx.cancel.cancel();
    assert!(update(&ctx, 1, proposal()).await.is_error);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn resumed_active_work_is_not_success_and_cannot_be_selected_again() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("source.rs"), "source").unwrap();
    let ctx = context(temp.path());
    assert!(!ResearchTreeTool.invoke(begin(), &ctx).await.is_error);
    assert!(!update(&ctx, 1, proposal()).await.is_error);
    let selected = ResearchTreeTool
        .invoke(
            json!({"action":"select","tree_id":"test","expected_revision":2}),
            &ctx,
        )
        .await;
    assert!(!selected.is_error, "{}", selected.content);
    let fresh_ctx = context(temp.path());
    let status = ResearchTreeStatus
        .invoke(json!({"tree_id":"test"}), &fresh_ctx)
        .await;
    assert_eq!(status.details["steps_used"], 1);
    assert_eq!(status.details["nodes"][0]["status"], "active");
    assert!(status.details["nodes"][0]["outcome"].is_null());
    assert_eq!(status.details["frontier"], json!([]));
    let blocked = update(
        &fresh_ctx,
        3,
        json!({"type":"recorded","key":"ownership",
        "outcome":"blocked","evidence_refs":[],"rationale":"Interrupted before an observation"}),
    )
    .await;
    assert!(!blocked.is_error, "{}", blocked.content);
    assert!(
        update(
            &fresh_ctx,
            4,
            json!({"type":"reopened","key":"ownership",
        "evidence_refs":["source.rs:1"],"rationale":"Try unchanged evidence again"})
        )
        .await
        .is_error
    );
    let reopen = update(
        &fresh_ctx,
        4,
        json!({"type":"reopened","key":"ownership",
        "evidence_refs":["new-test-receipt"],"rationale":"A new harness resolves the blocker"}),
    )
    .await;
    assert!(!reopen.is_error, "{}", reopen.content);
}

#[tokio::test]
async fn scoped_artifacts_stay_in_scope_and_unbound_sources_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("subdir")).unwrap();
    let mut ctx = context(temp.path());
    ctx.sandbox = Arc::new(
        ctx.sandbox
            .as_ref()
            .clone()
            .with_task_scope("subdir")
            .unwrap(),
    );
    let mut request = begin();
    request["sources"] = json!([]);
    let result = ResearchTreeTool.invoke(request, &ctx).await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.details["snapshot_bound"], false);
    assert!(result.details["snapshot_current"].is_null());
    assert!(temp
        .path()
        .join("subdir/.agent/research-trees/test.json")
        .exists());
    assert!(!temp.path().join(".agent").exists());
}

#[test]
fn storage_lock_corruption_and_path_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let store = store::Store::new(temp.path(), "test").unwrap();
    let lock = store.lock().unwrap();
    assert!(store.lock().is_err());
    drop(lock);
    assert!(store.lock().is_ok());
    std::fs::write(
        store.path(),
        r#"{"version":1,"events":[{"type":"started","key":"fake"}]}"#,
    )
    .unwrap();
    assert!(store.load().is_err());
    assert!(store::Store::new(temp.path(), "../outside").is_err());
    assert!(store
        .create(
            "o".into(),
            "s".into(),
            vec!["../outside".into()],
            Limits {
                max_nodes: 1,
                max_steps: 1
            }
        )
        .is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_and_fifo_sources_or_artifacts_are_rejected_without_hanging() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let ctx = context(temp.path());
    symlink(outside.path(), temp.path().join(".agent")).unwrap();
    assert!(ResearchTreeTool.invoke(begin(), &ctx).await.is_error);
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    std::fs::remove_file(temp.path().join(".agent")).unwrap();
    let fifo = temp.path().join("source.rs");
    let name = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        ResearchTreeTool.invoke(begin(), &ctx),
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("regular files"));
}

#[test]
fn tools_have_distinct_permission_classes_and_ordered_schemas() {
    let registry = ToolRegistry::new(None);
    let mutation = registry.get("research_tree").unwrap();
    let status = registry.get("research_tree_status").unwrap();
    assert!(mutation.mutating());
    assert_eq!(
        mutation.permission_class(),
        ToolPermissionClass::LocalMutation
    );
    assert!(!status.mutating());
    assert_eq!(status.permission_class(), ToolPermissionClass::LocalRead);
    let schema = mutation.parameters();
    let keys: Vec<_> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(&keys[..3], ["action", "tree_id", "expected_revision"]);
    let properties = &schema["properties"]["event"]["properties"];
    let keys: Vec<_> = properties
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        keys.iter().position(|v| *v == "evidence_refs")
            < keys.iter().position(|v| *v == "assessment")
    );
}

#[test]
fn cancellation_at_persistence_boundary_keeps_committed_revision() {
    let temp = tempfile::tempdir().unwrap();
    let store = store::Store::new(temp.path(), "cancel").unwrap();
    let _lock = store.lock().unwrap();
    let mut journal = store
        .create(
            "Question".into(),
            "Local only".into(),
            vec![],
            Limits {
                max_nodes: 8,
                max_steps: 8,
            },
        )
        .unwrap();
    let live = tokio_util::sync::CancellationToken::new();
    store.save(&journal, &live).unwrap();
    let before = std::fs::read(store.path()).unwrap();
    journal
        .events
        .push(serde_json::from_value(proposal()).unwrap());
    let canceled = tokio_util::sync::CancellationToken::new();
    canceled.cancel();
    assert!(store.save(&journal, &canceled).is_err());
    assert_eq!(std::fs::read(store.path()).unwrap(), before);
    assert_eq!(store.load().unwrap().revision(), 1);
    // A later successful persist has a durable revision; retries use that revision.
    store.save(&journal, &live).unwrap();
    assert_eq!(store.load().unwrap().revision(), 2);
}
