//! End-to-end test of the local agent loop against a fake OpenAI-compatible
//! streaming endpoint. Drives `LocalAgentProvider` through one full round-trip:
//! the model asks to read a real local file, the loop executes the tool on disk,
//! feeds the result back, and the model returns a final answer.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use agent_core::domain::{AgentEvent, ContentBlock, PendingUpload, RunStatus, ToolStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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

/// SSE body for the second model call: the final answer.
fn final_body() -> String {
    [
        r#"data: {"choices":[{"delta":{"content":"The file says: "}}]}"#,
        r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
        "",
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
    let calls = Arc::new(AtomicUsize::new(0));
    let mut captured = Vec::with_capacity(bodies.len());
    for _ in 0..bodies.len() {
        let Ok((mut sock, _)) = listener.accept().await else {
            break;
        };
        captured.push(read_request(&mut sock).await);
        let n = calls.fetch_add(1, Ordering::SeqCst);
        let _ = sock.write_all(&http_response(&bodies[n])).await;
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
                "memories": false
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
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
        AgentEvent::ToolCall { call, .. } if call.title.contains("read_file") => Some(call),
        _ => None,
    });
    assert!(
        read_tool.is_some(),
        "expected a read_file tool call: {events:?}"
    );

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
}

#[tokio::test]
async fn local_loop_auto_compacts_large_transcript_before_sampling() {
    let dir = tempfile::tempdir().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, vec![compact_summary_body(), final_body()]));

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": format!("http://{addr}/v1"),
                "model": "fake-model",
                "memories": false,
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
                "memories": false
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
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
    while let Some(ev) = stream.next().await {
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
                "memories": false
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let session = provider
        .new_session(SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
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

    // First request: the vision fallback, hitting the default agentic model
    // with BOTH images batched into one content-parts array.
    let vision_req = request_json(&captured[0]);
    assert_eq!(vision_req["model"], "clark");
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
