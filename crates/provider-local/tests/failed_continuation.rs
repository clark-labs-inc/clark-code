//! Continuation regressions for runs that stop after completing useful work.

use agent_core::domain::{AgentEvent, RunStatus};
use agent_core::ids::RunId;
use agent_core::provider::{PromptInput, Provider, ProviderConfig, Session, SessionOptions};
use agent_orchestration::{ExecutionEvent, ExecutionLedger, ExecutionState};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

fn tool_call_body() -> String {
    [
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":\"hello.txt\"}"}}]}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n")
}

fn final_body() -> String {
    [
        r#"data: {"choices":[{"delta":{"content":"continued"}}]}"#,
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
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn request_json(raw: &[u8]) -> Value {
    let headers_end = find_headers_end(raw).expect("captured request has headers");
    serde_json::from_slice(&raw[headers_end + 4..]).expect("request body is valid JSON")
}

fn request_header<'a>(raw: &'a [u8], expected: &str) -> Option<&'a str> {
    let headers_end = find_headers_end(raw)?;
    std::str::from_utf8(&raw[..headers_end])
        .ok()?
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(expected).then(|| value.trim())
        })
}

async fn new_provider(
    addr: std::net::SocketAddr,
    root: &std::path::Path,
) -> (provider_local::LocalAgentProvider, Session) {
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
            cwd: Some(root.to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .unwrap();
    (provider, session)
}

async fn run_status(stream: agent_core::provider::EventStream) -> RunStatus {
    futures::pin_mut!(stream);
    while let Some(event) = stream.next().await {
        if let AgentEvent::RunFinished { outcome, .. } = event {
            return outcome.status;
        }
    }
    panic!("run ended without RunFinished");
}

async fn run_events(stream: agent_core::provider::EventStream) -> Vec<AgentEvent> {
    futures::pin_mut!(stream);
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        let finished = matches!(event, AgentEvent::RunFinished { .. });
        events.push(event);
        if finished {
            break;
        }
    }
    events
}

fn assert_continuation_context(raw: &[u8]) {
    let request = request_json(raw);
    let messages = request["messages"]
        .as_array()
        .expect("follow-up request has messages");
    assert!(
        messages.iter().any(|message| {
            message["role"] == "user"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("CONTEXT_SENTINEL_2048"))
        }),
        "follow-up lost the original request: {messages:?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["role"] == "assistant"
                && message["tool_calls"].as_array().is_some_and(|calls| {
                    calls
                        .iter()
                        .any(|call| call["function"]["name"] == "read_file")
                })
        }) && messages.iter().any(|message| {
            message["role"] == "tool"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("hi"))
        }),
        "follow-up lost the completed assistant/tool pair: {messages:?}"
    );
    assert!(
        messages.last().is_some_and(|message| {
            message["role"] == "user"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.ends_with("continue"))
        }),
        "follow-up lost the new prompt: {messages:?}"
    );
}

fn assert_transparent_retry_context(raw: &[u8]) {
    let request = request_json(raw);
    let messages = request["messages"]
        .as_array()
        .expect("recovery request has messages");
    assert!(
        messages.iter().any(|message| {
            message["role"] == "assistant"
                && message["tool_calls"].as_array().is_some_and(|calls| {
                    calls
                        .iter()
                        .any(|call| call["function"]["name"] == "read_file")
                })
        }) && messages.iter().any(|message| {
            message["role"] == "tool"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("hi"))
        }),
        "transparent retry lost the completed assistant/tool pair: {messages:?}"
    );
    assert!(
        !messages.iter().any(|message| {
            message["content"]
                .as_str()
                .is_some_and(|content| content.contains("runtime recovery"))
        }),
        "request-local retry must not synthesize a whole-run recovery marker: {messages:?}"
    );
}

#[tokio::test]
async fn transient_failure_retries_the_same_model_turn_from_a_completed_tool_boundary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();

        let (mut first, _) = listener.accept().await.unwrap();
        requests.push(read_request(&mut first).await);
        first
            .write_all(&http_response(&tool_call_body()))
            .await
            .unwrap();

        // End the connection before HTTP headers. No model output exists for
        // this request, so transport can replay it without restarting the
        // whole agent attempt or repeating the completed tool.
        let (mut interrupted, _) = listener.accept().await.unwrap();
        requests.push(read_request(&mut interrupted).await);
        drop(interrupted);

        let (mut recovered, _) = listener.accept().await.unwrap();
        requests.push(read_request(&mut recovered).await);
        recovered
            .write_all(&http_response(&final_body()))
            .await
            .unwrap();
        requests
    });
    let (mut provider, session) = new_provider(addr, dir.path()).await;

    let stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Read hello.txt. CONTEXT_SENTINEL_2048"),
        )
        .await
        .unwrap();
    let events = run_events(stream).await;

    let outcome = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::RunFinished { outcome, .. } => Some(outcome),
            _ => None,
        })
        .expect("run has a terminal outcome");
    assert_eq!(outcome.status, RunStatus::Done, "events: {events:#?}");
    let execution = outcome.execution.as_ref().expect("root execution summary");
    assert_eq!(execution.attempts, 1);
    assert_eq!(execution.recoveries, 0);
    assert_eq!(
        execution
            .completed_tools
            .iter()
            .filter(|name| name.as_str() == "read_file")
            .count(),
        1,
        "a completed tool must not be counted twice"
    );

    let trace = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Trace {
                source, payload, ..
            } if source == "execution_lifecycle" => {
                Some(serde_json::from_value::<ExecutionEvent>(payload.clone()).unwrap())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let replay = ExecutionLedger::replay(&trace).expect("lifecycle trace is replayable");
    assert_eq!(replay.state, ExecutionState::Completed);
    assert_eq!(replay.recoveries, 0);
    assert_eq!(replay.evidence.tools.len(), 1);

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_transparent_retry_context(&requests[2]);
    let interrupted_key = request_header(&requests[1], "idempotency-key")
        .expect("interrupted request has an idempotency key");
    let retried_key =
        request_header(&requests[2], "idempotency-key").expect("retry has an idempotency key");
    assert_eq!(interrupted_key, retried_key);
}

#[tokio::test]
async fn exhausted_transport_retries_update_one_provider_incident_that_settles() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();

        let (mut first, _) = listener.accept().await.unwrap();
        requests.push(read_request(&mut first).await);
        first
            .write_all(&http_response(&tool_call_body()))
            .await
            .unwrap();

        // Initial request plus the three request-local retries all fail before
        // output, exhausting that layer and activating whole-run recovery.
        for _ in 0..4 {
            let (mut interrupted, _) = listener.accept().await.unwrap();
            requests.push(read_request(&mut interrupted).await);
            drop(interrupted);
        }

        let (mut recovered, _) = listener.accept().await.unwrap();
        requests.push(read_request(&mut recovered).await);
        recovered
            .write_all(&http_response(&final_body()))
            .await
            .unwrap();
        requests
    });
    let (mut provider, session) = new_provider(addr, dir.path()).await;

    let stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Read hello.txt. CONTEXT_SENTINEL_2048"),
        )
        .await
        .unwrap();
    let events = run_events(stream).await;

    let incidents = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProviderIncidentUpdated { incident, .. } => Some(incident),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(incidents.len(), 6, "events: {events:#?}");
    assert!(incidents.windows(2).all(|pair| pair[0].id == pair[1].id));
    assert_eq!(
        incidents[0].status,
        agent_core::recovery::ProviderIncidentStatus::Retrying
    );
    assert_eq!(incidents[0].request.attempts, 1);
    assert_eq!(incidents[0].request.retries.transient, 1);
    let recovering = incidents
        .iter()
        .find(|incident| incident.execution_recovery.is_some())
        .expect("execution recovery update");
    let recovery = recovering.execution_recovery.as_ref().unwrap();
    assert_eq!(recovery.attempt, 2);
    assert_eq!(recovery.max_attempts, 2);
    assert_eq!(recovering.request.retries.transient, 3);
    assert_eq!(recovery.boundary.completed_tools, 1);
    assert_eq!(
        recovery.boundary.last_completed_tool_name.as_deref(),
        Some("read_file")
    );
    assert_eq!(
        incidents.last().unwrap().status,
        agent_core::recovery::ProviderIncidentStatus::Recovered
    );
    assert!(incidents.last().unwrap().completed_at_ms.is_some());

    let outcome = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::RunFinished { outcome, .. } => Some(outcome),
            _ => None,
        })
        .expect("run has a terminal outcome");
    assert_eq!(outcome.status, RunStatus::Done);
    let execution = outcome.execution.as_ref().expect("execution summary");
    assert_eq!(execution.attempts, 2);
    assert_eq!(execution.recoveries, 1);

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 6);
    let recovered = request_json(&requests[5]);
    assert!(recovered["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| {
            message["content"]
                .as_str()
                .is_some_and(|content| content.contains("runtime recovery"))
        }));
}

#[tokio::test]
async fn cancelled_turn_preserves_completed_context_for_follow_up() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (waiting_tx, waiting_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();

        let (mut first, _) = listener.accept().await.unwrap();
        requests.push(read_request(&mut first).await);
        first
            .write_all(&http_response(&tool_call_body()))
            .await
            .unwrap();

        let (mut hanging, _) = listener.accept().await.unwrap();
        requests.push(read_request(&mut hanging).await);
        let _ = waiting_tx.send(());
        let _ = release_rx.await;
        drop(hanging);

        let (mut follow_up, _) = listener.accept().await.unwrap();
        requests.push(read_request(&mut follow_up).await);
        follow_up
            .write_all(&http_response(&final_body()))
            .await
            .unwrap();
        requests
    });
    let (mut provider, session) = new_provider(addr, dir.path()).await;

    let first = provider
        .prompt(
            &session.id,
            PromptInput::text("Read hello.txt. CONTEXT_SENTINEL_2048"),
        )
        .await
        .unwrap();
    waiting_rx.await.unwrap();
    provider
        .cancel(&session.id, &RunId::new("run-1"))
        .await
        .unwrap();
    let _ = release_tx.send(());
    assert_eq!(run_status(first).await, RunStatus::Cancelled);

    let follow_up = provider
        .prompt(&session.id, PromptInput::text("continue"))
        .await
        .unwrap();
    assert_eq!(run_status(follow_up).await, RunStatus::Done);

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_continuation_context(&requests[2]);
}
