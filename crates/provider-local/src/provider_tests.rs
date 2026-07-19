use super::*;
use agent_core::domain::PendingUpload;

#[test]
fn prompt_text_joins_blocks() {
    let input = PromptInput {
        blocks: vec![ContentBlock::text("hello "), ContentBlock::text("world")],
        attachments: Vec::new(),
    };
    assert_eq!(prompt_text(&input), "hello world");
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
fn kimi_native_images_are_forwarded_before_the_user_request() {
    let attachments = vec![PendingUpload {
        filename: "design.png".into(),
        content_type: "image/png".into(),
        data_base64: "QUJD".into(),
    }];

    let content = model_user_content("review the design".into(), &attachments, true);
    let clark_agent::UserContent::Blocks(blocks) = content else {
        panic!("expected multimodal user content");
    };
    assert!(matches!(
        &blocks[0],
        clark_agent::UserBlock::Image(image)
            if image.source == "data:image/png;base64,QUJD"
                && image.alt.as_deref() == Some("design.png")
    ));
    assert!(matches!(
        &blocks[1],
        clark_agent::UserBlock::Text(text) if text.text == "review the design"
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
        clark_agent::UserContent::Text(text) if text == "fallback description"
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

#[tokio::test]
async fn new_session_requires_cwd() {
    let mut p = LocalAgentProvider::new();
    p.connect(ProviderConfig::default()).await.unwrap();
    let err = p.new_session(SessionOptions::default()).await.unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
}

#[tokio::test]
async fn isolated_orchestration_session_has_no_ambient_writable_surfaces() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".clark")).unwrap();
    std::fs::write(
        dir.path().join(".clark/settings.json"),
        r#"{
          "hooks":{"PreToolUse":[{"matcher":"*","command":"touch /tmp/should-not-run"}]},
          "permissions":{"allow":["bash"]},
          "check_command":"touch /tmp/should-not-run"
        }"#,
    )
    .unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            extra: serde_json::json!({
                "isolated_writer": true,
                "memories": false,
                "research": false,
                "browser_enabled": false,
                "mcp_servers": [],
                "permissions": {
                    "write_file": "allow",
                    "edit_file": "allow",
                    "apply_patch": "allow",
                    "bash": "deny"
                }
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
}

#[tokio::test]
async fn set_mode_flips_plan_mode_flag() {
    let mut p = LocalAgentProvider::new();
    let session_id = SessionId::new("s1");
    assert!(!p.session.lock().await.plan_mode);

    p.set_mode(&session_id, "plan".to_string()).await.unwrap();
    assert!(p.session.lock().await.plan_mode);

    p.set_mode(&session_id, "ask".to_string()).await.unwrap();
    assert!(!p.session.lock().await.plan_mode);
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
    p.connect(ProviderConfig::default()).await.unwrap();
    let opts = SessionOptions {
        cwd: Some(dir.path().to_string_lossy().to_string()),
        mode: None,
        resume: None,
    };
    let session = p.new_session(opts).await.unwrap();
    assert_eq!(session.provider, ProviderId::new("local"));
    let s = p.session.lock().await;
    assert!(!s.system_prompt.is_empty());
    assert!(s.transcript.is_empty());
    assert!(!s.system_prompt.contains("# Resumed conversation"));
}

#[tokio::test]
async fn cached_system_prompt_excludes_mutable_project_instructions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "UNIQUE_PROJECT_RULE").unwrap();
    let mut provider = LocalAgentProvider::new();
    provider.connect(ProviderConfig::default()).await.unwrap();
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
async fn remote_connect_hides_desktop_mobile_capabilities() {
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            extra: serde_json::json!({
                "remote": {
                    "ws_url": "ws://127.0.0.1:9",
                    "token": "test-token",
                    "cwd": "/remote/project"
                }
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let registry = provider.registry.as_ref().unwrap();
    assert!(registry.get("android_boot_emulator").is_none());
    assert!(registry.get("ios_boot_simulator").is_none());
    assert!(registry.get("bash").is_some());
}

#[tokio::test]
async fn orchestration_tools_are_default_available_but_can_be_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let mut enabled = LocalAgentProvider::new();
    enabled.connect(ProviderConfig::default()).await.unwrap();
    let registry = enabled.registry.as_ref().unwrap();
    assert!(registry.get("delegate_read_only").is_some());
    assert!(registry.get("resolve_delegation").is_some());
    assert!(registry.get("delegate_coding_workstreams").is_some());
    assert!(registry.get("resolve_coding_workstreams").is_some());
    enabled
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let state = enabled.session.lock().await;
    assert!(!state.system_prompt.contains("bounded delegation"));
    drop(state);

    let mut disabled = LocalAgentProvider::new();
    disabled
        .connect(ProviderConfig {
            extra: serde_json::json!({"orchestration": {"enabled": false}}),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(disabled
        .registry
        .as_ref()
        .unwrap()
        .get("delegate_read_only")
        .is_none());
    assert!(disabled
        .registry
        .as_ref()
        .unwrap()
        .get("delegate_coding_workstreams")
        .is_none());
}

#[tokio::test]
async fn new_session_mode_option_controls_plan_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    p.connect(ProviderConfig::default()).await.unwrap();

    let session = p
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: Some("plan".to_string()),
            resume: None,
        })
        .await
        .unwrap();
    assert_eq!(session.mode.as_deref(), Some("plan"));
    assert!(p.session.lock().await.plan_mode);

    // A provider instance reused for a fresh session must not inherit the
    // stale flag.
    p.new_session(SessionOptions {
        cwd: Some(dir.path().to_string_lossy().to_string()),
        mode: Some("auto".to_string()),
        resume: None,
    })
    .await
    .unwrap();
    assert!(!p.session.lock().await.plan_mode);
}

#[tokio::test]
async fn set_mode_transitions_queue_the_one_shot_exit_note() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    p.connect(ProviderConfig::default()).await.unwrap();
    let session = p
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: Some("plan".to_string()),
            resume: None,
        })
        .await
        .unwrap();

    p.set_mode(&session.id, "auto".to_string()).await.unwrap();
    {
        let s = p.session.lock().await;
        assert!(!s.plan_mode);
        assert!(s.plan_exited, "leaving plan mode queues the exit note");
    }

    // Re-entering cancels a queued note (quick toggle must not tell the
    // model it both entered and exited).
    p.set_mode(&session.id, "plan".to_string()).await.unwrap();
    {
        let s = p.session.lock().await;
        assert!(s.plan_mode);
        assert!(!s.plan_exited);
    }
}

#[tokio::test]
async fn new_session_replays_typed_resume_into_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    p.connect(ProviderConfig::default()).await.unwrap();
    let opts = SessionOptions {
        cwd: Some(dir.path().to_string_lossy().to_string()),
        mode: None,
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
                        token_budget: Some(20_000),
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
        clark_agent::AgentMessage::User { .. }
    ));
    let goal = s.goal.as_ref().expect("standing goal restored");
    assert_eq!(goal.id, "goal-restore");
    assert_eq!(goal.status, agent_core::domain::GoalStatus::Blocked);
    assert_eq!(goal.tokens_used, 4_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "paid trigger-precision A/B; run only with explicit model and API-key authorization"]
async fn paid_explicit_vs_proactive_delegation_trigger_precision() {
    use agent_core::domain::RunUsage;
    use futures::StreamExt as _;

    let api_key = std::env::var("CLARK_CODE_API_KEY")
        .or_else(|_| std::env::var("CLARK_API_KEY"))
        .expect("CLARK_CODE_API_KEY or CLARK_API_KEY must be set");
    let root_model = std::env::var("CLARK_PAID_EVAL_ROOT_MODEL")
        .unwrap_or_else(|_| "clark-code:kimi_k27_code".into());
    let subagent_model = std::env::var("CLARK_PAID_EVAL_SUBAGENT_MODEL")
        .unwrap_or_else(|_| "clark-code:kimi_k27_code".into());
    let base_url = std::env::var("CLARK_PAID_EVAL_BASE_URL")
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
            "explicit_ordinary",
            "explicit",
            "Inspect alpha and beta and summarize their contracts with file citations.",
            false,
        ),
        (
            "explicit_requested",
            "explicit",
            "Use exactly two read-only subagents in parallel. Have one inspect alpha and one inspect beta. Wait for both, verify their citations, then compare the contracts.",
            true,
        ),
        (
            "proactive_parallel",
            "proactive",
            "Inspect alpha and beta independently and compare their contracts with file citations.",
            true,
        ),
        (
            "proactive_trivial",
            "proactive",
            "Read alpha/module_0.txt and summarize it in one sentence.",
            false,
        ),
    ];
    let mut records = Vec::new();
    for (id, mode, prompt, expected_delegate) in cases {
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
                    "max_iterations": 64,
                    "permissions": {"write_file":"deny","edit_file":"deny","apply_patch":"deny","bash":"deny"},
                    "orchestration": {
                        "enabled": true,
                        "mode": mode,
                        "max_agents": 2,
                        "max_attempts": 1,
                        "minimum_context_tokens": 1,
                        "token_budget": 80_000,
                        "subagent_model": subagent_model
                    },
                    "research": false,
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
            "mode": mode,
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
async fn side_question_leaves_session_transcript_byte_identical() {
    // The forked side-question call must NOT mutate session state: the active
    // run reads/writes the same transcript, and a side question that scribbled
    // into it would corrupt the run's history. We can't reach a live model in
    // a unit test, so we exercise the snapshot+build half of the fork directly
    // and assert the transcript is unchanged after building wire messages.
    use clark_agent::{AgentMessage, UserContent};

    let dir = tempfile::tempdir().unwrap();
    let mut p = LocalAgentProvider::new();
    p.connect(ProviderConfig::default()).await.unwrap();
    p.new_session(SessionOptions {
        cwd: Some(dir.path().to_string_lossy().to_string()),
        mode: None,
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
