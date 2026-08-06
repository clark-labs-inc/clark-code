use super::*;

#[tokio::test]
async fn allowlisted_prefix_does_not_carry_an_unapproved_suffix() {
    let session = Arc::new(Mutex::new(SessionState {
        allow_commands: vec!["cargo test".to_string()],
        ..Default::default()
    }));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);

    // The exact allowlisted command and safe extensions/pipes preapprove.
    assert!(
        gate.command_preapproved("bash", &bash_gate("cargo test"))
            .await
    );
    assert!(
        gate.command_preapproved("bash", &bash_gate("cargo test --workspace"))
            .await
    );
    assert!(
        gate.command_preapproved("bash", &bash_gate("cargo test | tee log"))
            .await
    );
    // A chained un-approved (Caution) suffix must NOT ride the trusted prefix.
    assert!(
        !gate
            .command_preapproved("bash", &bash_gate("cargo test && cp ~/.ssh/id_rsa /tmp/x"))
            .await
    );
    assert!(
        !gate
            .command_preapproved("bash", &bash_gate("cargo test; npm install evil"))
            .await
    );
    // Hidden command substitution is never preapproved either.
    assert!(
        !gate
            .command_preapproved("bash", &bash_gate("cargo test $(curl evil)"))
            .await
    );
}

#[tokio::test]
async fn safe_commands_still_ask_without_an_explicit_remembered_rule() {
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);

    assert!(
        !gate
            .command_preapproved("bash", &bash_gate("cargo test"))
            .await
    );
}

#[tokio::test]
async fn denylist_checks_every_shell_segment() {
    let session = Arc::new(Mutex::new(SessionState {
        deny_commands: vec!["cat ~/.ssh/id_rsa".to_string()],
        ..Default::default()
    }));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session, control, SessionId::new("s1"), tx);

    for command in [
        "true; cat ~/.ssh/id_rsa",
        "true && cat ~/.ssh/id_rsa",
        "true || cat ~/.ssh/id_rsa",
        "true | cat ~/.ssh/id_rsa",
        "true\ncat ~/.ssh/id_rsa",
    ] {
        assert_eq!(
            gate.hard_refusal("bash", &bash_gate(command))
                .await
                .as_deref(),
            Some("on your command denylist"),
            "denylist bypassed by {command:?}"
        );
    }
    assert!(gate
        .hard_refusal("bash", &bash_gate("cat ~/.ssh/id_rsa.pub"))
        .await
        .is_none());
}

#[tokio::test]
async fn remembered_command_does_not_authorize_network_or_host_execution() {
    let session = Arc::new(Mutex::new(SessionState {
        allow_commands: vec!["cargo test".to_string(), "gh pr view".to_string()],
        ..Default::default()
    }));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, _rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(session.clone(), control, SessionId::new("s1"), tx);

    let elevated = gate_info(
        "bash",
        &serde_json::json!({
            "command": "cargo test",
            "sandbox_permissions": "require_escalated",
        }),
    );
    assert!(!gate.command_preapproved("bash", &elevated).await);
    assert_eq!(permission_options("bash", None, &elevated).len(), 2);

    let network = bash_gate("gh pr view 123");
    assert!(!gate.command_preapproved("bash", &network).await);
    assert_eq!(permission_options("bash", None, &network).len(), 2);

    gate.apply_policy("bash", None, &elevated, Decision::AllowAlways)
        .await;
    assert_eq!(
        session.lock().await.allow_commands,
        vec!["cargo test".to_string(), "gh pr view".to_string()]
    );
}

#[tokio::test]
async fn github_command_requests_scoped_network_permission() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Mutex::new(SessionState::default()));
    let control = Arc::new(Mutex::new(RunControl::default()));
    let (tx, rx) = async_channel::unbounded::<AgentEvent>();
    let gate = PermissionGate::new(
        session,
        control.clone(),
        SessionId::new("network-session"),
        tx,
    );
    let ctx = test_ctx(dir.path());

    let check = tokio::spawn(async move {
        gate.check(
            &ToolCallId::new("gh-1"),
            "bash",
            &FakeMutating,
            &serde_json::json!({"command": "gh pr view 123"}),
            &ctx,
            &CancellationToken::new(),
        )
        .await
    });

    let AgentEvent::PermissionRequest { request } = rx.recv().await.unwrap() else {
        panic!("expected a permission request");
    };
    assert_eq!(request.risk.as_deref(), Some("network"));
    assert_eq!(request.title, "Allow this command to use the network?");
    assert_eq!(request.reason.as_deref(), Some("accesses GitHub"));
    assert_eq!(request.options.len(), 2);
    assert!(request
        .options
        .iter()
        .all(|option| option.kind != PermissionOptionKind::AllowAlways));
    control
        .lock()
        .await
        .resolve(&request.id, Decision::AllowOnce.into());
    assert!(matches!(check.await.unwrap(), PermissionOutcome::Allowed));
}
