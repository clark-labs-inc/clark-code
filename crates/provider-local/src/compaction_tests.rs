#[cfg(test)]
mod tests {
    use super::*;
    use clark_agent_compaction::TranscriptMessage;

    fn config() -> CompactionConfig {
        CompactionConfig {
            auto_compact_token_limit: 20,
            compact_request_token_limit: 1_000,
            recent_user_token_budget: 12,
            ..CompactionConfig::default()
        }
    }

    fn assistant_tool_call_for(id: &str) -> ca::AgentMessage {
        ca::AgentMessage::Assistant {
            content: ca::AssistantContent::with_tool_calls(
                Some("reading".into()),
                vec![ca::ToolCall {
                    id: id.into(),
                    name: "read_file".into(),
                    arguments: serde_json::json!({"path":"src/main.rs"}),
                }],
            ),
            stop_reason: ca::StopReason::ToolUse,
            error_message: None,
            timestamp: None,
            usage: None,
        }
    }

    fn assistant_tool_call() -> ca::AgentMessage {
        assistant_tool_call_for("call_1")
    }

    fn tool_result_for(id: &str) -> ca::AgentMessage {
        ca::AgentMessage::ToolResult {
            tool_call_id: id.into(),
            tool_name: "read_file".into(),
            content: ca::ToolResultContent::text("fn main() {}"),
            is_error: false,
            narration: None,
            details: None,
            timestamp: None,
        }
    }

    fn tool_result() -> ca::AgentMessage {
        tool_result_for("call_1")
    }

    fn rendered(message: &ca::AgentMessage) -> String {
        let mut out = String::new();
        AgentMessageView(message).render_for_compaction(&mut out);
        out
    }

    #[test]
    fn adapter_renders_tool_context_for_core_compaction() {
        let transcript = vec![
            user_message("please inspect the project"),
            assistant_tool_call(),
            tool_result(),
        ];
        let views = message_views(&transcript);

        let prepared = core::prepare_compaction(&views, &config(), &core::CharHeuristic)
            .expect("compaction request");

        assert!(prepared
            .request
            .prompt
            .contains(r#"read_file({"path":"src/main.rs"})"#));
        assert!(prepared
            .request
            .prompt
            .contains("[tool result call_1 read_file ok]"));
    }

    #[test]
    fn adapter_keeps_image_alt_in_render_but_not_recent_user_text() {
        let message = ca::AgentMessage::User {
            content: ca::UserContent::Blocks(vec![
                ca::UserBlock::Text(ca::TextContent {
                    text: "look".into(),
                }),
                ca::UserBlock::Image(ca::ImageContent {
                    source: "data:image/png;base64,abc".into(),
                    media_type: Some("image/png".into()),
                    alt: Some("diagram".into()),
                }),
            ]),
            timestamp: None,
        };

        assert!(rendered(&message).contains("[image: diagram]"));

        let mut text = String::new();
        assert!(AgentMessageView(&message).user_text_for_compaction(&mut text));
        assert_eq!(text, "look");
    }

    #[test]
    fn raw_tail_preserves_a_complete_tool_exchange() {
        let assistant = assistant_tool_call();
        let result = tool_result();
        let transcript = vec![
            user_message("old context ".repeat(200)),
            assistant.clone(),
            result.clone(),
        ];

        assert_eq!(recent_raw_tail(&transcript, 100), vec![assistant, result]);
    }

    #[test]
    fn raw_tail_never_starts_with_an_orphan_tool_result() {
        let transcript = vec![
            user_message("old context ".repeat(200)),
            assistant_tool_call(),
            tool_result(),
        ];

        assert!(recent_raw_tail(&transcript, 8).is_empty());
    }
}

#[cfg(test)]
mod usage_trigger_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    fn assistant_tool_call(id: &str) -> ca::AgentMessage {
        ca::AgentMessage::Assistant {
            content: ca::AssistantContent::with_tool_calls(
                Some("reading".into()),
                vec![ca::ToolCall {
                    id: id.into(),
                    name: "read_file".into(),
                    arguments: serde_json::json!({"path":"src/main.rs"}),
                }],
            ),
            stop_reason: ca::StopReason::ToolUse,
            error_message: None,
            timestamp: None,
            usage: None,
        }
    }

    fn tool_result(id: &str) -> ca::AgentMessage {
        ca::AgentMessage::ToolResult {
            tool_call_id: id.into(),
            tool_name: "read_file".into(),
            content: ca::ToolResultContent::text("fn main() {}"),
            is_error: false,
            narration: None,
            details: None,
            timestamp: None,
        }
    }

    fn rendered(message: &ca::AgentMessage) -> String {
        use clark_agent_compaction::TranscriptMessage;
        let mut out = String::new();
        AgentMessageView(message).render_for_compaction(&mut out);
        out
    }

    fn compactor(limit: usize) -> CheckpointCompactor {
        let provider_config = agent_core::provider::ProviderConfig {
            auth_token: Some("k".into()),
            extra: serde_json::json!({"base_url": "http://127.0.0.1:1/v1", "model": "m"}),
            ..Default::default()
        };
        let local = crate::config::LocalConfig::from_provider_config(&provider_config);
        let llm = crate::llm::LlmClient::new(&local).unwrap();
        CheckpointCompactor::new(
            llm,
            CompactionConfig {
                auto_compact_token_limit: limit,
                ..CompactionConfig::default()
            },
        )
    }

    #[test]
    fn committing_checkpoint_replaces_canonical_lineage_once() {
        let compactor = compactor(1_000);
        let mut raw = vec![user_message("large raw prefix")];
        let source_len = raw.len();
        let source_fingerprint = lineage_fingerprint(&raw, source_len);
        let checkpoint = vec![user_message("compacted summary")];
        compactor.install_checkpoint(source_len, source_fingerprint, checkpoint.clone());

        let suffix = vec![assistant_tool_call("call-2"), tool_result("call-2")];
        raw.extend(suffix.clone());
        let committed = compactor.commit_checkpoint(raw);

        let mut expected = checkpoint;
        expected.extend(suffix);
        assert_eq!(committed, expected);
        assert_eq!(compactor.commit_checkpoint(committed.clone()), committed);
    }

    #[test]
    fn provider_usage_triggers_compaction_before_the_char_heuristic() {
        use clark_agent::ContextTransform;
        let cancel = CancellationToken::new();
        // A tiny transcript the char heuristic would never flag…
        let messages = vec![user_message("short")];
        let over = ca::types::Usage {
            input_tokens: 2_000,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        let cx = ca::TransformContext {
            signal: &cancel,
            model_id: "m",
            iteration: 3,
            last_provider_usage: Some(&over),
            estimator: &ca::CHAR_HEURISTIC,
        };
        // …but the provider says the real prompt already crossed the limit.
        assert!(compactor(1_000).should_run(&messages, &cx));

        let under = ca::types::Usage {
            input_tokens: 10,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        let cx_under = ca::TransformContext {
            signal: &cancel,
            model_id: "m",
            iteration: 3,
            last_provider_usage: Some(&under),
            estimator: &ca::CHAR_HEURISTIC,
        };
        assert!(!compactor(1_000).should_run(&messages, &cx_under));
    }

    async fn summary_endpoint() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let server_calls = calls.clone();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            server_calls.fetch_add(1, Ordering::SeqCst);
            let body = [
                r#"data: {"choices":[{"delta":{"content":"checkpoint summary"}}]}"#,
                r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
                "data: [DONE]",
                "",
            ]
            .join("\n\n");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        (format!("http://{address}/v1"), calls)
    }

    #[tokio::test]
    async fn checkpoint_is_reused_and_new_raw_tool_turns_are_appended() {
        use clark_agent::ContextTransform;

        let (base_url, calls) = summary_endpoint().await;
        let llm =
            crate::llm::LlmClient::from_parts(&base_url, "fake-model", None, Vec::new(), None)
                .unwrap();
        let compactor = CheckpointCompactor::new(
            llm,
            CompactionConfig {
                auto_compact_token_limit: 200,
                compact_request_token_limit: 1_000,
                recent_user_token_budget: 100,
                ..CompactionConfig::default()
            },
        );
        let cancel = CancellationToken::new();
        let cx = ca::TransformContext {
            signal: &cancel,
            model_id: "fake-model",
            iteration: 1,
            last_provider_usage: None,
            estimator: &ca::CHAR_HEURISTIC,
        };
        let mut raw = vec![
            user_message("large original context ".repeat(100)),
            assistant_tool_call("call_1"),
            tool_result("call_1"),
        ];

        assert!(compactor.should_run(&raw, &cx));
        let first = compactor.transform(raw.clone(), &cx).await;
        assert!(rendered(first.first().unwrap()).contains("checkpoint summary"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let appended = vec![assistant_tool_call("call_2"), tool_result("call_2")];
        raw.extend(appended.clone());
        assert!(compactor.should_run(&raw, &cx));
        let second = compactor.transform(raw.clone(), &cx).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "summary request repeated");
        assert_eq!(
            &second[second.len() - appended.len()..],
            appended.as_slice()
        );

        let mut different_lineage = raw;
        // Mutate an earlier message while leaving the checkpoint boundary's
        // final message untouched; validating only a last-message anchor
        // would splice the summary into this unrelated lineage.
        different_lineage[0] = user_message("replacement lineage");
        let (projected, applied) = compactor.projected_messages(&different_lineage);
        assert!(!applied);
        assert_eq!(projected, different_lineage);
    }

    #[tokio::test]
    async fn manual_compaction_is_a_standalone_run_and_replaces_only_model_history() {
        let (base_url, calls) = summary_endpoint().await;
        let llm =
            crate::llm::LlmClient::from_parts(&base_url, "fake-model", None, Vec::new(), None)
                .unwrap();
        let original = vec![
            user_message("large original context ".repeat(100)),
            assistant_tool_call("call_1"),
            tool_result("call_1"),
        ];
        let session = Arc::new(tokio::sync::Mutex::new(SessionState {
            transcript: original.clone(),
            ..SessionState::default()
        }));
        let (tx, rx) = async_channel::unbounded();
        let run = RunId::new("compact-1");

        run_manual_compaction(
            llm,
            CompactionConfig {
                auto_compact_token_limit: usize::MAX,
                compact_request_token_limit: usize::MAX,
                recent_user_token_budget: 0,
                ..CompactionConfig::disabled()
            },
            session.clone(),
            tx,
            run.clone(),
            CancellationToken::new(),
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let compacted = session.lock().await.transcript.clone();
        assert_ne!(compacted, original);
        assert_eq!(compacted.len(), 1);
        assert!(rendered(&compacted[0]).contains("checkpoint summary"));

        let mut events = Vec::new();
        while let Ok(event) = rx.recv().await {
            events.push(event);
        }
        assert!(matches!(
            events.first(),
            Some(AgentEvent::RunStarted { run: event_run }) if event_run == &run
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Trace { source, payload, .. }
                if source == "clark_code_compaction" && payload["trigger"] == "manual"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageChunk {
                role: Role::System,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ContextCompacted { transcript, .. }
                if transcript.items.len() == 1
        )));
        assert!(matches!(
            events.last(),
            Some(AgentEvent::RunFinished { outcome, .. }) if outcome.status == RunStatus::Done
        ));
    }
}
