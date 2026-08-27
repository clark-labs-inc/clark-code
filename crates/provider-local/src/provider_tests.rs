use super::*;
use agent_core::domain::PendingUpload;

fn provider_test_config() -> ProviderConfig {
    ProviderConfig {
        // Session/prompt unit tests do not execute untrusted model-authored
        // tools. Keep them independent of host sandbox availability; sandbox
        // selection and fail-closed behavior have dedicated contract tests.
        extra: serde_json::json!({"sandbox_mode": "disabled"}),
        ..Default::default()
    }
}

fn provider_test_config_with_extra(mut extra: serde_json::Value) -> ProviderConfig {
    extra
        .as_object_mut()
        .expect("test provider extras must be an object")
        .insert(
            "sandbox_mode".to_string(),
            serde_json::Value::String("disabled".to_string()),
        );
    ProviderConfig {
        extra,
        ..Default::default()
    }
}

#[tokio::test]
async fn scout_sessions_cannot_be_downgraded_from_full_access() {
    let identity = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(provider_test_config_with_extra(serde_json::json!({
            "scout_cartography": {
                "organization_id": uuid::Uuid::new_v4(),
                "workspace_id": uuid::Uuid::new_v4(),
                "identity_root": identity.path(),
                "platform": "macos",
                "architecture": "aarch64",
                "route_prefix": "/v1/cartography"
            }
        })))
        .await
        .unwrap();

    let session = provider
        .new_session(SessionOptions {
            cwd: Some(identity.path().to_string_lossy().into_owned()),
            mode: Some("ask".into()),
            collaboration_mode: Some(CollaborationMode::Plan),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(session.mode.as_deref(), Some("full"));
    assert_eq!(session.collaboration_mode, CollaborationMode::Default);

    provider.set_mode(&session.id, "auto".into()).await.unwrap();
    assert_eq!(provider.session_mode.as_deref(), Some("full"));

    provider
        .set_collaboration_mode(&session.id, CollaborationMode::Plan)
        .await
        .unwrap();
    assert!(!provider.session.lock().await.planning.plan_mode());
}

#[test]
fn prompt_text_joins_blocks() {
    let input = PromptInput {
        blocks: vec![ContentBlock::text("hello "), ContentBlock::text("world")],
        attachments: Vec::new(),
    };
    assert_eq!(prompt_text(&input), "hello world");
}

#[test]
fn goal_command_requires_an_exact_prefix_and_preserves_the_objective() {
    assert_eq!(
        goal_command_objective("  /goal investigate and fix the composer"),
        Some("investigate and fix the composer".into())
    );
    assert_eq!(goal_command_objective("/goal"), Some(String::new()));
    assert_eq!(goal_command_objective("/goals list"), None);
    assert_eq!(goal_command_objective("please /goal later"), None);
}

#[test]
fn explicit_goal_language_preactivates_without_inference_from_ordinary_work() {
    assert!(explicitly_requests_goal_lifecycle(
        "Create a goal with create_goal and keep working until it is complete."
    ));
    assert!(explicitly_requests_goal_lifecycle(
        "Please start a goal for this migration."
    ));
    assert!(!explicitly_requests_goal_lifecycle(
        "Fix the migration and run its tests."
    ));
}

#[test]
fn prompt_text_inlines_text_attachment() {
    let input = PromptInput {
        blocks: vec![ContentBlock::text("see file")],
        attachments: vec![PendingUpload {
            filename: "note.txt".into(),
            content_type: "text/plain".into(),
            data_base64: "aGVsbG8=".into(), // "hello"
        }],
    };
    let text = prompt_text(&input);
    assert!(text.contains("see file"));
    assert!(text.contains("attached text file: note.txt"));
    assert!(text.contains("hello"));
}

#[test]
fn turn_prompt_keeps_context_and_attachments_before_the_user_request() {
    let input = PromptInput {
        blocks: vec![ContentBlock::text("implement the feature")],
        attachments: vec![PendingUpload {
            filename: "notes.txt".into(),
            content_type: "text/plain".into(),
            data_base64: "ZXh0cmEgY29udGV4dA==".into(),
        }],
    };
    let parts = prompt_parts(&input);
    let wire = assemble_turn_prompt(
        &["[runtime policy]".into(), parts.text_attachment_context],
        &parts.user_request,
    );
    assert!(wire.find("[runtime policy]").unwrap() < wire.find("notes.txt").unwrap());
    assert!(wire.find("notes.txt").unwrap() < wire.find("# User request").unwrap());
    assert!(wire.ends_with("implement the feature"));
}

#[test]
fn native_image_content_is_forwarded_when_policy_allows_it() {
    let attachments = vec![PendingUpload {
        filename: "design.png".into(),
        content_type: "image/png".into(),
        data_base64: "QUJD".into(),
    }];

    let content = model_user_content("review the design".into(), &attachments, true);
    let agent_loop::UserContent::Blocks(blocks) = content else {
        panic!("expected multimodal user content");
    };
    assert!(matches!(
        &blocks[0],
        agent_loop::UserBlock::Image(image)
            if image.source == "data:image/png;base64,QUJD"
                && image.alt.as_deref() == Some("design.png")
    ));
    assert!(matches!(
        &blocks[1],
        agent_loop::UserBlock::Text(text) if text.text == "review the design"
    ));
}

#[test]
fn non_vision_models_keep_plain_text_user_content() {
    let attachments = vec![PendingUpload {
        filename: "design.png".into(),
        content_type: "image/png".into(),
        data_base64: "QUJD".into(),
    }];

    assert!(matches!(
        model_user_content("fallback description".into(), &attachments, false),
        agent_loop::UserContent::Text(text) if text == "fallback description"
    ));
}

#[test]
fn prompt_text_does_not_note_non_text_attachments() {
    // A non-text attachment (e.g. an image) must never get a bare
    // filename note here — that's exactly what previously sent the model
    // hunting the filesystem for a file that only existed as inline
    // base64. Non-text handling now lives in `crate::attachments`.
    let input = PromptInput {
        blocks: vec![ContentBlock::text("look at this")],
        attachments: vec![PendingUpload {
            filename: "image.webp".into(),
            content_type: "image/webp".into(),
            data_base64: "aGVsbG8=".into(),
        }],
    };
    let text = prompt_text(&input);
    assert!(!text.contains("attached file:"));
    assert!(!text.contains("image.webp"));
}

#[test]
fn base64_decodes_text() {
    assert_eq!(
        decode_base64_text("aGVsbG8gd29ybGQ=").unwrap(),
        "hello world"
    );
}

#[test]
fn cancellation_registry_targets_the_requested_run() {
    let registry = RunCancellationRegistry::default();
    let first = CancellationToken::new();
    let second = CancellationToken::new();
    registry.register(&RunId::new("run-1"), first.clone());
    registry.register(&RunId::new("run-2"), second.clone());

    assert!(registry.cancel(&RunId::new("run-1")));
    assert!(first.is_cancelled());
    assert!(!second.is_cancelled());
    assert!(!registry.cancel(&RunId::new("missing")));
}

#[tokio::test]
async fn cancel_reports_an_inactive_run_with_a_typed_error() {
    let mut provider = LocalAgentProvider::new();
    let error = provider
        .cancel(&SessionId::new("session-1"), &RunId::new("run-finished"))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        Error::RunNotActive(run) if run == RunId::new("run-finished")
    ));
}

#[tokio::test]
async fn a_new_prompt_supersedes_a_run_parked_on_a_permission_request() {
    use agent_core::ids::PermissionRequestId;
    use tokio::sync::oneshot::error::TryRecvError;
    let mut provider = LocalAgentProvider::new();

    // A previous turn was abandoned mid-run (e.g. its remote host timed out
    // while it waited for this permission answer): its run task is still
    // registered and it still holds the session's single armed request.
    let parked = CancellationToken::new();
    provider
        .run_cancellations
        .register(&RunId::new("run-1"), parked.clone());
    let (stale_responder, mut stale_response) = tokio::sync::oneshot::channel();
    provider
        .control
        .lock()
        .await
        .arm(PermissionRequestId::new("perm-call_stale"), stale_responder)
        .expect("stale request arms");

    assert!(provider.supersede_parked_runs().await);

    // The parked run is cancelled and its armed request dropped, so the
    // parked waiter's response channel closes: it ends as Cancelled instead
    // of leaking while blocking every later turn's permission prompt.
    assert!(parked.is_cancelled());
    assert!(matches!(
        stale_response.try_recv(),
        Err(TryRecvError::Closed)
    ));

    // The next run can arm its own request. Before the fix this `arm` failed
    // ("request ... was still pending when ... tried to start") and escalated
    // to a fatal `tool_fatal` run failure.
    let (responder, _response) = tokio::sync::oneshot::channel();
    provider
        .control
        .lock()
        .await
        .arm(PermissionRequestId::new("perm-call_next"), responder)
        .expect("next run arms after supersede");

    // A session with nothing parked reports no supersession.
    let mut fresh = LocalAgentProvider::new();
    assert!(!fresh.supersede_parked_runs().await);
}

#[tokio::test]
async fn project_settings_extend_the_session_sandbox_write_roots() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let absolute_root = cache.path().canonicalize().unwrap();
    std::fs::create_dir_all(project.path().join(".agent")).unwrap();
    std::fs::write(
        project.path().join(".agent/settings.json"),
        serde_json::json!({
            "sandbox_write_roots": [absolute_root.to_string_lossy(), "relative/escape"]
        })
        .to_string(),
    )
    .unwrap();

    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(project.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    // The absolute shared-cache root reaches the session config (and from there
    // the WorkspaceWrite sandbox policy); the relative entry is refused instead
    // of silently widened.
    assert_eq!(
        provider.config().unwrap().sandbox_write_roots,
        vec![absolute_root.clone()]
    );

    // Executor rebuilds on later mode switches reuse the same merged inputs.
    provider
        .set_collaboration_mode(&session.id, CollaborationMode::Plan)
        .await
        .unwrap();
    assert_eq!(
        provider.config().unwrap().sandbox_write_roots,
        vec![absolute_root]
    );
}

#[tokio::test]
async fn full_access_lifts_file_tool_containment_and_auto_restores_it() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    // Unlike `provider_test_config` (sandbox_mode disabled → always host
    // trusted), this fixture exercises the contained presets themselves.
    let mut config = provider_test_config();
    config
        .extra
        .as_object_mut()
        .unwrap()
        .insert("sandbox_mode".to_string(), serde_json::json!("auto"));
    provider.connect(config).await.unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(project.path().to_string_lossy().into_owned()),
            mode: Some("ask".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // ask/auto: contained.
    assert!(!provider.sandbox.as_ref().unwrap().host_trusted());

    // Full Access: file-tool containment lifts with the process sandbox.
    provider.set_mode(&session.id, "full".into()).await.unwrap();
    assert!(provider.sandbox.as_ref().unwrap().host_trusted());
    let trusted = provider.sandbox.as_ref().unwrap();
    assert!(trusted
        .resolve_for_write(outside.path().join("x.txt").to_str().unwrap())
        .is_ok());

    // Back to auto: walls return without restarting the session.
    provider.set_mode(&session.id, "auto".into()).await.unwrap();
    let contained = provider.sandbox.as_ref().unwrap();
    assert!(!contained.host_trusted());
    assert!(contained
        .resolve_for_write(outside.path().join("x.txt").to_str().unwrap())
        .is_err());
}

#[tokio::test]
async fn new_session_requires_cwd() {
    let mut p = LocalAgentProvider::new();
    p.connect(ProviderConfig::default()).await.unwrap();
    let err = p.new_session(SessionOptions::default()).await.unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
}

#[tokio::test]
async fn live_session_can_admit_and_revoke_an_explicit_read_only_repository() {
    let workspace = tempfile::tempdir().unwrap();
    let repository = tempfile::tempdir().unwrap();
    std::fs::write(repository.path().join("README.md"), "code evidence").unwrap();
    let mut provider = LocalAgentProvider::new();
    // Unlike `provider_test_config` (sandbox_mode disabled → always host
    // trusted), this fixture exercises the contained presets so read-root
    // admission and revocation actually gate file-tool reads.
    let mut config = provider_test_config();
    config
        .extra
        .as_object_mut()
        .unwrap()
        .insert("sandbox_mode".to_string(), serde_json::json!("auto"));
    provider.connect(config).await.unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(workspace.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let readme = repository.path().join("README.md");
    assert!(provider
        .sandbox
        .as_ref()
        .unwrap()
        .resolve_existing(readme.to_string_lossy().as_ref())
        .is_err());

    provider
        .add_read_roots(
            &session.id,
            vec![repository.path().to_string_lossy().into_owned()],
        )
        .await
        .unwrap();

    assert!(provider
        .sandbox
        .as_ref()
        .unwrap()
        .resolve_existing(readme.to_string_lossy().as_ref())
        .is_ok());
    assert!(provider
        .sandbox
        .as_ref()
        .unwrap()
        .resolve_for_write(readme.to_string_lossy().as_ref())
        .is_err());

    provider
        .remove_read_roots(
            &session.id,
            vec![repository.path().to_string_lossy().into_owned()],
        )
        .await
        .unwrap();
    assert!(provider
        .sandbox
        .as_ref()
        .unwrap()
        .resolve_existing(readme.to_string_lossy().as_ref())
        .is_err());
}

#[tokio::test]
async fn isolated_orchestration_session_has_no_ambient_writable_surfaces() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agent")).unwrap();
    std::fs::write(
        dir.path().join(".agent/settings.json"),
        r#"{
          "hooks":{"PreToolUse":[{"matcher":"*","command":"touch /tmp/should-not-run"}]},
          "permissions":{"allow":["bash"]},
          "check_command":"touch /tmp/should-not-run"
        }"#,
    )
    .unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(provider_test_config_with_extra(serde_json::json!({
            "isolated_writer": true,
            "memories": false,
            "browser_enabled": false,
            "mcp_servers": [],
            "permissions": {
                "write_file": "allow",
                "edit_file": "allow",
                "apply_patch": "allow",
                "bash": "deny"
            }
        })))
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            mode: Some("auto".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let environment = session.environment.unwrap();
    assert!(environment.docs_root.is_none());
    assert_eq!(environment.workspace_roots.len(), 1);
    let state = provider.session.lock().await;
    assert!(state.hooks.is_empty());
    assert!(state.allow_commands.is_empty());
    assert!(state.check_command.is_none());
    drop(state);
    let registry = provider.registry.as_ref().unwrap();
    assert!(registry.get("memory").is_none());
    assert!(registry.get("organization_knowledge").is_none());
    assert!(registry.get("browser").is_none());
    assert!(registry.get("delegate_read_only").is_none());
    assert!(registry.get("delegate_coding_workstreams").is_none());
    assert!(registry.get("read_skill").is_none());
}

#[tokio::test]
async fn set_collaboration_mode_flips_plan_mode_flag() {
    let mut p = LocalAgentProvider::new();
    let session_id = SessionId::new("s1");
    {
        let mut state = p.session.lock().await;
        assert!(!state.planning.plan_mode());
        crate::tools::goal::start_goal(&mut state, "finish the migration".into()).unwrap();
    }

    p.set_collaboration_mode(&session_id, CollaborationMode::Plan)
        .await
        .unwrap();
    assert!(p.session.lock().await.planning.plan_mode());

    p.set_collaboration_mode(&session_id, CollaborationMode::Default)
        .await
        .unwrap();
    assert!(!p.session.lock().await.planning.plan_mode());
    assert_eq!(
        p.session.lock().await.goal.as_ref().unwrap().objective,
        "finish the migration",
        "Goal Mode remains an orthogonal lifecycle"
    );
}

#[tokio::test]
async fn strict_toolless_session_uses_the_host_owned_cache_identity() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(provider_test_config_with_extra(serde_json::json!({
            "tools_enabled": false,
            "response_format": {"type": "json_object"},
            "cache_session_id": "product-specialist-cache-1",
            "memories": false,
        })))
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(session.id.as_str(), "product-specialist-cache-1");
}

#[tokio::test]
async fn set_output_style_persists_on_session_state() {
    let mut p = LocalAgentProvider::new();
    let session_id = SessionId::new("s1");
    assert_eq!(p.session.lock().await.output_style, "");

    p.set_output_style(&session_id, "terse".to_string())
        .await
        .unwrap();
    assert_eq!(p.session.lock().await.output_style, "terse");
}

#[tokio::test]
async fn live_configuration_uses_only_provider_advertised_choices() {
    let directory = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(provider_test_config_with_extra(serde_json::json!({
            "model": "managed-model-standard",
            "models": [
                {
                    "id": "managed-model-standard",
                    "label": "Standard",
                    "description": "Standard managed coding model",
                    "reasoning_effort": "high"
                },
                {
                    "id": "managed-model-large",
                    "label": "Large",
                    "description": "Larger managed coding model",
                    "reasoning_effort": "xhigh"
                }
            ],
            "memories": false,
            "browser_enabled": false,
            "browser_binary": {
                "version": "1.0.0",
                "release_tag": "browser-v1",
                "download_base_url": "https://downloads.example/browser",
                "archive_prefix": "managed-browser",
                "cache_namespace": ".agent-desktop"
            }
        })))
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    let initial = provider.configuration(&session.id).await.unwrap();
    assert_eq!(initial.models.len(), 2);
    assert_eq!(initial.model.as_deref(), Some("managed-model-standard"));
    let changed = provider
        .configure(
            &session.id,
            ProviderConfigurationChange::Model {
                model: "managed-model-large".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.model.as_deref(), Some("managed-model-large"));
    assert_eq!(changed.reasoning_effort.as_deref(), Some("xhigh"));
    assert!(provider
        .configure(
            &session.id,
            ProviderConfigurationChange::Model {
                model: "invented".into(),
            },
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("does not advertise"));
}

#[tokio::test]
async fn memory_and_browser_toggles_change_the_active_tool_contract() {
    let directory = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(provider_test_config_with_extra(serde_json::json!({
            "memories": false,
            "browser_enabled": false,
            "browser_binary": {
                "version": "1.0.0",
                "release_tag": "browser-v1",
                "download_base_url": "https://downloads.example/browser",
                "archive_prefix": "managed-browser",
                "cache_namespace": ".agent-desktop"
            }
        })))
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!provider.tool_names().iter().any(|name| name == "memory"));
    assert!(!provider.tool_names().iter().any(|name| name == "browser"));

    provider
        .configure(
            &session.id,
            ProviderConfigurationChange::Memories { enabled: true },
        )
        .await
        .unwrap();
    provider
        .configure(
            &session.id,
            ProviderConfigurationChange::Experiment {
                id: "browser".into(),
                enabled: true,
            },
        )
        .await
        .unwrap();
    let names = provider.tool_names();
    assert!(names.iter().any(|name| name == "memory"));
    assert!(names.iter().any(|name| name == "memory_recall"));
    assert!(names.iter().any(|name| name == "browser"));
    assert!(provider
        .session
        .lock()
        .await
        .system_prompt
        .contains("# Memory"));

    provider
        .configure(
            &session.id,
            ProviderConfigurationChange::Memories { enabled: false },
        )
        .await
        .unwrap();
    provider
        .configure(
            &session.id,
            ProviderConfigurationChange::Experiment {
                id: "browser".into(),
                enabled: false,
            },
        )
        .await
        .unwrap();
    let names = provider.tool_names();
    assert!(!names.iter().any(|name| name == "memory"));
    assert!(!names.iter().any(|name| name == "memory_recall"));
    assert!(!names.iter().any(|name| name == "browser"));
    assert!(!provider
        .session
        .lock()
        .await
        .system_prompt
        .contains("# Memory"));
}

#[tokio::test]
async fn provider_lists_stops_and_cleans_exact_background_task_ids() {
    let directory = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    let id = provider
        .background
        .spawn(
            Arc::new(LocalExecutor),
            command.to_string(),
            directory.path(),
        )
        .await
        .unwrap();
    let listed = provider.background_tasks(&session.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].command, command);
    assert_eq!(listed[0].state, agent_core::BackgroundTaskState::Running);

    let stopped = provider
        .stop_background_task(&session.id, &id)
        .await
        .unwrap();
    assert_eq!(stopped.id, id);
    assert_eq!(stopped.state, agent_core::BackgroundTaskState::Stopping);
    for _ in 0..100 {
        if provider
            .background
            .status(&id)
            .await
            .is_some_and(|status| status.exit_code.is_some())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let cleaned = provider.clean_background_tasks(&session.id).await.unwrap();
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].id, id);
    assert!(provider
        .background_tasks(&session.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn typed_goal_state_resumes_blocked_goals() {
    let directory = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    {
        let mut state = provider.session.lock().await;
        crate::tools::goal::start_goal(
            &mut state,
            "finish Clark Code-owned terminal parity".into(),
        )
        .unwrap();
        let goal = state.goal.as_mut().unwrap();
        goal.status = agent_core::GoalStatus::Blocked;
        goal.tokens_used = 4_000;
        goal.continuations = 3;
        goal.blocker_reason = Some("waiting for user input".into());
        goal.blocker_observations = 3;
    }

    let restored = provider.goal_state(&session.id).await.unwrap().unwrap();
    assert_eq!(restored.status, agent_core::GoalStatus::Blocked);
    assert_eq!(restored.continuations, 3);
    let resumed = provider.resume_goal(&session.id).await.unwrap();
    assert_eq!(resumed.status, agent_core::GoalStatus::Active);
    assert_eq!(resumed.tokens_used, 4_000);
    assert_eq!(resumed.continuations, 3);
    assert!(resumed.blocker_reason.is_none());
}

#[tokio::test]
async fn typed_goal_can_be_cleared_and_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    {
        let mut state = provider.session.lock().await;
        crate::tools::goal::start_goal(&mut state, "ship the migration".into()).unwrap();
    }
    assert!(provider.goal_state(&session.id).await.unwrap().is_some());

    provider.clear_goal(&session.id).await.unwrap();
    assert!(provider.goal_state(&session.id).await.unwrap().is_none());

    // A cleared goal frees the session for a fresh objective immediately.
    {
        let mut state = provider.session.lock().await;
        crate::tools::goal::start_goal(&mut state, "next objective".into()).unwrap();
    }
    assert!(provider.goal_state(&session.id).await.unwrap().is_some());
}

#[tokio::test]
async fn prompt_admission_accepts_same_blocked_goal_and_rejects_conflicting_goal() {
    let directory = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    {
        let mut state = provider.session.lock().await;
        crate::tools::goal::start_goal(&mut state, "finish the migration".into()).unwrap();
        state.goal.as_mut().unwrap().status = agent_core::GoalStatus::Blocked;
    }

    provider
        .validate_prompt(
            &session.id,
            &PromptInput::text("/goal finish the migration"),
        )
        .await
        .unwrap();
    let error = provider
        .validate_prompt(
            &session.id,
            &PromptInput::text("/goal rewrite the renderer"),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("send a follow-up to continue it"));
    assert!(!error.contains("update_goal"));
}

#[test]
fn next_run_registry_forks_while_prior_run_snapshot_is_still_held() {
    let mut provider = LocalAgentProvider::new();
    provider.registry = Some(Arc::new(ToolRegistry::new(None)));
    let prior_run = provider.registry.as_ref().unwrap().clone();

    provider
        .next_run_registry_mut()
        .unwrap()
        .enable_browser(crate::browser_binary::test_browser_config());

    let next_run = provider.registry.as_ref().unwrap();
    assert!(!Arc::ptr_eq(next_run, &prior_run));
    assert!(next_run.get("browser").is_some());
    assert!(prior_run.get("browser").is_none());
}

#[test]
fn run_ids_do_not_collide_across_provider_instances() {
    let first = LocalAgentProvider::new();
    let second = LocalAgentProvider::new();

    let first_run = first.next_run_id();
    let first_follow_up = first.next_run_id();
    let resumed_run = second.next_run_id();

    assert_ne!(first_run, first_follow_up);
    assert_ne!(first_run, resumed_run);
    assert_ne!(first_follow_up, resumed_run);
    assert!(first_run.as_str().starts_with("run-"));
}

#[tokio::test]
async fn close_session_stops_session_owned_background_tasks() {
    let mut provider = LocalAgentProvider::new();
    let dir = tempfile::tempdir().unwrap();
    let task = provider
        .background
        .spawn(Arc::new(LocalExecutor), "sleep 30".to_string(), dir.path())
        .await
        .unwrap();
    provider
        .close_session(&SessionId::new("session"))
        .await
        .unwrap();
    assert!(provider.background.status(&task).await.is_none());
}

#[tokio::test]
async fn new_session_seeds_system_prompt_without_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    p.connect(provider_test_config()).await.unwrap();
    let opts = SessionOptions {
        session_id: None,
        cwd: Some(dir.path().to_string_lossy().to_string()),
        mode: None,
        collaboration_mode: None,
        resume: None,
    };
    let session = p.new_session(opts).await.unwrap();
    assert_eq!(session.provider, ProviderId::new("local"));
    let s = p.session.lock().await;
    assert!(!s.system_prompt.is_empty());
    assert!(s.system_prompt.contains("# Skills"));
    assert!(s.system_prompt.contains("`github:gh-fix-ci`"));
    assert!(s.transcript.is_empty());
    assert!(!s.system_prompt.contains("# Resumed conversation"));
    drop(s);
    assert!(p.registry.as_ref().unwrap().get("read_skill").is_some());
}

#[tokio::test]
async fn host_session_id_is_used_before_document_workspace_creation() {
    let dir = tempfile::tempdir().unwrap();
    let requested = SessionId::new(format!("conversation-{}", uuid::Uuid::new_v4()));
    let expected_workspace = crate::workspace::session_workspace(requested.as_str()).unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();

    let session = provider
        .new_session(SessionOptions {
            session_id: Some(requested.clone()),
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    let expected_workspace_string = expected_workspace.to_string_lossy().into_owned();
    assert_eq!(session.id, requested);
    assert_eq!(
        session
            .environment
            .and_then(|environment| environment.docs_root)
            .as_deref(),
        Some(expected_workspace_string.as_str())
    );
    let _ = std::fs::remove_dir_all(expected_workspace);
}

#[tokio::test]
async fn host_session_id_does_not_replace_the_specialist_cache_identity() {
    let dir = tempfile::tempdir().unwrap();
    let requested = SessionId::new(format!("conversation-{}", uuid::Uuid::new_v4()));
    let expected_workspace = crate::workspace::session_workspace(requested.as_str()).unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(provider_test_config_with_extra(serde_json::json!({
            "tools_enabled": false,
            "response_format": {"type": "json_object"},
            "cache_session_id": "product-specialist-cache-1",
            "memories": false,
        })))
        .await
        .unwrap();

    let session = provider
        .new_session(SessionOptions {
            session_id: Some(requested.clone()),
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(session.id, requested);
    assert_eq!(
        provider
            .llm
            .as_ref()
            .and_then(|llm| llm.session_id_for_test()),
        Some("product-specialist-cache-1")
    );
    let _ = std::fs::remove_dir_all(expected_workspace);
}

#[tokio::test]
async fn tool_free_system_override_has_no_document_or_skill_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(provider_test_config_with_extra(serde_json::json!({
            "tools_enabled": false,
            "memories": false,
            "project_knowledge": false,
            "system_prompt_override": "BOUNDED_SPECIALIST_PROMPT"
        })))
        .await
        .unwrap();
    provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    let state = provider.session.lock().await;
    assert_eq!(state.system_prompt, "BOUNDED_SPECIALIST_PROMPT");
    assert!(!state.system_prompt.contains("# Documents"));
    assert!(!state.system_prompt.contains("# Skills"));
    assert!(provider
        .registry
        .as_ref()
        .unwrap()
        .executors()
        .next()
        .is_none());
}

#[tokio::test]
async fn planning_eval_preactivates_only_registered_deferred_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(provider_test_config_with_extra(serde_json::json!({
            "planning_eval_preactivated_tools": [
                "memory",
                "scout_enterprise_query",
                "not_a_registered_tool"
            ]
        })))
        .await
        .unwrap();
    provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let state = provider.session.lock().await;
    assert!(state.deferred_tools.contains("memory"));
    assert!(state.deferred_tools.contains("scout_enterprise_query"));
    assert!(!state.deferred_tools.contains("not_a_registered_tool"));
}

#[tokio::test]
async fn cached_system_prompt_excludes_mutable_project_instructions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "UNIQUE_PROJECT_RULE").unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();
    provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!provider
        .session
        .lock()
        .await
        .system_prompt
        .contains("UNIQUE_PROJECT_RULE"));
}

#[tokio::test]
async fn project_settings_customize_claude_style_commit_attribution() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agent")).unwrap();
    std::fs::write(
        dir.path().join(".agent/settings.json"),
        r#"{
          "attribution": {
            "commit": "Co-Authored-By: Project Agent <agent@example.com>"
          }
        }"#,
    )
    .unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();
    provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    let prompt = &provider.session.lock().await.system_prompt;
    assert_eq!(
        prompt
            .matches("Co-Authored-By: Project Agent <agent@example.com>")
            .count(),
        1
    );
    assert!(prompt.contains("git commit -F <path>"));
    assert!(!prompt.contains("git commit -m \"$(cat <<'EOF'"));
    let registry = provider.registry.as_ref().unwrap();
    assert!(registry.get("bash").is_some());
    assert!(registry.get("git_commit").is_none());
}

#[tokio::test]
async fn orchestration_tools_are_always_available_to_the_root_agent() {
    let mut enabled = LocalAgentProvider::new();
    enabled.connect(provider_test_config()).await.unwrap();
    let registry = enabled.registry.as_ref().unwrap();
    assert!(registry.get("delegate_read_only").is_some());
    assert!(registry.get("resolve_delegation").is_some());
    assert!(registry.get("delegate_coding_workstreams").is_some());
    assert!(registry.get("resolve_coding_workstreams").is_some());
    assert!(registry.get("scout_capabilities").is_some());
    assert!(registry.get("scout_repository_census").is_some());
    assert!(registry.get("scout_adapter").is_some());
    assert!(registry.get("scout_enterprise").is_some());
    assert!(registry.get("scout_enterprise_query").is_some());
    assert!(registry.get("scout_ledger").is_none());
    assert!(registry.get("scout_probe").is_none());
    assert!(registry.get("scout_measure").is_none());
    let mut legacy_disable_request = LocalAgentProvider::new();
    legacy_disable_request
        .connect(ProviderConfig {
            extra: serde_json::json!({"orchestration": {"enabled": false}}),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(legacy_disable_request
        .registry
        .as_ref()
        .unwrap()
        .get("delegate_read_only")
        .is_some());
    assert!(legacy_disable_request
        .registry
        .as_ref()
        .unwrap()
        .get("delegate_coding_workstreams")
        .is_some());
    assert!(legacy_disable_request
        .registry
        .as_ref()
        .unwrap()
        .get("scout_capabilities")
        .is_some());
}

#[tokio::test]
async fn collaboration_mode_option_controls_plan_mode_independently() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    let config = if cfg!(target_os = "macos") {
        // This one test intentionally exercises the real Plan-mode read-only
        // containment transition on macOS.
        ProviderConfig::default()
    } else {
        provider_test_config()
    };
    p.connect(config).await.unwrap();

    let session = p
        .new_session(SessionOptions {
            session_id: None,
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: Some("auto".to_string()),
            collaboration_mode: Some(CollaborationMode::Plan),
            resume: None,
        })
        .await
        .unwrap();
    assert_eq!(session.mode.as_deref(), Some("auto"));
    assert_eq!(session.collaboration_mode, CollaborationMode::Plan);
    assert!(p.session.lock().await.planning.plan_mode());
    #[cfg(target_os = "macos")]
    assert!(p
        .executor
        .write(&dir.path().join("plan-must-not-write.txt"), b"denied")
        .await
        .is_err());

    // A provider instance reused for a fresh session must not inherit the
    // stale flag.
    p.new_session(SessionOptions {
        session_id: None,
        cwd: Some(dir.path().to_string_lossy().to_string()),
        mode: Some("auto".to_string()),
        collaboration_mode: Some(CollaborationMode::Default),
        resume: None,
    })
    .await
    .unwrap();
    assert!(!p.session.lock().await.planning.plan_mode());
    p.executor
        .write(&dir.path().join("auto-can-write.txt"), b"allowed")
        .await
        .unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn full_access_switches_platform_containment_off_and_default_restores_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(ProviderConfig::default()).await.unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            mode: Some("auto".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        provider.executor.containment(),
        exec_core::ExecutionContainment::Managed
    );

    provider
        .set_mode(&session.id, "full".to_string())
        .await
        .unwrap();
    assert_eq!(
        provider.executor.containment(),
        exec_core::ExecutionContainment::Host
    );

    provider
        .set_collaboration_mode(&session.id, CollaborationMode::Plan)
        .await
        .unwrap();
    assert_eq!(
        provider.executor.containment(),
        exec_core::ExecutionContainment::Managed,
        "Plan temporarily restores the read-only sandbox"
    );

    provider
        .set_mode(&session.id, "full".to_string())
        .await
        .unwrap();
    assert_eq!(
        provider.executor.containment(),
        exec_core::ExecutionContainment::Managed,
        "changing approval policy must not widen an active Plan session"
    );

    provider
        .set_collaboration_mode(&session.id, CollaborationMode::Default)
        .await
        .unwrap();
    assert_eq!(
        provider.executor.containment(),
        exec_core::ExecutionContainment::Host,
        "leaving Plan restores the selected Full access executor"
    );

    provider
        .set_mode(&session.id, "auto".to_string())
        .await
        .unwrap();
    assert_eq!(
        provider.executor.containment(),
        exec_core::ExecutionContainment::Managed
    );
}

#[tokio::test]
async fn set_mode_transitions_queue_the_one_shot_exit_note() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    p.connect(provider_test_config()).await.unwrap();
    let session = p
        .new_session(SessionOptions {
            session_id: None,
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: Some("auto".to_string()),
            collaboration_mode: Some(CollaborationMode::Plan),
            resume: None,
        })
        .await
        .unwrap();

    p.set_collaboration_mode(&session.id, CollaborationMode::Default)
        .await
        .unwrap();
    {
        let s = p.session.lock().await;
        assert!(!s.planning.plan_mode());
        assert!(s.planning.exited, "leaving plan mode queues the exit note");
    }

    // Re-entering cancels a queued note (quick toggle must not tell the
    // model it both entered and exited).
    p.set_collaboration_mode(&session.id, CollaborationMode::Plan)
        .await
        .unwrap();
    {
        let s = p.session.lock().await;
        assert!(s.planning.plan_mode());
        assert!(!s.planning.exited);
    }
}

#[tokio::test]
async fn new_session_replays_typed_resume_into_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    p.connect(provider_test_config()).await.unwrap();
    let opts = SessionOptions {
        session_id: None,
        cwd: Some(dir.path().to_string_lossy().to_string()),
        mode: None,
        collaboration_mode: None,
        resume: Some(agent_core::ResumeTranscript {
            truncated: false,
            items: vec![
                agent_core::ResumeItem::Message {
                    role: Role::User,
                    blocks: vec![ContentBlock::text("install node")],
                },
                agent_core::ResumeItem::Goal {
                    goal: agent_core::domain::GoalState {
                        id: "goal-restore".into(),
                        objective: "finish the installation".into(),
                        status: agent_core::domain::GoalStatus::Blocked,
                        run: Some(RunId::new("old-run")),
                        tokens_used: 4_000,
                        time_used_seconds: 43,
                        continuations: 2,
                        updated_at_ms: 100,
                        blocker_reason: Some("session was closed".into()),
                    },
                },
            ],
        }),
    };
    p.new_session(opts).await.unwrap();
    let s = p.session.lock().await;
    assert!(!s.system_prompt.contains("# Resumed conversation"));
    assert_eq!(s.transcript.len(), 1);
    assert!(matches!(
        s.transcript[0],
        agent_loop::AgentMessage::User { .. }
    ));
    let goal = s.goal.as_ref().expect("standing goal restored");
    assert_eq!(goal.id, "goal-restore");
    assert_eq!(goal.status, agent_core::domain::GoalStatus::Blocked);
    assert_eq!(goal.tokens_used, 4_000);
}

#[tokio::test]
async fn session_transcript_exports_canonical_history_and_typed_goal() {
    let dir = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(provider_test_config()).await.unwrap();
    let expected_goal = agent_core::domain::GoalState {
        id: "goal-export".into(),
        objective: "preserve the experiment".into(),
        status: agent_core::domain::GoalStatus::Blocked,
        run: Some(RunId::new("old-run")),
        tokens_used: 2_500,
        time_used_seconds: 20,
        continuations: 1,
        updated_at_ms: 42,
        blocker_reason: Some("waiting for evidence".into()),
    };
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            resume: Some(agent_core::ResumeTranscript {
                truncated: false,
                items: vec![
                    agent_core::ResumeItem::Message {
                        role: Role::User,
                        blocks: vec![ContentBlock::text("replicate this")],
                    },
                    agent_core::ResumeItem::Goal {
                        goal: expected_goal.clone(),
                    },
                ],
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let exported = provider.session_transcript(&session.id).await.unwrap();
    assert!(matches!(
        &exported.items[0],
        agent_core::ResumeItem::Message { role: Role::User, blocks }
            if blocks == &vec![ContentBlock::text("replicate this")]
    ));
    assert!(matches!(
        exported.items.last(),
        Some(agent_core::ResumeItem::Goal { goal })
            if goal.id == expected_goal.id
                && goal.objective == expected_goal.objective
                && goal.tokens_used == expected_goal.tokens_used
    ));
    assert!(matches!(
        provider
            .session_transcript(&SessionId::new("wrong-session"))
            .await,
        Err(Error::SessionNotFound(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "paid trigger-precision A/B; run only with explicit model and API-key authorization"]
async fn paid_autonomous_delegation_trigger_precision() {
    use agent_core::domain::RunUsage;
    use futures::StreamExt as _;

    let api_key = std::env::var("MODEL_API_KEY")
        .or_else(|_| std::env::var("PRODUCT_API_KEY"))
        .expect("MODEL_API_KEY or PRODUCT_API_KEY must be set");
    let root_model = std::env::var("PAID_EVAL_ROOT_MODEL")
        .unwrap_or_else(|_| crate::config::DEFAULT_MODEL.into());
    let subagent_model = std::env::var("PAID_EVAL_SUBAGENT_MODEL")
        .unwrap_or_else(|_| crate::config::DEFAULT_MODEL.into());
    let base_url = std::env::var("PAID_EVAL_BASE_URL")
        .unwrap_or_else(|_| crate::config::DEFAULT_BASE_URL.into());
    let dir = tempfile::tempdir().unwrap();
    for scope in ["alpha", "beta"] {
        let scope_dir = dir.path().join(scope);
        std::fs::create_dir_all(&scope_dir).unwrap();
        for index in 0..4 {
            std::fs::write(
                scope_dir.join(format!("module_{index}.txt")),
                format!("{scope} contract {index}\n{}", "evidence line\n".repeat(80)),
            )
            .unwrap();
        }
    }
    let cases = [
        (
            "autonomous_parallel",
            "Inspect alpha and beta independently and compare their contracts with file citations.",
            true,
        ),
        (
            "autonomous_trivial",
            "Read alpha/module_0.txt and summarize it in one sentence.",
            false,
        ),
    ];
    let mut records = Vec::new();
    for (id, prompt, expected_delegate) in cases {
        let mut provider = LocalAgentProvider::new();
        provider
            .connect(ProviderConfig {
                cwd: Some(dir.path().to_string_lossy().into_owned()),
                auth_token: Some(api_key.clone()),
                extra: serde_json::json!({
                    "base_url": base_url,
                    "model": root_model,
                    "reasoning_effort": "low",
                    "temperature": 0.0,
                    "permissions": {"write_file":"deny","edit_file":"deny","apply_patch":"deny","bash":"deny"},
                    "orchestration": {
                        "max_agents": 2,
                        "minimum_context_tokens": 1,
                        "token_budget": 80_000,
                        "subagent_model": subagent_model
                    },
                    "memories": false,
                    "project_knowledge": false,
                    "browser_enabled": false,
                    "mcp_servers": []
                }),
                ..Default::default()
            })
            .await
            .unwrap();
        let session = provider
            .new_session(SessionOptions {
                cwd: Some(dir.path().to_string_lossy().into_owned()),
                mode: Some("auto".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut events = provider
            .prompt(&session.id, PromptInput::text(prompt))
            .await
            .unwrap();
        let mut delegated = false;
        let mut usage = RunUsage::default();
        while let Some(event) = events.next().await {
            match event {
                AgentEvent::ToolCall { call, .. } => {
                    delegated |= call
                        .tool_name
                        .as_deref()
                        .is_some_and(|name| name.starts_with("delegate_"));
                }
                AgentEvent::RunFinished { outcome, .. } => {
                    usage = outcome.usage.unwrap_or_default();
                }
                AgentEvent::PermissionRequest { request } => {
                    panic!("unexpected permission request: {}", request.title)
                }
                _ => {}
            }
        }
        records.push(serde_json::json!({
            "case": id,
            "expected_delegate": expected_delegate,
            "actual_delegate": delegated,
            "root_model": root_model,
            "subagent_model": subagent_model,
            "usage": usage
        }));
        assert_eq!(delegated, expected_delegate, "trigger mismatch for {id}");
    }
    println!("{}", serde_json::to_string_pretty(&records).unwrap());
}

#[tokio::test]
async fn side_question_returns_not_connected_without_llm() {
    // A provider that never connected has no LLM client: the fork must fail
    // cleanly with NotConnected, not panic.
    let mut p = LocalAgentProvider::new();
    let err = p
        .side_question(&SessionId::new("s1"), "what files have you touched?")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotConnected));
}

#[tokio::test]
async fn detached_side_question_future_owns_its_provider_snapshot() {
    let provider = LocalAgentProvider::new();
    let future =
        provider.start_side_question(&SessionId::new("side-session"), "what is the current plan?");
    drop(provider);
    let error = future.await.unwrap_err();
    assert!(matches!(error, Error::NotConnected));
}

#[tokio::test]
async fn side_question_leaves_session_transcript_byte_identical() {
    // The forked side-question call must NOT mutate session state: the active
    // run reads/writes the same transcript, and a side question that scribbled
    // into it would corrupt the run's history. We can't reach a live model in
    // a unit test, so we exercise the snapshot+build half of the fork directly
    // and assert the transcript is unchanged after building wire messages.
    use agent_loop::{AgentMessage, UserContent};

    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    p.connect(provider_test_config()).await.unwrap();
    p.new_session(SessionOptions {
        session_id: None,
        cwd: Some(dir.path().to_string_lossy().to_string()),
        mode: None,
        collaboration_mode: None,
        resume: None,
    })
    .await
    .unwrap();

    // Seed a transcript that mimics an in-flight run.
    {
        let mut s = p.session.lock().await;
        s.transcript.push(AgentMessage::User {
            content: UserContent::Text("read foo.rs and fix the bug".into()),
            timestamp: None,
        });
    }
    let before: Vec<String> = p
        .session
        .lock()
        .await
        .transcript
        .iter()
        .map(|m| format!("{m:?}"))
        .collect();

    // Rebuild the fork's wire messages (the read-only half of side_question).
    let (system_prompt, transcript) = {
        let s = p.session.lock().await;
        (s.system_prompt.clone(), s.transcript.clone())
    };
    let _messages = crate::agent_adapter::to_wire_messages(&system_prompt, &transcript);

    let after: Vec<String> = p
        .session
        .lock()
        .await
        .transcript
        .iter()
        .map(|m| format!("{m:?}"))
        .collect();
    assert_eq!(
        before, after,
        "side-question snapshot must not mutate transcript"
    );
}

#[tokio::test]
async fn plan_decision_current_approves_without_erasing_research_context() {
    use agent_loop::{AgentMessage, UserContent};

    let mut provider = LocalAgentProvider::new();
    let plan_id = {
        let mut state = provider.session.lock().await;
        state.planning.set_mode(CollaborationMode::Plan);
        state.transcript.push(AgentMessage::User {
            content: UserContent::Text("research context".into()),
            timestamp: None,
        });
        state.planning.next_proposal("1. Implement it".into()).id
    };
    provider
        .respond(
            &SessionId::new("session"),
            ClientResponse::PlanDecision {
                plan_id,
                decision: PlanDecision::Implement {
                    context: agent_core::provider::PlanImplementationContext::Current,
                },
            },
        )
        .await
        .unwrap();
    let state = provider.session.lock().await;
    assert_eq!(state.transcript.len(), 1);
    assert_eq!(state.planning.mode, CollaborationMode::Default);
    assert_eq!(
        state.planning.proposed_plan.as_ref().unwrap().status,
        agent_core::domain::ProposedPlanStatus::Approved
    );
}

#[tokio::test]
async fn plan_decision_fresh_discards_research_transcript_but_keeps_typed_plan() {
    use agent_loop::{AgentMessage, UserContent};

    let mut provider = LocalAgentProvider::new();
    let plan_id = {
        let mut state = provider.session.lock().await;
        state.planning.set_mode(CollaborationMode::Plan);
        state.transcript.push(AgentMessage::User {
            content: UserContent::Text("large research transcript".into()),
            timestamp: None,
        });
        state.planning.next_proposal("1. Implement it".into()).id
    };
    provider
        .respond(
            &SessionId::new("session"),
            ClientResponse::PlanDecision {
                plan_id,
                decision: PlanDecision::Implement {
                    context: agent_core::provider::PlanImplementationContext::Fresh,
                },
            },
        )
        .await
        .unwrap();
    let state = provider.session.lock().await;
    assert!(state.transcript.is_empty());
    assert!(state.planning.proposed_plan.is_some());
    assert!(state.planning.exited);
}
