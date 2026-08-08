//! End-to-end test of the local agent loop against a fake OpenAI-compatible
//! streaming endpoint. Drives `LocalAgentProvider` through one full round-trip:
//! the model asks to read a real local file, the loop executes the tool on disk,
//! feeds the result back, and the model returns a final answer.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use agent_core::domain::{
    AgentEvent, ContentBlock, GoalStatus, MessagePhase, PendingUpload, Role, RunStatus, ToolStatus,
};
use agent_core::provider::{
    ClientResponse, CollaborationMode, PlanDecision, PlanImplementationContext, PromptInput,
    Provider, ProviderConfig, SessionOptions,
};
use agent_core::TimelineItem;
use agent_orchestration::{ExecutionEvent, ExecutionEventKind, ExecutionLedger, ExecutionState};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const TEST_SCOUT_MODEL: &str = "scout-model";
const TEST_SECURITY_MODEL: &str = "security-model";

/// SSE body for the first model call: ask to read `hello.txt`.
fn tool_call_body() -> String {
    [
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":\"hello.txt\"}"}}]}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n")
}

/// SSE body where one assistant response pairs visible progress text with the
/// tool request that proves work is continuing.
fn commentary_tool_call_body() -> String {
    [
        r#"data: {"choices":[{"delta":{"content":"I found the target file. I’ll read it now, then verify the result."}}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":\"hello.txt\"}"}}]}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n")
}

/// SSE body for the second model call: the final answer.
fn final_body() -> String {
    final_answer_body("The file says: hi")
}

fn final_answer_body(text: &str) -> String {
    let arguments = json!({"content": text}).to_string();
    [
        format!(
            "data: {}",
            json!({"choices":[{"delta":{"tool_calls":[{
                "index": 0,
                "id": "final-answer",
                "function": {"name": "final_answer", "arguments": arguments}
            }]}}]})
        ),
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#.to_string(),
        "data: [DONE]".to_string(),
        String::new(),
    ]
    .join("\n\n")
}

fn compact_summary_body() -> String {
    [
        r#"data: {"choices":[{"delta":{"content":"Summary: the user asked a large coding question. Continue with a concise answer."}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n")
}

fn http_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

/// SSE body for the vision-fallback call: describes two attached images.
fn vision_description_body() -> String {
    [
        r#"data: {"choices":[{"delta":{"content":"Image 1 shows a red square. Image 2 shows a blue circle."}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n")
}

/// SSE body for a first model call that asks to write a file (a mutating tool).
fn write_call_body() -> String {
    [
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_2","function":{"name":"write_file","arguments":"{\"path\":\"out.txt\",\"content\":\"written\"}"}}]}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n")
}

/// Serve a fixed sequence of chat-completion responses, one per request.
/// Returns every request's raw captured bytes, in arrival order, once all
/// bodies have been served — most callers ignore the return value (via
/// `tokio::spawn` without awaiting the handle); tests that need to inspect
/// the outgoing request JSON can `.await` the `JoinHandle` instead.
async fn serve(listener: TcpListener, bodies: Vec<String>) -> Vec<Vec<u8>> {
    let responses = bodies.iter().map(|body| http_response(body)).collect();
    serve_responses(listener, responses).await
}

async fn serve_responses(listener: TcpListener, responses: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut captured = Vec::with_capacity(responses.len());
    for _ in 0..responses.len() {
        let Ok((mut sock, _)) = listener.accept().await else {
            break;
        };
        captured.push(read_request(&mut sock).await);
        let n = calls.fetch_add(1, Ordering::SeqCst);
        let _ = sock.write_all(&responses[n]).await;
        let _ = sock.flush().await;
    }
    captured
}

/// Read one HTTP request off `sock` and return its raw bytes (headers + body).
async fn read_request(sock: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut content_length = None;
    loop {
        let Ok(n) = sock.read(&mut tmp).await else {
            return buf;
        };
        if n == 0 {
            return buf;
        }
        buf.extend_from_slice(&tmp[..n]);
        if content_length.is_none() {
            if let Some(headers_end) = find_headers_end(&buf) {
                let headers = String::from_utf8_lossy(&buf[..headers_end]);
                content_length = headers.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                });
            }
        }
        if let (Some(headers_end), Some(len)) = (find_headers_end(&buf), content_length) {
            if buf.len() >= headers_end + 4 + len {
                return buf;
            }
        }
    }
}

fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parse the JSON body out of one `read_request`-captured raw HTTP request.
fn request_json(raw: &[u8]) -> serde_json::Value {
    let headers_end = find_headers_end(raw).expect("captured request has header terminator");
    serde_json::from_slice(&raw[headers_end + 4..]).expect("request body is valid JSON")
}

#[tokio::test]
async fn scout_uses_its_host_route_when_the_conversation_uses_the_included_lane() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = tokio::spawn(serve(listener, vec![final_body()]));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": provider_local::DEFAULT_MODEL,
                "reasoning_effort": "max",
                "skill_model_overrides": {
                    "scout": {"model": TEST_SCOUT_MODEL, "reasoning_effort": "max"}
                },
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("$scout:scout map this business system"),
        )
        .await
        .unwrap();
    while let Some(event) = stream.next().await {
        if matches!(event, AgentEvent::RunFinished { .. }) {
            break;
        }
    }

    let requests = captured.await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = request_json(&requests[0]);
    assert_eq!(request["model"], TEST_SCOUT_MODEL);
    assert_eq!(
        request.get("reasoning_effort").and_then(Value::as_str),
        Some("max"),
        "Scout must use its host-pinned reasoning configuration"
    );
}

#[tokio::test]
async fn security_uses_host_pinned_model_instead_of_conversation_model_settings() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = tokio::spawn(serve(listener, vec![final_body()]));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "local-model-large",
                "reasoning_effort": "max",
                "skill_model_overrides": {
                    "security": {"model": TEST_SECURITY_MODEL, "reasoning_effort": "max"}
                },
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("$security:security-deep scan this repository deeply"),
        )
        .await
        .unwrap();
    while let Some(event) = stream.next().await {
        if matches!(event, AgentEvent::RunFinished { .. }) {
            break;
        }
    }

    let requests = captured.await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = request_json(&requests[0]);
    assert_eq!(request["model"], TEST_SECURITY_MODEL);
    assert_eq!(
        request.get("reasoning_effort").and_then(Value::as_str),
        Some("max"),
        "Security must use the host-provided reasoning configuration"
    );
    assert!(
        request["tools"].as_array().is_some_and(|tools| tools
            .iter()
            .any(|tool| tool["function"]["name"] == "security_scan_contract")),
        "selecting Security must expose its deterministic contract on the first model turn"
    );
    for name in ["delegate_read_only", "resolve_delegation"] {
        assert!(
            request["tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().any(|tool| tool["function"]["name"] == name)),
            "selecting deep Security must expose {name} on the first model turn"
        );
    }
}

#[tokio::test]
async fn security_skill_fake_provider_seals_artifact_and_exposes_history() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join(".agent/security-scans/fake-e2e")).unwrap();
    std::fs::write(
        dir.path().join("SECURITY.md"),
        "Review src/. This is a deterministic fake-provider fixture.\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/handler.rs"),
        "pub fn handler() -> bool { true }\n",
    )
    .unwrap();
    let inventory = provider_local::security::collect_security_inventory(
        &provider_local::LocalExecutor,
        dir.path(),
        dir.path(),
    )
    .await
    .unwrap();
    let bundle = json!({
        "contractVersion": provider_local::security::SECURITY_SCAN_CONTRACT_VERSION,
        "scanId": "fake-e2e",
        "mode": "standard",
        "model": TEST_SECURITY_MODEL,
        "scope": ".",
        "inventoryId": inventory.inventory_id,
        "phase": "reporting",
        "threatModel": {
            "assets": ["Fixture state"],
            "trustBoundaries": ["Caller to fixture handler"],
            "attackerInputs": ["Fixture request"],
            "invariants": ["The deterministic fixture has no reportable sink"]
        },
        "coverage": inventory.paths.iter().map(|path| json!({
            "path": path,
            "status": "reviewed",
            "reason": null
        })).collect::<Vec<_>>(),
        "supportingCoverage": [],
        "diffTarget": null,
        "deepRunId": null,
        "candidates": []
    });
    let scan_path = ".agent/security-scans/fake-e2e/scan.json";

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = tokio::spawn(serve(
        listener,
        vec![
            tool_call_sse(
                "sec-schema",
                "security_scan_contract",
                json!({"action": "schema"}),
            ),
            tool_call_sse(
                "sec-inventory",
                "security_scan_contract",
                json!({"action": "inventory", "scope": "."}),
            ),
            tool_call_sse(
                "sec-write",
                "write_file",
                json!({
                    "path": scan_path,
                    "content": serde_json::to_string_pretty(&bundle).unwrap()
                }),
            ),
            tool_call_sse(
                "sec-finalize",
                "security_scan_contract",
                json!({"action": "finalize", "path": scan_path}),
            ),
            text_body("Security scan sealed with zero reportable findings."),
        ],
    ));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "local-model-large",
                "skill_model_overrides": {
                    "security": {"model": TEST_SECURITY_MODEL, "reasoning_effort": "max"}
                },
                "memories": false,
                "sandbox_mode": "disabled",
                "permissions": {"write_file": "allow"}
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("$security:security-scan scan this deterministic fixture"),
        )
        .await
        .unwrap();
    let mut tools = Vec::new();
    let mut status = None;
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::ToolCall { call, .. } => {
                if let Some(name) = call.tool_name {
                    tools.push(name);
                }
            }
            AgentEvent::RunFinished { outcome, .. } => {
                status = Some(outcome.status);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(status, Some(RunStatus::Done));
    assert_eq!(
        tools,
        [
            "security_scan_contract",
            "security_scan_contract",
            "write_file",
            "security_scan_contract"
        ]
    );
    assert!(dir
        .path()
        .join(".agent/security-scans/fake-e2e/seal.json")
        .is_file());
    let history = provider_local::list_security_scans(&provider_local::LocalExecutor, dir.path())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].bundle.scan_id, "fake-e2e");
    assert!(history[0].seal.is_some());

    let requests = served.await.unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests
        .iter()
        .all(|request| request_json(request)["model"] == TEST_SECURITY_MODEL));
}

#[tokio::test]
async fn local_loop_reads_file_and_answers() {
    // A real project dir with a real file the tool will read.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, vec![tool_call_body(), final_body()]));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "fake-model",
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();

    let mut stream = provider
        .prompt(&session.id, PromptInput::text("What does hello.txt say?"))
        .await
        .unwrap();

    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        let done = matches!(&ev, AgentEvent::RunFinished { .. });
        events.push(ev);
        if done {
            break;
        }
    }

    // The model's read_file call ran locally against the real file.
    let read_tool = events.iter().find_map(|e| match e {
        AgentEvent::ToolCall { call, .. } if call.tool_name.as_deref() == Some("read_file") => {
            Some(call)
        }
        _ => None,
    });
    assert!(
        read_tool.is_some(),
        "expected a read_file tool call: {events:?}"
    );
    assert_eq!(read_tool.unwrap().title, "Read hello.txt");

    // It completed (not failed), having found the file.
    let completed = events.iter().any(|e| {
        matches!(
            e,
            AgentEvent::ToolCallUpdate { patch, .. }
                if patch.status == Some(ToolStatus::Completed)
        )
    });
    assert!(completed, "expected the tool call to complete: {events:?}");

    // The final assistant text streamed through.
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(text.contains("The file says: hi"), "got: {text:?}");

    // The run finished cleanly.
    let finished = events.iter().any(|e| {
        matches!(
            e,
            AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
        )
    });
    assert!(finished, "expected RunFinished Done: {events:?}");
    let execution = events.iter().find_map(|event| match event {
        AgentEvent::RunFinished { outcome, .. } => outcome.execution.as_ref(),
        _ => None,
    });
    let execution = execution.expect("default single-agent run has a /root execution receipt");
    assert_eq!(execution.root_path, "/root");
    assert_eq!(execution.attempts, 1);
    assert_eq!(execution.recoveries, 0);
    assert_eq!(execution.completed_tools, vec!["read_file"]);
}

#[tokio::test]
async fn local_loop_projects_text_with_tool_as_commentary_then_plain_text_as_final() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(
        listener,
        vec![commentary_tool_call_body(), final_body()],
    ));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "fake-model",
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(&session.id, PromptInput::text("What does hello.txt say?"))
        .await
        .unwrap();

    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        let finished = matches!(event, AgentEvent::RunFinished { .. });
        events.push(event);
        if finished {
            break;
        }
    }

    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::MessagePhase {
            phase: MessagePhase::Commentary,
            ..
        }
    )));
    let snapshot = agent_core::reduce_all(&events);
    assert_eq!(
        snapshot.timeline.len(),
        3,
        "timeline: {:?}",
        snapshot.timeline
    );
    assert!(matches!(
        &snapshot.timeline[0],
        TimelineItem::Message {
            role: Role::Agent,
            phase: Some(MessagePhase::Commentary),
            ..
        }
    ));
    assert!(matches!(
        &snapshot.timeline[1],
        TimelineItem::ToolCall { .. }
    ));
    assert!(matches!(
        &snapshot.timeline[2],
        TimelineItem::Message {
            role: Role::Agent,
            phase: Some(MessagePhase::FinalAnswer),
            ..
        }
    ));
}

#[tokio::test]
async fn local_loop_auto_compacts_large_transcript_before_sampling() {
    let dir = tempfile::tempdir().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![compact_summary_body(), final_body(), final_body()],
    ));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "fake-model",
                "memories": false,
                "sandbox_mode": "disabled",
                "auto_compact_token_limit": 2_000,
                "compact_request_token_limit": 1_500,
                "compact_recent_user_token_budget": 200
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();

    let huge_prompt = format!(
        "What should I do next?\n{}",
        "important detail ".repeat(900)
    );
    let mut stream = provider
        .prompt(&session.id, PromptInput::text(huge_prompt))
        .await
        .unwrap();

    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        let done = matches!(&ev, AgentEvent::RunFinished { .. });
        events.push(ev);
        if done {
            break;
        }
    }

    let text: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(text.contains("The file says: hi"), "got: {text:?}");
    assert!(
        events.iter().any(
            |e| matches!(e, AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done)
        ),
        "expected RunFinished Done: {events:?}"
    );

    // A new provider run must start from the installed checkpoint. Before the
    // fix, the request-time transform left the raw transcript canonical, so
    // this follow-up invoked the summarizer all over again.
    let mut follow_up = provider
        .prompt(&session.id, PromptInput::text("Any final caveat?"))
        .await
        .unwrap();
    let follow_up_events = drain_run(&mut follow_up).await;
    assert!(follow_up_events.iter().any(|event| matches!(
        event,
        AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
    )));

    let captured = serve_handle.await.unwrap();
    assert_eq!(captured.len(), 3, "one summary plus two normal turns");
    let follow_up_request = String::from_utf8_lossy(&captured[2]);
    assert!(follow_up_request.contains("compacted transcript handoff"));
    assert!(!follow_up_request.contains(&"important detail ".repeat(100)));
}

#[tokio::test]
async fn mutating_tool_waits_for_permission_then_writes() {
    let dir = tempfile::tempdir().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, vec![write_call_body(), final_body()]));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "fake",
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(&session.id, PromptInput::text("create out.txt"))
        .await
        .unwrap();

    // Consume events; when the gate fires, approve it once and keep going.
    let mut saw_permission = false;
    let mut approved = false;
    let mut finished = false;
    let mut lifecycle = Vec::new();
    while let Some(ev) = stream.next().await {
        if let AgentEvent::Trace {
            source, payload, ..
        } = &ev
        {
            if source == "execution_lifecycle" {
                lifecycle.push(
                    serde_json::from_value::<ExecutionEvent>(payload.clone())
                        .expect("typed lifecycle trace"),
                );
            }
        }
        match &ev {
            AgentEvent::PermissionRequest { request } => {
                saw_permission = true;
                // The file must NOT exist yet — the tool is gated before running.
                assert!(!dir.path().join("out.txt").exists());
                provider
                    .respond(
                        &session.id,
                        ClientResponse::Permission {
                            request: request.id.clone(),
                            option: "allow_once".into(),
                            feedback: None,
                        },
                    )
                    .await
                    .unwrap();
                approved = true;
            }
            AgentEvent::RunFinished { .. } => {
                finished = true;
                break;
            }
            _ => {}
        }
    }

    assert!(
        saw_permission,
        "expected a permission request for write_file"
    );
    assert!(approved);
    assert!(finished);
    // After approval, the file was actually written locally.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
        "written"
    );
    assert!(lifecycle.iter().any(|event| matches!(
        event.kind,
        ExecutionEventKind::StateChanged {
            to: ExecutionState::AwaitingInput,
            ..
        }
    )));
    assert_eq!(
        ExecutionLedger::replay(&lifecycle).unwrap().state,
        ExecutionState::Completed
    );
}

/// Neither local coding model can see images: an image-bearing prompt must
/// trigger exactly ONE batched vision-fallback call (not one per image)
/// before the coding model ever sees the turn, and the coding model's own
/// request must stay plain text — no raw image data — with the vision
/// description spliced in instead. Regression coverage for the bug where
/// images were silently dropped and replaced with a bare filename note that
/// sent the model hunting the filesystem for a file that never existed.
#[tokio::test]
async fn image_attachments_are_described_by_vision_fallback_before_the_coding_call() {
    let dir = tempfile::tempdir().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![vision_description_body(), final_body()],
    ));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "fake-model",
                "vision_model": "vision-model",
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();

    let input = PromptInput {
        blocks: vec![ContentBlock::text("what do you see?")],
        attachments: vec![
            PendingUpload {
                filename: "one.png".into(),
                content_type: "image/png".into(),
                data_base64: "aGVsbG8=".into(),
            },
            PendingUpload {
                filename: "two.png".into(),
                content_type: "image/png".into(),
                data_base64: "d29ybGQ=".into(),
            },
        ],
    };
    let mut stream = provider.prompt(&session.id, input).await.unwrap();

    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        let done = matches!(&ev, AgentEvent::RunFinished { .. });
        events.push(ev);
        if done {
            break;
        }
    }
    assert!(
        events.iter().any(
            |e| matches!(e, AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done)
        ),
        "expected RunFinished Done: {events:?}"
    );

    let captured = serve_handle.await.unwrap();
    assert_eq!(
        captured.len(),
        2,
        "expected exactly one vision call plus one coding call"
    );

    // First request: the vision fallback, hitting the stateless multimodal
    // model with BOTH images batched into one content-parts array.
    let vision_req = request_json(&captured[0]);
    assert_eq!(vision_req["model"], "vision-model");
    let vision_content = &vision_req["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user")
        .expect("vision request has a user message")["content"];
    let image_parts: Vec<_> = vision_content
        .as_array()
        .expect("vision user content is a content-parts array, not a bare string")
        .iter()
        .filter(|p| p["type"] == "image_url")
        .collect();
    assert_eq!(image_parts.len(), 2, "both images batched into one call");

    // Second request: the coding model. Its user content must be a plain
    // string (no raw image data) with the vision description spliced in.
    let coding_req = request_json(&captured[1]);
    assert_eq!(coding_req["model"], "fake-model");
    let coding_content = &coding_req["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user")
        .expect("coding request has a user message")["content"];
    let coding_text = coding_content
        .as_str()
        .expect("coding model's user content stays a plain string, not content-parts");
    assert!(
        coding_text.contains("Image 1 shows a red square"),
        "expected the vision description spliced in: {coding_text:?}"
    );
    assert!(
        !coding_text.contains("aGVsbG8=") && !coding_text.contains("data:image"),
        "raw image data must never reach the coding model: {coding_text:?}"
    );
}

/// One SSE chat-completion body that calls `name` with `args`, then stops for
/// tool results.
fn tool_call_sse(call_id: &str, name: &str, args: serde_json::Value) -> String {
    let chunk = json!({
        "choices": [{"delta": {"tool_calls": [{
            "index": 0,
            "id": call_id,
            "function": {"name": name, "arguments": args.to_string()}
        }]}}]
    });
    format!(
        "data: {chunk}\n\ndata: {}\n\ndata: [DONE]\n\n",
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#
    )
}

fn text_sse(text: &str) -> String {
    let chunk = json!({"choices": [{"delta": {"content": text}}]});
    format!(
        "data: {chunk}\n\ndata: {}\n\ndata: [DONE]\n\n",
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#
    )
}

/// The full typed Plan Mode journey against the real engine: edits are denied,
/// proposals are first-class state (not permissions or files), feedback starts
/// a revision, approval exits read-only mode, and implementation can proceed.
#[tokio::test]
async fn plan_mode_journey_denies_edits_threads_feedback_and_builds_after_approval() {
    let dir = tempfile::tempdir().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![
            // Planning turn 1: an edit is refused, then proposal v1 is emitted.
            tool_call_sse(
                "c1",
                "write_file",
                json!({"path": "out.txt", "content": "written"}),
            ),
            text_sse("<proposed_plan>Plan v1: add out.txt</proposed_plan>"),
            // Planning turn 2: feedback arrives as a normal user turn and the
            // proposal keeps its identity while incrementing its revision.
            text_sse("<proposed_plan>Plan v2: add out.txt with tests</proposed_plan>"),
            // Implementation turn after approval.
            tool_call_sse(
                "c4",
                "write_file",
                json!({"path": "out.txt", "content": "written"}),
            ),
            final_body(),
        ],
    ));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "fake-model",
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: Some(CollaborationMode::Plan),
            resume: None,
        })
        .await
        .unwrap();
    assert_eq!(session.collaboration_mode, CollaborationMode::Plan);

    let mut stream = provider
        .prompt(&session.id, PromptInput::text("add an out.txt file"))
        .await
        .unwrap();

    let mut first_plan = None;
    while let Some(ev) = stream.next().await {
        match &ev {
            AgentEvent::PermissionRequest { request } => {
                panic!("Plan Mode writes and proposals must not become permissions: {request:?}");
            }
            AgentEvent::ProposedPlanUpdated { plan, .. } => first_plan = Some(plan.clone()),
            AgentEvent::RunFinished { .. } => break,
            _ => {}
        }
    }
    assert!(!dir.path().join("out.txt").exists());
    let first_plan = first_plan.expect("typed proposal v1");
    assert_eq!(first_plan.revision, 1);

    provider
        .respond(
            &session.id,
            ClientResponse::PlanDecision {
                plan_id: first_plan.id.clone(),
                decision: PlanDecision::ContinuePlanning {
                    feedback: Some("make it two files".into()),
                },
            },
        )
        .await
        .unwrap();

    let mut stream = provider
        .prompt(&session.id, PromptInput::text("make it two files"))
        .await
        .unwrap();
    let mut revised = None;
    while let Some(ev) = stream.next().await {
        if let AgentEvent::ProposedPlanUpdated { plan, .. } = &ev {
            revised = Some(plan.clone());
        }
        if matches!(ev, AgentEvent::RunFinished { .. }) {
            break;
        }
    }
    let revised = revised.expect("typed proposal v2");
    assert_eq!(revised.id, first_plan.id);
    assert_eq!(revised.revision, 2);

    provider
        .respond(
            &session.id,
            ClientResponse::PlanDecision {
                plan_id: revised.id.clone(),
                decision: PlanDecision::Implement {
                    context: PlanImplementationContext::Current,
                },
            },
        )
        .await
        .unwrap();
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Implement the approved plan."),
        )
        .await
        .unwrap();
    let mut saw_approved = false;
    while let Some(ev) = stream.next().await {
        match ev {
            AgentEvent::ProposedPlanUpdated { plan, .. } => {
                saw_approved = plan.status == agent_core::domain::ProposedPlanStatus::Approved;
            }
            AgentEvent::PermissionRequest { request } => {
                provider
                    .respond(
                        &session.id,
                        ClientResponse::Permission {
                            request: request.id,
                            option: "allow_once".into(),
                            feedback: None,
                        },
                    )
                    .await
                    .unwrap();
            }
            AgentEvent::RunFinished { .. } => break,
            _ => {}
        }
    }
    assert!(saw_approved);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
        "written"
    );

    let captured = serve_handle.await.unwrap();
    assert_eq!(captured.len(), 5, "five model calls end to end");
    let last_user = |i: usize| -> String {
        request_json(&captured[i])["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rfind(|m| m["role"] == "user")
            .expect("request has a user message")["content"]
            .as_str()
            .expect("user content is a plain string")
            .to_string()
    };
    let last_developer = |i: usize| -> String {
        request_json(&captured[i])["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rfind(|message| message["role"] == "developer")
            .expect("request has a developer collaboration instruction")["content"]
            .as_str()
            .expect("developer content is plain text")
            .to_string()
    };

    let turn1 = last_developer(0);
    assert!(turn1.contains("Plan Mode is active."));
    assert!(turn1.contains("Propose, do not execute"));
    assert!(!last_user(0).contains("Plan Mode is active"));
    assert!(!turn1.contains("plan.md"));
    let call2 = request_json(&captured[1]).to_string();
    assert!(call2.contains("Plan Mode is active"));
    assert!(last_user(2).contains("make it two files"));
    let revision_turn = last_developer(2);
    assert!(revision_turn.contains("Plan Mode remains active"));
    assert!(revision_turn.contains("Previous proposal"));
    let implementation_turn = last_developer(3);
    assert!(implementation_turn.contains("Plan Mode is off"));
    assert!(implementation_turn.contains("Plan v2"));
    assert!(!implementation_turn.contains("Plan Mode is active."));
}

/// One SSE body carrying TWO tool calls in a single assistant turn.
fn two_tool_calls_sse(name: &str, args_a: serde_json::Value, args_b: serde_json::Value) -> String {
    let chunk = json!({
        "choices": [{"delta": {"tool_calls": [
            {"index": 0, "id": "par_a", "function": {"name": name, "arguments": args_a.to_string()}},
            {"index": 1, "id": "par_b", "function": {"name": name, "arguments": args_b.to_string()}}
        ]}}]
    });
    format!(
        "data: {chunk}\n\ndata: {}\n\ndata: [DONE]\n\n",
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#
    )
}

/// An in-band provider failure ending the stream — the shape OpenRouter uses
/// when it must fail after committing to SSE.
fn overflow_error_body() -> String {
    [
        r#"data: {"error":{"code":400,"message":"This endpoint's maximum context length is 1000 tokens. However, you requested 2000 tokens."}}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n")
}

fn text_body(text: &str) -> String {
    final_answer_body(text)
}

fn plain_text_body(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":{"content": text}}]}),
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#
    )
}

async fn connect_provider(addr: std::net::SocketAddr) -> provider_local::LocalAgentProvider {
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "fake-model",
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    provider
}

async fn drain_run(stream: &mut agent_core::provider::EventStream) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        let done = matches!(&ev, AgentEvent::RunFinished { .. });
        events.push(ev);
        if done {
            break;
        }
    }
    events
}

/// A giant accepted tool result reaches the next model request byte-for-byte.
#[tokio::test]
async fn giant_tool_output_is_preserved_before_the_next_model_call() {
    let dir = tempfile::tempdir().unwrap();
    let big: String = (0..8_000)
        .map(|i| format!("row {i} {}\n", "x".repeat(10)))
        .collect();
    assert!(big.len() > 100_000);
    std::fs::write(dir.path().join("big.txt"), &big).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![
            tool_call_sse("c1", "read_file", json!({"path": "big.txt"})),
            final_body(),
        ],
    ));

    let mut provider = connect_provider(addr).await;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(&session.id, PromptInput::text("read big.txt"))
        .await
        .unwrap();
    drain_run(&mut stream).await;

    let captured = serve_handle.await.unwrap();
    let second_request = request_json(&captured[1]);
    let tool_result = second_request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool")
        .expect("second request carries the tool result")["content"]
        .as_str()
        .expect("tool result content is text");
    let expected = big
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>6}\t{line}\n", index + 1))
        .collect::<String>();
    assert_eq!(tool_result, expected);
}

/// A user message sent while the run is active lands INSIDE the run (between
/// tool batches through live steering, not after it finishes.
#[tokio::test]
async fn steering_message_is_injected_into_the_active_run() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let slow_command = if cfg!(windows) {
        "Start-Sleep -Seconds 1"
    } else {
        "sleep 1"
    };
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![
            // A slow tool keeps the run alive while the steer lands.
            tool_call_sse("c1", "bash", json!({"command": slow_command})),
            final_body(),
        ],
    ));

    let mut provider = connect_provider(addr).await;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(&session.id, PromptInput::text("wait a moment"))
        .await
        .unwrap();

    let mut steered = false;
    let mut approved = false;
    let mut finished = false;
    while let Some(ev) = stream.next().await {
        match &ev {
            AgentEvent::ToolCall { .. } if !steered => {
                steered = true;
                provider
                    .steer(&session.id, PromptInput::text("ALSO CHECK THE README"))
                    .await
                    .expect("active run accepts steering");
            }
            AgentEvent::PermissionRequest { request } => {
                assert_eq!(request.risk.as_deref(), Some("safe"));
                provider
                    .respond(
                        &session.id,
                        ClientResponse::Permission {
                            request: request.id.clone(),
                            option: "allow_once".into(),
                            feedback: None,
                        },
                    )
                    .await
                    .expect("Ask mode command approval resolves");
                approved = true;
            }
            AgentEvent::RunFinished { .. } => {
                finished = true;
                break;
            }
            _ => {}
        }
    }
    assert!(steered && approved && finished);

    let captured = serve_handle.await.unwrap();
    let second = String::from_utf8_lossy(&captured[1]).to_string();
    assert!(
        second.contains("ALSO CHECK THE README"),
        "the steered message must reach the model inside the same run"
    );

    // With no active run, steering is refused so callers fall back to a
    // normal follow-up message.
    let refused = provider
        .steer(&session.id, PromptInput::text("too late"))
        .await;
    assert!(refused.is_err());
}

/// A provider context-window rejection no longer kills the run: agent-loop's
/// checkpoint compactor's overflow-recovery hook force-compacts the live
/// transcript and retries the same call, transparently, and the run finishes.
#[tokio::test]
async fn context_overflow_recovers_by_compacting_and_continuing() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![
            // 1) the turn's first model call is rejected for context size,
            overflow_error_body(),
            // 2) the compaction summarizer runs,
            plain_text_body("SUMMARY: the user wants a haiku about databases."),
            // 3) the same turn continues on the compacted transcript.
            final_body(),
        ],
    ));

    let mut provider = connect_provider(addr).await;
    // Seed a genuinely large prior exchange so compaction has something to
    // shrink (compacting a one-message transcript would only ADD a summary —
    // the recovery's no-progress guard correctly refuses that). The big
    // assistant turn is what gets folded away.
    let big_assistant = "In prior work I explored the schema at length. ".repeat(3_000);
    let resume = agent_core::provider::ResumeTranscript {
        truncated: false,
        items: vec![
            agent_core::provider::ResumeItem::Message {
                role: agent_core::domain::Role::User,
                blocks: vec![agent_core::domain::ContentBlock::text(
                    "earlier: design the DB",
                )],
            },
            agent_core::provider::ResumeItem::Message {
                role: agent_core::domain::Role::Agent,
                blocks: vec![agent_core::domain::ContentBlock::text(big_assistant)],
            },
        ],
    };
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: Some(resume),
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("write a haiku about databases"),
        )
        .await
        .unwrap();
    let events = drain_run(&mut stream).await;

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
        )),
        "the run must finish cleanly after recovery: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::MessageChunk {
                role: agent_core::domain::Role::System,
                ..
            }
        )),
        "the recovery must be visible in the conversation"
    );

    let captured = serve_handle.await.unwrap();
    assert_eq!(captured.len(), 3, "overflow → summarize → continue");
    let retry = String::from_utf8_lossy(&captured[2]).to_string();
    assert!(
        retry.contains("compacted transcript handoff"),
        "the retried call must run on the compacted transcript"
    );
    assert!(
        retry.contains("haiku about databases"),
        "the user's request survives compaction"
    );
}

/// Two read-only tool calls in one assistant turn execute as one batch and
/// both results come back, in emission order (the parallel execution mode).
#[tokio::test]
async fn parallel_read_batch_returns_both_results_in_order() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "alpha contents").unwrap();
    std::fs::write(dir.path().join("b.txt"), "bravo contents").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![
            two_tool_calls_sse(
                "read_file",
                json!({"path": "a.txt"}),
                json!({"path": "b.txt"}),
            ),
            final_body(),
        ],
    ));

    let mut provider = connect_provider(addr).await;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(&session.id, PromptInput::text("read both files"))
        .await
        .unwrap();
    let events = drain_run(&mut stream).await;
    assert!(events.iter().any(|e| matches!(
        e,
        AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
    )));

    let captured = serve_handle.await.unwrap();
    let second = String::from_utf8_lossy(&captured[1]).to_string();
    assert!(second.contains("alpha contents"));
    assert!(second.contains("bravo contents"));
    let a = second.find("par_a").expect("first result present");
    let b = second.find("par_b").expect("second result present");
    assert!(a < b, "results keep tool-call emission order");
}

fn final_body_with_usage(text: &str, prompt_tokens: u64, completion_tokens: u64) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":{"content": text}}]}),
        json!({
            "choices":[{"delta":{},"finish_reason":"stop"}],
            "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": completion_tokens}
        })
    )
}

/// The full goal lifecycle against the real engine: `create_goal` starts the
/// autonomy loop, the engine launches a continuation turn carrying the
/// objective + budget, the model does real work in it (a gated write), marks
/// the goal complete, and the run ends instead of continuing forever.
#[tokio::test]
async fn goal_mode_continues_the_run_until_update_goal_complete() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![
            // Goal tools are deferred until the model searches for them.
            tool_call_sse("g0", "tool_search", json!({"query": "goal autonomy"})),
            // Turn 1: the model creates the goal…
            tool_call_sse(
                "g1",
                "create_goal",
                json!({"objective": "hello.txt must exist containing exactly HELLO"}),
            ),
            // …and ends its turn with a status line (natural stop).
            plain_text_body("Goal created — starting."),
            // Continuation turn 1 (engine-launched): do the actual work…
            tool_call_sse(
                "g2",
                "write_file",
                json!({"path": "hello.txt", "content": "HELLO"}),
            ),
            // …verify + mark the goal complete…
            tool_call_sse("g3", "update_goal", json!({"status": "complete"})),
            // …and deliver the final answer.
            final_body(),
            // A later ordinary conversation turn must not inherit the goal.
            text_body("Happy to help with the follow-up."),
        ],
    ));

    let mut provider = connect_provider(addr).await;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("keep going until hello.txt exists — make it a goal"),
        )
        .await
        .unwrap();

    let mut goal_notes = 0;
    let mut typed_goal_complete = false;
    let mut finished = None;
    while let Some(ev) = stream.next().await {
        match &ev {
            AgentEvent::PermissionRequest { request } => {
                provider
                    .respond(
                        &session.id,
                        ClientResponse::Permission {
                            request: request.id.clone(),
                            option: "allow_once".into(),
                            feedback: None,
                        },
                    )
                    .await
                    .unwrap();
            }
            AgentEvent::MessageChunk {
                role: agent_core::domain::Role::System,
                ..
            } => goal_notes += 1,
            AgentEvent::GoalUpdated { goal, .. } => {
                typed_goal_complete |= goal.status == GoalStatus::Complete;
            }
            AgentEvent::RunFinished { outcome, .. } => {
                finished = Some(outcome.status);
                break;
            }
            _ => {}
        }
    }

    assert_eq!(finished, Some(RunStatus::Done));
    assert!(
        typed_goal_complete,
        "the provider emits the terminal typed goal state"
    );
    assert!(goal_notes >= 1, "the continuation must be visible");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "HELLO",
        "the goal's work happened in the continuation turn"
    );

    let mut follow_up = provider
        .prompt(&session.id, PromptInput::text("Thanks — one more question"))
        .await
        .unwrap();
    let follow_up_events = drain_run(&mut follow_up).await;
    assert!(
        follow_up_events
            .iter()
            .all(|event| !matches!(event, AgentEvent::GoalUpdated { .. })),
        "a completed goal is not reassigned to an ordinary follow-up run"
    );

    let captured = serve_handle.await.unwrap();
    assert_eq!(
        captured.len(),
        7,
        "discovery + one user turn + one continuation + one follow-up"
    );
    let initial_tools = request_json(&captured[0])["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(initial_tools.contains(&"tool_search".to_string()));
    assert!(!initial_tools.contains(&"create_goal".to_string()));
    let discovered_tools = request_json(&captured[1])["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(discovered_tools.contains(&"create_goal".to_string()));
    assert!(discovered_tools.contains(&"update_goal".to_string()));

    let continuation = String::from_utf8_lossy(&captured[3]).to_string();
    assert!(
        continuation.contains("goal continuation turn 1"),
        "the engine-launched turn carries the continuation reminder"
    );
    assert!(continuation.contains("hello.txt must exist containing exactly HELLO"));
    assert!(continuation.contains("audit EVERY explicit requirement"));
    let after_complete = String::from_utf8_lossy(&captured[5]).to_string();
    assert!(
        after_complete.contains("Goal marked complete"),
        "the model sees the completion confirmation"
    );
    let follow_up_request = String::from_utf8_lossy(&captured[6]).to_string();
    assert!(follow_up_request.contains("Thanks — one more question"));
}

#[tokio::test]
async fn goal_mode_continues_beyond_the_previous_24_turn_limit() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut bodies = vec![
        tool_call_sse("g0", "tool_search", json!({"query": "goal autonomy"})),
        tool_call_sse(
            "g1",
            "create_goal",
            json!({"objective": "continue until the twenty-fifth goal turn"}),
        ),
        plain_text_body("Goal created — starting."),
    ];
    bodies.extend(
        (1..=24).map(|turn| plain_text_body(&format!("Continuation {turn} made progress."))),
    );
    bodies.push(tool_call_sse(
        "g25",
        "update_goal",
        json!({"status": "complete"}),
    ));
    bodies.push(text_body("Completed on continuation 25."));
    let serve_handle = tokio::spawn(serve(listener, bodies));

    let mut provider = connect_provider(addr).await;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("make this a goal and continue for twenty-five goal turns"),
        )
        .await
        .unwrap();
    let events = drain_run(&mut stream).await;

    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::GoalUpdated { goal, .. }
            if goal.status == GoalStatus::Complete && goal.continuations >= 25
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
    )));

    let captured = serve_handle.await.unwrap();
    assert_eq!(captured.len(), 29);
    assert!(
        String::from_utf8_lossy(&captured[27]).contains("goal continuation turn 25"),
        "the engine must launch the turn beyond the former 24-continuation cap"
    );
}

/// A goal with a token budget gets exactly one wrap-up turn once usage crosses
/// the budget, then the run stops — no infinite autonomy.
#[tokio::test]
async fn goal_budget_exhaustion_triggers_one_wrapup_turn_then_stops() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_handle = tokio::spawn(serve(
        listener,
        vec![
            tool_call_sse("g0", "tool_search", json!({"query": "goal autonomy"})),
            tool_call_sse(
                "g1",
                "create_goal",
                json!({"objective": "an endless task", "token_budget": 10}),
            ),
            // Ending the first turn reports usage far over the 10-token budget.
            final_body_with_usage("Working on it.", 500, 100),
            // The engine's ONE wrap-up turn.
            final_body_with_usage("Out of budget — here is where things stand.", 200, 50),
        ],
    ));

    let mut provider = connect_provider(addr).await;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("pursue this as a goal with a 10 token budget"),
        )
        .await
        .unwrap();
    let events = drain_run(&mut stream).await;

    let live_usage = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::RunUsageUpdated { usage, .. } => Some(*usage),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        live_usage
            .iter()
            .map(|usage| (usage.input_tokens, usage.output_tokens))
            .collect::<Vec<_>>(),
        vec![(500, 100), (700, 150)],
        "usage is published cumulatively after each model call"
    );
    let finished_index = events
        .iter()
        .position(|event| matches!(event, AgentEvent::RunFinished { .. }))
        .expect("run finishes");
    let last_usage_index = events
        .iter()
        .rposition(|event| matches!(event, AgentEvent::RunUsageUpdated { .. }))
        .expect("live usage event");
    assert!(
        last_usage_index < finished_index,
        "the final live usage update precedes the terminal outcome"
    );

    assert!(events.iter().any(|e| matches!(
        e,
        AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
    )));

    let captured = serve_handle.await.unwrap();
    assert_eq!(
        captured.len(),
        4,
        "discovery + user turn + exactly one budget wrap-up turn"
    );
    let wrapup = String::from_utf8_lossy(&captured[3]).to_string();
    assert!(
        wrapup.contains("goal budget exhausted"),
        "the wrap-up turn carries the budget-limit reminder"
    );
    assert!(wrapup.contains("Do not start new substantive work"));
}
