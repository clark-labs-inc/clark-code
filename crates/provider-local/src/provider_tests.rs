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
        assert!(text.contains("attached file: note.txt"));
        assert!(text.contains("hello"));
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
            .spawn(
                Arc::new(LocalExecutor),
                "sleep 30".to_string(),
                dir.path(),
            )
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
    async fn new_session_replays_typed_resume_into_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = LocalAgentProvider::new();
        p.connect(ProviderConfig::default()).await.unwrap();
        let opts = SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            resume: Some(agent_core::ResumeTranscript {
                truncated: false,
                items: vec![agent_core::ResumeItem::Message {
                    role: Role::User,
                    blocks: vec![ContentBlock::text("install node")],
                }],
            }),
        };
        p.new_session(opts).await.unwrap();
        let s = p.session.lock().await;
        assert!(!s.system_prompt.contains("# Resumed conversation"));
        assert_eq!(s.transcript.len(), 1);
        assert!(matches!(s.transcript[0], clark_agent::AgentMessage::User { .. }));
    }
