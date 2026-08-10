//! Regression coverage for model-emitted multi-call turns.
//!
//! Clark Code leaves agent-loop's per-turn call cap unset. Read batches
//! may run in parallel, while any batch containing an exclusive workspace tool
//! is executed sequentially in the exact order emitted by the model.

use agent_core::domain::{AgentEvent, RunStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const CALL_COUNT: usize = 12;

fn write_batch_body() -> String {
    let calls = (0..CALL_COUNT)
        .map(|index| {
            json!({
                "index": index,
                "id": format!("write_{index}"),
                "function": {
                    "name": "write_file",
                    "arguments": json!({
                        "path": "ordered.txt",
                        "content": index.to_string(),
                    }).to_string(),
                },
            })
        })
        .collect::<Vec<_>>();
    [
        format!(
            "data: {}",
            json!({"choices":[{"delta":{"tool_calls":calls}}]})
        ),
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#.to_string(),
        "data: [DONE]".to_string(),
        String::new(),
    ]
    .join("\n\n")
}

fn final_body() -> String {
    let arguments = json!({"content": "done"}).to_string();
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

fn http_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    )
    .into_bytes()
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut body_len = None;
    loop {
        let count = stream.read(&mut buffer).await.unwrap();
        if count == 0 {
            return bytes;
        }
        bytes.extend_from_slice(&buffer[..count]);
        let Some(headers_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        if body_len.is_none() {
            let headers = String::from_utf8_lossy(&bytes[..headers_end]);
            body_len = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
        }
        if body_len.is_some_and(|length| bytes.len() >= headers_end + 4 + length) {
            return bytes;
        }
    }
}

async fn serve(listener: TcpListener) -> Vec<Vec<u8>> {
    let responses = [write_batch_body(), final_body()];
    let mut captured = Vec::new();
    for body in responses {
        let (mut stream, _) = listener.accept().await.unwrap();
        captured.push(read_request(&mut stream).await);
        stream.write_all(&http_response(&body)).await.unwrap();
        stream.flush().await.unwrap();
    }
    captured
}

#[tokio::test]
async fn large_mutating_batch_executes_every_call_sequentially_without_a_turn_cap() {
    let workspace = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(serve(listener));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{address}/v1"),
                "model": "fake-model",
                "memories": false,
                "sandbox_mode": "disabled",
                "permissions": {"write_file": "allow"},
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(workspace.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut run = provider
        .prompt(&session.id, PromptInput::text("apply every ordered write"))
        .await
        .unwrap();

    let mut emitted_tools = Vec::new();
    let mut status = None;
    while let Some(event) = run.next().await {
        match event {
            AgentEvent::ToolCall { call, .. } => {
                if call.tool_name.as_deref() == Some("write_file") {
                    emitted_tools.push(call.id);
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
    assert_eq!(emitted_tools.len(), CALL_COUNT);
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("ordered.txt")).unwrap(),
        (CALL_COUNT - 1).to_string(),
        "exclusive writes must execute in model emission order",
    );

    let captured = server.await.unwrap();
    let follow_up = String::from_utf8_lossy(&captured[1]);
    for index in 0..CALL_COUNT {
        assert!(
            follow_up.contains(&format!("write_{index}")),
            "tool result {index} was omitted from the next model turn",
        );
    }
    assert!(!follow_up.contains("max_tool_calls_per_turn"));
}
