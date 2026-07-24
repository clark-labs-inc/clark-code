use super::*;

#[test]
fn prefix_matching_keeps_word_boundary() {
    assert!(prefix_match("cargo test --lib", "cargo test"));
    assert!(prefix_match("cargo test", "cargo test"));
    assert!(!prefix_match("cargo testfoo", "cargo test"));
}

#[test]
fn tool_titles_are_user_facing() {
    assert_eq!(
        permission_title("bash", None, &bash_gate("cargo test")),
        "Run a shell command?"
    );
    assert_eq!(
        permission_title("bash", None, &bash_gate("gh pr view 123")),
        "Allow this command to use the network?"
    );
    assert_eq!(
        permission_title(
            "bash",
            None,
            &gate_info(
                "bash",
                &serde_json::json!({
                    "command": "tool-with-custom-host-access",
                    "sandbox_permissions": "require_escalated",
                }),
            ),
        ),
        "Run this command outside the project sandbox?"
    );
    assert_eq!(
        permission_title("write_file", None, &gate_info("write_file", &Value::Null)),
        "Write this file?"
    );
    assert_eq!(
        permission_title(
            "enter_plan_mode",
            None,
            &gate_info("enter_plan_mode", &Value::Null)
        ),
        "Start with a plan?"
    );
    assert_eq!(
        permission_title(
            "generate_image",
            None,
            &gate_info("generate_image", &Value::Null),
        ),
        "Generate an image?"
    );
    assert_eq!(
        permission_title("browser", None, &gate_info("browser", &Value::Null)),
        "Run this browser action?"
    );
    assert_eq!(
        permission_title(
            "mcp_github_create_issue",
            None,
            &gate_info("mcp_github_create_issue", &Value::Null)
        ),
        "Run this connected action?"
    );
    assert_eq!(
        permission_title(
            "future_internal_tool",
            None,
            &gate_info("future_internal_tool", &Value::Null)
        ),
        "Allow this action to run?"
    );
    assert_eq!(
        permission_options(
            "future_internal_tool",
            None,
            &gate_info("future_internal_tool", &Value::Null)
        )[1]
        .label,
        "Always allow similar actions"
    );
}

#[test]
fn image_generation_requires_external_review_without_copying_reference_bytes() {
    let info = gate_info(
        "generate_image",
        &serde_json::json!({
            "prompt": "A small mossy cabin beside a lake",
            "input_images": ["data:image/png;base64,very-large-image"],
            "output_path": "images/cabin.png",
        }),
    );
    assert!(info.external);
    let detail = info.detail.expect("generation detail");
    assert!(detail.contains("mossy cabin"));
    assert!(detail.contains("images/cabin.png"));
    assert!(detail.contains("extension matches returned image"));
    assert!(!detail.contains("very-large-image"));
}

#[tokio::test]
async fn image_generation_uses_the_typed_billed_permission_risk() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control.clone(), SessionId::new("s1"), tx);
    let ctx = test_ctx(dir.path());

    let check = tokio::spawn(async move {
        gate.check(
            &ToolCallId::new("image-1"),
            "generate_image",
            &FakeMutating,
            &serde_json::json!({"prompt": "a small yellow house"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await
    });

    let event = rx.recv().await.unwrap();
    let AgentEvent::PermissionRequest { request } = event else {
        panic!("expected a permission request");
    };
    assert_eq!(request.risk.as_deref(), Some("billed"));
    control
        .lock()
        .await
        .resolve(&request.id, Decision::AllowOnce.into());
    assert!(matches!(check.await.unwrap(), PermissionOutcome::Allowed));
}

#[test]
fn browser_gate_info_is_marked_external_like_mcp_tools() {
    let info = gate_info(
        "browser",
        &serde_json::json!({"action": "navigate", "url": "https://x"}),
    );
    assert!(info.external);
    assert!(info.detail.unwrap().contains("navigate"));
}

#[test]
fn connected_service_permission_does_not_expose_its_internal_name() {
    let info = gate_info(
        "mcp_github_create_issue",
        &serde_json::json!({"title": "Bug report"}),
    );

    assert!(info.external);
    assert!(info.detail.as_deref().unwrap().contains("Bug report"));
    assert!(!info.detail.as_deref().unwrap().contains("mcp_github"));
    assert_eq!(
        info.reason.as_deref(),
        Some("connected service action - review its inputs")
    );
}

struct FakeMutating;

#[async_trait::async_trait]
impl ToolExecutor for FakeMutating {
    fn name(&self) -> &str {
        "fake_mutate"
    }
    fn description(&self) -> &str {
        "test tool"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    fn kind(&self) -> agent_core::domain::ToolKind {
        agent_core::domain::ToolKind::Other
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolCtx) -> crate::tools::ToolOutcome {
        crate::tools::ToolOutcome::ok("done")
    }
}

struct FakeClarkCloud;

#[async_trait::async_trait]
impl ToolExecutor for FakeClarkCloud {
    fn name(&self) -> &str {
        "clark_research"
    }
    fn description(&self) -> &str {
        "test cloud tool"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    fn kind(&self) -> agent_core::domain::ToolKind {
        agent_core::domain::ToolKind::Research
    }
    fn permission_class(&self) -> crate::tools::ToolPermissionClass {
        crate::tools::ToolPermissionClass::BrokeredClarkCloud
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolCtx) -> crate::tools::ToolOutcome {
        crate::tools::ToolOutcome::ok("done")
    }
}

struct FakeExternal;

#[async_trait::async_trait]
impl ToolExecutor for FakeExternal {
    fn name(&self) -> &str {
        "fake_external"
    }
    fn description(&self) -> &str {
        "test external tool"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    fn kind(&self) -> agent_core::domain::ToolKind {
        agent_core::domain::ToolKind::Fetch
    }
    fn permission_class(&self) -> crate::tools::ToolPermissionClass {
        crate::tools::ToolPermissionClass::External
    }
    async fn invoke(&self, _args: Value, _ctx: &ToolCtx) -> crate::tools::ToolOutcome {
        crate::tools::ToolOutcome::ok("done")
    }
}

fn test_ctx(dir: &std::path::Path) -> ToolCtx {
    ToolCtx {
        sandbox: Arc::new(crate::sandbox::Sandbox::new(dir).unwrap()),
        executor: Arc::new(crate::exec::LocalExecutor),
        reads: Arc::new(std::sync::Mutex::new(crate::tools::ReadTracker::default())),
        cancel: CancellationToken::new(),
        background: Arc::new(crate::background::BackgroundTasks::default()),
        session: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
        progress: None,
        agent_progress: None,
        call_progress: None,
    }
}

fn plan_session_state() -> SessionState {
    let mut state = SessionState::default();
    state
        .planning
        .set_mode(agent_core::provider::CollaborationMode::Plan);
    state
}

#[tokio::test]
async fn brokered_clark_cloud_is_default_allowed_without_opening_local_network() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);
    let outcome = gate
        .check(
            &ToolCallId::new("t1"),
            "clark_research",
            &FakeClarkCloud,
            &serde_json::json!({"query": "current docs"}),
            &test_ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(outcome, PermissionOutcome::Allowed));
    assert!(rx.is_empty());
}

#[tokio::test]
async fn parallel_external_permissions_are_presented_and_resolved_without_loss() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(
        session,
        control.clone(),
        SessionId::new("parallel-session"),
        tx,
    );
    let ctx = test_ctx(dir.path());

    let first_gate = gate.clone();
    let first_ctx = ctx.clone();
    let first = tokio::spawn(async move {
        first_gate
            .check(
                &ToolCallId::new("web_fetch_0"),
                "web_fetch",
                &FakeExternal,
                &serde_json::json!({"url": "https://one.example"}),
                &first_ctx,
                &CancellationToken::new(),
            )
            .await
    });
    let second = tokio::spawn(async move {
        gate.check(
            &ToolCallId::new("web_fetch_1"),
            "web_fetch",
            &FakeExternal,
            &serde_json::json!({"url": "https://two.example"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await
    });

    let AgentEvent::PermissionRequest {
        request: first_request,
    } = rx.recv().await.expect("first permission request")
    else {
        panic!("expected a permission request");
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), rx.recv())
            .await
            .is_err(),
        "the second permission must queue instead of replacing the first"
    );
    assert!(control
        .lock()
        .await
        .resolve(&first_request.id, Decision::AllowOnce.into()));

    let AgentEvent::PermissionRequest {
        request: second_request,
    } = rx.recv().await.expect("second permission request")
    else {
        panic!("expected a permission request");
    };
    assert_ne!(first_request.id, second_request.id);
    assert!(control
        .lock()
        .await
        .resolve(&second_request.id, Decision::AllowOnce.into()));

    assert!(matches!(first.await.unwrap(), PermissionOutcome::Allowed));
    assert!(matches!(second.await.unwrap(), PermissionOutcome::Allowed));
}

#[tokio::test]
async fn allow_always_applies_to_parallel_waiters_before_they_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(
        session,
        control.clone(),
        SessionId::new("always-session"),
        tx,
    );
    let ctx = test_ctx(dir.path());
    let mut checks = Vec::new();
    for index in 0..2 {
        let gate = gate.clone();
        let ctx = ctx.clone();
        checks.push(tokio::spawn(async move {
            gate.check(
                &ToolCallId::new(format!("web_fetch_{index}")),
                "web_fetch",
                &FakeExternal,
                &serde_json::json!({"url": format!("https://{index}.example")}),
                &ctx,
                &CancellationToken::new(),
            )
            .await
        }));
    }

    let AgentEvent::PermissionRequest { request } =
        rx.recv().await.expect("first permission request")
    else {
        panic!("expected a permission request");
    };
    assert!(control
        .lock()
        .await
        .resolve(&request.id, Decision::AllowAlways.into()));
    for check in checks {
        assert!(matches!(check.await.unwrap(), PermissionOutcome::Allowed));
    }
    assert!(
        rx.is_empty(),
        "the queued call must inherit allow-always without another prompt"
    );
}

#[tokio::test]
async fn orphaned_permission_responder_is_an_explicit_failure() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(
        session,
        control.clone(),
        SessionId::new("orphan-session"),
        tx,
    );
    let ctx = test_ctx(dir.path());
    let check = tokio::spawn(async move {
        gate.check(
            &ToolCallId::new("web_fetch_orphan"),
            "web_fetch",
            &FakeExternal,
            &serde_json::json!({"url": "https://example.com"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await
    });

    let AgentEvent::PermissionRequest { .. } = rx.recv().await.expect("permission request") else {
        panic!("expected a permission request");
    };
    control.lock().await.clear();

    let PermissionOutcome::Failed(message) = check.await.unwrap() else {
        panic!("an orphaned permission must not masquerade as user cancellation");
    };
    assert!(message.contains("closed without a decision"));
}

#[tokio::test]
async fn closed_permission_event_channel_is_an_explicit_failure() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    rx.close();
    let gate = PermissionGate::new(
        session,
        control.clone(),
        SessionId::new("closed-events-session"),
        tx,
    );
    let ctx = test_ctx(dir.path());

    let PermissionOutcome::Failed(message) = gate
        .check(
            &ToolCallId::new("web_fetch_closed_events"),
            "web_fetch",
            &FakeExternal,
            &serde_json::json!({"url": "https://example.com"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await
    else {
        panic!("an undeliverable permission must not wait or look cancelled");
    };
    assert!(message.contains("could not be delivered"));
    assert!(!control.lock().await.has_pending());
}

#[tokio::test]
async fn plan_mode_denies_mutating_tools_except_propose_plan() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(plan_session_state()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);
    let ctx = test_ctx(dir.path());
    let outcome = gate
        .check(
            &ToolCallId::new("t1"),
            "fake_mutate",
            &FakeMutating,
            &serde_json::json!({}),
            &ctx,
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(outcome, PermissionOutcome::Denied(_)));
}

#[tokio::test]
async fn document_workspace_writes_are_denied_in_plan_mode() {
    let project = tempfile::tempdir().unwrap();
    let docs = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(plan_session_state()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);
    let mut ctx = test_ctx(project.path());
    ctx.sandbox = Arc::new(
        crate::sandbox::Sandbox::new(project.path())
            .unwrap()
            .with_docs(docs.path().to_path_buf()),
    );
    let target = ctx.sandbox.docs_root().unwrap().join("design.md");

    let outcome = gate
        .check(
            &ToolCallId::new("t1"),
            "write_file",
            &FakeMutating,
            &serde_json::json!({"path": target}),
            &ctx,
            &CancellationToken::new(),
        )
        .await;

    assert!(matches!(outcome, PermissionOutcome::Denied(_)));
    assert!(rx.is_empty(), "plan-mode writes must not prompt");
}

#[tokio::test]
async fn document_workspace_writes_reach_the_ask_mode_gate() {
    let project = tempfile::tempdir().unwrap();
    let docs = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control.clone(), SessionId::new("docs-session"), tx);
    let mut ctx = test_ctx(project.path());
    ctx.sandbox = Arc::new(
        crate::sandbox::Sandbox::new(project.path())
            .unwrap()
            .with_docs(docs.path().to_path_buf()),
    );
    let target = ctx.sandbox.docs_root().unwrap().join("design.md");

    let check = tokio::spawn(async move {
        gate.check(
            &ToolCallId::new("docs-1"),
            "write_file",
            &FakeMutating,
            &serde_json::json!({"path": target}),
            &ctx,
            &CancellationToken::new(),
        )
        .await
    });

    let AgentEvent::PermissionRequest { request } = rx.recv().await.unwrap() else {
        panic!("expected a document write permission request");
    };
    assert_eq!(request.title, "Write this file?");
    control
        .lock()
        .await
        .resolve(&request.id, Decision::AllowOnce.into());
    assert!(matches!(check.await.unwrap(), PermissionOutcome::Allowed));
}

#[tokio::test]
async fn explicit_write_deny_still_applies_to_document_workspace() {
    let project = tempfile::tempdir().unwrap();
    let docs = tempfile::tempdir().unwrap();
    let mut state = SessionState::default();
    state
        .policy
        .insert("write_file".to_string(), PermissionMode::Deny);
    let session = Arc::new(Mutex::new(state));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);
    let mut ctx = test_ctx(project.path());
    ctx.sandbox = Arc::new(
        crate::sandbox::Sandbox::new(project.path())
            .unwrap()
            .with_docs(docs.path().to_path_buf()),
    );
    let target = ctx.sandbox.docs_root().unwrap().join("design.md");

    let outcome = gate
        .check(
            &ToolCallId::new("t1"),
            "write_file",
            &FakeMutating,
            &serde_json::json!({"path": target}),
            &ctx,
            &CancellationToken::new(),
        )
        .await;

    assert!(matches!(outcome, PermissionOutcome::Denied(_)));
}

fn bash_gate(cmd: &str) -> GateInfo {
    gate_info("bash", &serde_json::json!({ "command": cmd }))
}

#[allow(clippy::type_complexity)] // test fixture tuple, destructured at every call site
fn plan_mode_gate(
    state: SessionState,
) -> (
    PermissionGate,
    Arc<Mutex<SessionState>>,
    Arc<Mutex<RunControl>>,
    async_channel::Receiver<AgentEvent>,
) {
    let session = Arc::new(Mutex::new(state));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session.clone(), control.clone(), SessionId::new("s1"), tx);
    (gate, session, control, rx)
}

#[tokio::test]
async fn plan_mode_allows_readonly_bash_and_denies_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let (gate, _session, _control, rx) = plan_mode_gate(plan_session_state());
    let ctx = test_ctx(dir.path());

    let readonly = gate
        .check(
            &ToolCallId::new("t1"),
            "bash",
            &FakeMutating,
            &serde_json::json!({"command": "git status && rg propose_plan | head"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(readonly, PermissionOutcome::Allowed));
    assert!(rx.is_empty(), "read-only research must not prompt");

    let mutating = gate
        .check(
            &ToolCallId::new("t2"),
            "bash",
            &FakeMutating,
            &serde_json::json!({"command": "touch src/new.rs"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await;
    let PermissionOutcome::Denied(message) = mutating else {
        panic!("a mutating command must be denied in plan mode");
    };
    assert!(message.contains("Plan mode is active"));
    assert!(message.contains("propose_plan"));

    let network = gate
        .check(
            &ToolCallId::new("t3"),
            "bash",
            &FakeMutating,
            &serde_json::json!({"command": "gh pr view 123"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await;
    let PermissionOutcome::Denied(message) = network else {
        panic!("network access must be denied in plan mode");
    };
    assert!(message.contains("network and host access"));
    assert!(rx.is_empty(), "plan-mode network access must not prompt");
}

#[tokio::test]
async fn plan_mode_keeps_the_hard_floor_for_bash() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = plan_session_state();
    state.deny_commands = vec!["git log".to_string()];
    let (gate, _session, _control, _rx) = plan_mode_gate(state);
    let ctx = test_ctx(dir.path());

    // Read-only but user-denylisted → still refused.
    let denylisted = gate
        .check(
            &ToolCallId::new("t1"),
            "bash",
            &FakeMutating,
            &serde_json::json!({"command": "git log"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await;
    let PermissionOutcome::Denied(message) = denylisted else {
        panic!("denylisted command must be refused");
    };
    assert!(message.contains("denylist"));
}

#[tokio::test]
async fn enter_plan_mode_asks_then_flips_plan_mode_on_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (gate, session, control, rx) = plan_mode_gate(SessionState::default());
    let ctx = test_ctx(dir.path());

    let check = tokio::spawn(async move {
        gate.check(
            &ToolCallId::new("t1"),
            "enter_plan_mode",
            &FakeMutating,
            &serde_json::json!({"reason": "touches several files"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await
    });

    let event = rx.recv().await.unwrap();
    let AgentEvent::PermissionRequest { request } = event else {
        panic!("expected a permission request");
    };
    assert_eq!(request.risk.as_deref(), Some("plan_entry"));
    assert_eq!(request.title, "Start with a plan?");
    assert_eq!(request.detail.as_deref(), Some("touches several files"));
    assert_eq!(
        request
            .options
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>(),
        vec!["allow_once", "reject_once"]
    );
    control
        .lock()
        .await
        .resolve(&request.id, Decision::AllowOnce.into());

    let outcome = check.await.unwrap();
    assert!(matches!(outcome, PermissionOutcome::Allowed));
    let s = session.lock().await;
    assert!(s.planning.plan_mode());
    assert!(!s.planning.exited);
}

#[tokio::test]
async fn enter_plan_mode_rejection_tells_the_model_to_proceed() {
    let dir = tempfile::tempdir().unwrap();
    let (gate, session, control, rx) = plan_mode_gate(SessionState::default());
    let ctx = test_ctx(dir.path());

    let check = tokio::spawn(async move {
        gate.check(
            &ToolCallId::new("t1"),
            "enter_plan_mode",
            &FakeMutating,
            &serde_json::json!({}),
            &ctx,
            &CancellationToken::new(),
        )
        .await
    });

    let event = rx.recv().await.unwrap();
    let AgentEvent::PermissionRequest { request } = event else {
        panic!("expected a permission request");
    };
    control
        .lock()
        .await
        .resolve(&request.id, Decision::RejectOnce.into());

    let outcome = check.await.unwrap();
    let PermissionOutcome::Denied(message) = outcome else {
        panic!("expected a denial");
    };
    assert!(message.contains("proceed directly"));
    assert!(!session.lock().await.planning.plan_mode());
}

#[tokio::test]
async fn enter_plan_mode_is_denied_when_already_planning() {
    let dir = tempfile::tempdir().unwrap();
    let (gate, _session, _control, rx) = plan_mode_gate(plan_session_state());
    let ctx = test_ctx(dir.path());

    let outcome = gate
        .check(
            &ToolCallId::new("t1"),
            "enter_plan_mode",
            &FakeMutating,
            &serde_json::json!({}),
            &ctx,
            &CancellationToken::new(),
        )
        .await;

    let PermissionOutcome::Denied(message) = outcome else {
        panic!("expected a denial");
    };
    assert!(message.contains("already active"));
    assert!(rx.is_empty(), "no prompt for a redundant request");
}

#[path = "permissions_tests/command_scope.rs"]
mod command_scope;
