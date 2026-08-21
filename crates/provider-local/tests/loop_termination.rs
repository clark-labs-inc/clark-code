//! End-to-end termination regressions for the production local-agent engine.
//!
//! These use a deterministic OpenAI-compatible endpoint. They exercise the
//! provider composition root rather than only the underlying `agent-loop`
//! loop, so production builder defaults and completion plugins are in scope.

use agent_core::domain::{AgentEvent, ContentBlock, GoalStatus, RunFailureKind, RunStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn tool_call_body(call_id: &str, name: &str, args: Value) -> String {
    let arguments = serde_json::to_string(&args).expect("tool arguments serialize");
    [
        format!(
            "data: {}",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": call_id,
                            "function": {"name": name, "arguments": arguments}
                        }]
                    }
                }]
            })
        ),
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}".to_string(),
        "data: [DONE]".to_string(),
        String::new(),
    ]
    .join("\n\n")
}

fn tool_call_body_with_usage(
    call_id: &str,
    name: &str,
    args: Value,
    prompt_tokens: u64,
    completion_tokens: u64,
) -> String {
    let arguments = serde_json::to_string(&args).expect("tool arguments serialize");
    [
        format!(
            "data: {}",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": call_id,
                            "function": {"name": name, "arguments": arguments}
                        }]
                    }
                }]
            })
        ),
        format!(
            "data: {}",
            json!({
                "choices": [{"delta": {}, "finish_reason": "tool_calls"}],
                "usage": {
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": completion_tokens,
                    "total_tokens": prompt_tokens + completion_tokens
                }
            })
        ),
        "data: [DONE]".to_string(),
        String::new(),
    ]
    .join("\n\n")
}

fn text_body(text: &str) -> String {
    tool_call_body("final-answer", "final_answer", json!({"content": text}))
}

fn plain_text_body(text: &str) -> String {
    [
        format!(
            "data: {}",
            json!({"choices": [{"delta": {"content": text}}]})
        ),
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}".to_string(),
        "data: [DONE]".to_string(),
        String::new(),
    ]
    .join("\n\n")
}

fn reasoning_body_with_usage(text: &str, prompt_tokens: u64, completion_tokens: u64) -> String {
    [
        format!(
            "data: {}",
            json!({"choices": [{"delta": {"reasoning": text}}]})
        ),
        format!(
            "data: {}",
            json!({
                "choices": [{"delta": {}, "finish_reason": "stop"}],
                "usage": {
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": completion_tokens,
                    "total_tokens": prompt_tokens + completion_tokens
                }
            })
        ),
        "data: [DONE]".to_string(),
        String::new(),
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

async fn read_request(socket: &mut TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut content_length = None;
    loop {
        let count = socket.read(&mut buffer).await.expect("read request");
        if count == 0 {
            return request;
        }
        request.extend_from_slice(&buffer[..count]);
        let headers_end = request.windows(4).position(|bytes| bytes == b"\r\n\r\n");
        if content_length.is_none() {
            content_length = headers_end.and_then(|end| {
                String::from_utf8_lossy(&request[..end])
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
            });
        }
        if let (Some(end), Some(length)) = (headers_end, content_length) {
            if request.len() >= end + 4 + length {
                return request;
            }
        }
    }
}

async fn serve(listener: TcpListener, bodies: Vec<String>) -> Vec<Vec<u8>> {
    serve_responses(
        listener,
        bodies
            .into_iter()
            .map(|body| http_response(&body))
            .collect(),
    )
    .await
}

async fn serve_responses(listener: TcpListener, responses: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut requests = Vec::with_capacity(responses.len());
    for response in responses {
        let (mut socket, _) = listener.accept().await.expect("accept model request");
        requests.push(read_request(&mut socket).await);
        socket
            .write_all(&response)
            .await
            .expect("write model response");
        socket.flush().await.expect("flush model response");
    }
    requests
}

fn error_response(status: u16, message: &str) -> Vec<u8> {
    let body = json!({
        "error": {"message": message, "type": "invalid_request_error"}
    })
    .to_string();
    format!(
        "HTTP/1.1 {status} Bad Request\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

async fn connect(
    address: std::net::SocketAddr,
    root: &std::path::Path,
    extra: Value,
) -> provider_local::LocalAgentProvider {
    let mut base = json!({
        "base_url": format!("http://{address}/v1"),
        "model": "deterministic-stress-model",
        "memories": false,
        "sandbox_mode": "disabled"
    });
    base.as_object_mut()
        .expect("base config object")
        .extend(extra.as_object().cloned().unwrap_or_default());
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            cwd: Some(root.to_string_lossy().into_owned()),
            auth_token: Some("test-key".to_string()),
            extra: base,
            ..ProviderConfig::default()
        })
        .await
        .expect("connect provider");
    provider
}

async fn new_session(
    provider: &mut provider_local::LocalAgentProvider,
    root: &std::path::Path,
) -> agent_core::provider::Session {
    provider
        .new_session(SessionOptions {
            cwd: Some(root.to_string_lossy().into_owned()),
            ..SessionOptions::default()
        })
        .await
        .expect("create session")
}

async fn terminal_outcome(
    provider: &mut provider_local::LocalAgentProvider,
    session: &agent_core::provider::Session,
    prompt: &str,
) -> (agent_core::domain::RunOutcome, usize) {
    let mut events = provider
        .prompt(&session.id, PromptInput::text(prompt))
        .await
        .expect("start prompt");
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let mut tool_calls = 0;
        while let Some(event) = events.next().await {
            match event {
                AgentEvent::ToolCall { .. } => tool_calls += 1,
                AgentEvent::RunFinished { outcome, .. } => return (outcome, tool_calls),
                _ => {}
            }
        }
        panic!("event stream closed without RunFinished");
    })
    .await
    .expect("run reached a terminal boundary")
}

#[tokio::test]
async fn structured_final_answer_is_terminal_after_one_model_response() {
    let root = tempfile::tempdir().expect("temporary project");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind model endpoint");
    let address = listener.local_addr().expect("model endpoint address");
    let server = tokio::spawn(serve(
        listener,
        vec![text_body("ORDINARY_COMPLETION_STOPS_HERE")],
    ));
    let mut provider = connect(address, root.path(), json!({})).await;
    let session = new_session(&mut provider, root.path()).await;

    let (outcome, tool_calls) = terminal_outcome(
        &mut provider,
        &session,
        "Reply with the completion sentinel and then stop.",
    )
    .await;

    assert_eq!(outcome.status, RunStatus::Done, "{outcome:?}");
    assert_eq!(outcome.failure_kind, None, "{outcome:?}");
    assert_eq!(tool_calls, 0);
    let requests = server.await.expect("model server task");
    assert_eq!(
        requests.len(),
        1,
        "the typed final-answer tool is the natural completion boundary"
    );
}

#[tokio::test]
async fn one_failed_run_does_not_override_the_goal_blocker_contract() {
    let root = tempfile::tempdir().expect("temporary project");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind model endpoint");
    let address = listener.local_addr().expect("model endpoint address");
    let server = tokio::spawn(serve_responses(
        listener,
        vec![
            http_response(&tool_call_body(
                "discover-goal",
                "tool_search",
                json!({"query": "goal autonomy"}),
            )),
            http_response(&tool_call_body(
                "create-goal",
                "create_goal",
                json!({"objective": "finish durable work"}),
            )),
            http_response(&plain_text_body("Goal created; continuing.")),
            error_response(400, "deterministic provider rejection"),
        ],
    ));
    let mut provider = connect(address, root.path(), json!({})).await;
    let session = new_session(&mut provider, root.path()).await;
    let mut events = provider
        .prompt(
            &session.id,
            PromptInput::text("Create a standing goal and finish durable work."),
        )
        .await
        .expect("start prompt");

    let mut last_goal = None;
    let mut outcome = None;
    while let Some(event) = events.next().await {
        match event {
            AgentEvent::GoalUpdated { goal, .. } => last_goal = Some(goal),
            AgentEvent::RunFinished {
                outcome: finished, ..
            } => {
                outcome = Some(finished);
                break;
            }
            _ => {}
        }
    }

    let outcome = outcome.expect("run finished");
    assert_eq!(outcome.status, RunStatus::Failed);
    assert_eq!(outcome.failure_kind, Some(RunFailureKind::ProviderError));
    let goal = last_goal.expect("typed goal state");
    assert_eq!(goal.status, GoalStatus::Active);
    assert_eq!(goal.blocker_reason, None);
    assert_eq!(server.await.expect("model server task").len(), 4);
}

#[tokio::test]
async fn required_tool_contract_violation_gets_one_isolated_repair() {
    const DISCARDED: &str = "I am done, but I forgot the final-answer tool.";
    const DELIVERED: &str = "The structured final answer was repaired.";
    let root = tempfile::tempdir().expect("temporary project");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind model endpoint");
    let address = listener.local_addr().expect("model endpoint address");
    let server = tokio::spawn(serve(
        listener,
        vec![
            reasoning_body_with_usage(DISCARDED, 100, 5),
            tool_call_body_with_usage(
                "final-answer",
                "final_answer",
                json!({"content": DELIVERED}),
                110,
                6,
            ),
        ],
    ));
    let mut provider = connect(address, root.path(), json!({})).await;
    let session = new_session(&mut provider, root.path()).await;
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Return a structured answer."),
        )
        .await
        .expect("start prompt");
    let mut visible_text = String::new();
    let mut visible_thinking = String::new();
    let outcome = loop {
        match stream.next().await.expect("run reaches terminal event") {
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => visible_text.push_str(&text),
            AgentEvent::MessageChunk {
                delta: ContentBlock::Thinking { text },
                ..
            } => visible_thinking.push_str(&text),
            AgentEvent::RunFinished { outcome, .. } => break outcome,
            _ => {}
        }
    };

    assert_eq!(outcome.status, RunStatus::Done, "{outcome:?}");
    let usage = outcome.usage.expect("repair usage is retained");
    assert_eq!(usage.input_tokens, 210, "{outcome:?}");
    assert_eq!(usage.output_tokens, 11, "{outcome:?}");
    assert_eq!(usage.context_tokens, 110, "{outcome:?}");
    assert!(!visible_text.contains(DISCARDED), "{visible_text}");
    assert!(visible_thinking.contains(DISCARDED), "{visible_thinking}");
    assert!(visible_text.contains(DELIVERED), "{visible_text}");
    let requests = server.await.expect("model server task");
    assert_eq!(requests.len(), 2);
    let request_text = requests
        .iter()
        .map(|request| String::from_utf8_lossy(request))
        .collect::<Vec<_>>();
    for request in &requests {
        assert!(
            String::from_utf8_lossy(request).contains(r#""tool_choice":"required""#),
            "required tool choice missing"
        );
    }
    let idempotency_key = |request: &str| {
        request.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("idempotency-key")
                .then(|| value.trim().to_string())
        })
    };
    assert!(idempotency_key(&request_text[0]).is_none());
    assert!(idempotency_key(&request_text[1]).is_none());
    assert!(
        request_text[1]
            .contains("previous response violated the required structured-tool boundary"),
        "repair request lacked a precise contract correction"
    );
}

#[tokio::test]
async fn unstructured_provider_output_is_quarantined_before_visible_history() {
    const MALFORMED: &str = "?: yes please -> @9ff4... drawn.";
    let root = tempfile::tempdir().expect("temporary project");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind model endpoint");
    let address = listener.local_addr().expect("model endpoint address");
    let server = tokio::spawn(serve(
        listener,
        vec![
            plain_text_body(MALFORMED),
            plain_text_body(MALFORMED),
            plain_text_body(MALFORMED),
        ],
    ));
    let mut provider = connect(address, root.path(), json!({})).await;
    let session = new_session(&mut provider, root.path()).await;
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Return a structured answer."),
        )
        .await
        .expect("start prompt");
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        let done = matches!(event, AgentEvent::RunFinished { .. });
        events.push(event);
        if done {
            break;
        }
    }

    assert!(!events.iter().any(|event| matches!(
        event,
        AgentEvent::MessageChunk {
            delta: ContentBlock::Text { text },
            ..
        } if text.contains(MALFORMED)
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::Trace { source, .. }
            if source == "provider_output_contract_violation"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::RunFinished { outcome, .. }
            if outcome.status == RunStatus::Failed
                && outcome.failure_kind == Some(RunFailureKind::ProviderError)
    )));
    let requests = server.await.expect("model server task");
    assert_eq!(requests.len(), 3);
    assert!(
        String::from_utf8_lossy(&requests[1])
            .contains("previous response violated the required structured-tool boundary"),
        "second provider request was not an isolated contract repair"
    );
    let final_request = String::from_utf8_lossy(&requests[2]);
    let final_body: Value = serde_json::from_str(
        final_request
            .split_once("\r\n\r\n")
            .expect("HTTP request body")
            .1,
    )
    .expect("final recovery JSON body");
    assert_eq!(
        final_body["tool_choice"],
        json!({
            "type": "function",
            "function": { "name": "final_answer" },
        }),
        "final recovery request did not pin the typed delivery tool"
    );
}

#[tokio::test]
async fn productive_turn_runs_past_128_model_tool_iterations_without_global_cap() {
    const PRODUCTIVE_STEPS: usize = 160;
    let root = tempfile::tempdir().expect("temporary project");
    let mut bodies = Vec::with_capacity(PRODUCTIVE_STEPS + 1);
    for index in 0..PRODUCTIVE_STEPS {
        let path = format!("step-{index:03}.txt");
        std::fs::write(root.path().join(&path), format!("receipt-{index:03}"))
            .expect("write step fixture");
        bodies.push(tool_call_body(
            &format!("read-{index:03}"),
            "read_file",
            json!({"path": path}),
        ));
    }
    bodies.push(text_body("LONG_PRODUCTIVE_RUN_COMPLETE"));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind model endpoint");
    let address = listener.local_addr().expect("model endpoint address");
    let server = tokio::spawn(serve(listener, bodies));
    let mut provider = connect(address, root.path(), json!({})).await;
    let session = new_session(&mut provider, root.path()).await;

    let (outcome, tool_calls) = terminal_outcome(
        &mut provider,
        &session,
        "Read every numbered fixture in order and then return the completion sentinel.",
    )
    .await;

    assert_eq!(outcome.status, RunStatus::Done, "{outcome:?}");
    assert_eq!(outcome.failure_kind, None, "{outcome:?}");
    assert_eq!(tool_calls, PRODUCTIVE_STEPS);
    assert_eq!(
        outcome
            .execution
            .as_ref()
            .expect("execution receipt")
            .completed_tools,
        ["read_file"]
    );
    let requests = server.await.expect("model server task");
    assert_eq!(requests.len(), PRODUCTIVE_STEPS + 1);
}

#[tokio::test]
async fn unresolved_effect_blocks_final_answer_until_canonical_verification() {
    let root = tempfile::tempdir().expect("temporary project");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind model endpoint");
    let address = listener.local_addr().expect("model endpoint address");
    let bodies = vec![
        tool_call_body(
            "external-mutation",
            "bash",
            json!({
                "command": "touch external-marker.txt",
                "sandbox_permissions": "require_escalated",
                "effect": "create",
                "effect_target": "external-marker.txt"
            }),
        ),
        text_body("This premature answer must be rejected."),
        tool_call_body(
            "canonical-readback",
            "bash",
            json!({
                "command": "test -f external-marker.txt && printf present",
                "effect": "none"
            }),
        ),
        tool_call_body(
            "verification",
            "verify_effect",
            json!({
                "effect_id": "external-mutation",
                "status": "verified",
                "evidence": "Canonical read-back observed the marker as present.",
                "expected": "present",
                "observed": "present"
            }),
        ),
        text_body("Created the marker and verified canonical read-back."),
    ];
    let server = tokio::spawn(serve(listener, bodies));
    let mut provider = connect(
        address,
        root.path(),
        json!({"permissions": {"bash": "allow"}}),
    )
    .await;
    let session = new_session(&mut provider, root.path()).await;

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text(
                "Create the external marker and verify its canonical state before finishing.",
            ),
        )
        .await
        .expect("start prompt");
    let mut tool_calls = 0;
    let mut visible_text = Vec::new();
    let outcome = loop {
        match stream.next().await.expect("run reaches terminal event") {
            AgentEvent::ToolCall { .. } => tool_calls += 1,
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => visible_text.push(text),
            AgentEvent::RunFinished { outcome, .. } => break outcome,
            _ => {}
        }
    };

    assert_eq!(tool_calls, 3);
    assert_eq!(outcome.status, RunStatus::Done, "{outcome:?}");
    assert_eq!(outcome.failure_kind, None, "{outcome:?}");
    assert!(
        !visible_text
            .iter()
            .any(|text| text.contains("premature answer")),
        "unverified final answer reached the UI: {visible_text:?}"
    );
    assert!(visible_text
        .iter()
        .any(|text| text.contains("verified canonical read-back")));

    let requests = server.await.expect("model server task");
    assert_eq!(requests.len(), 5);
    let first_follow_up = String::from_utf8_lossy(&requests[1]);
    assert!(
        first_follow_up.contains("verify_effect"),
        "verification resolver was not automatically exposed"
    );
    assert!(
        first_follow_up.contains(r#""tool_choice":"required""#),
        "the provider request did not require a structured tool boundary"
    );
}
