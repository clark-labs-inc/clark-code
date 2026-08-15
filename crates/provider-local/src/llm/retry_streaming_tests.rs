use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

async fn progressive_endpoint(chunks: Vec<(Duration, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body_len = chunks.iter().map(|(_, chunk)| chunk.len()).sum::<usize>();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 16 * 1024];
        let _ = stream.read(&mut request).await;
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {body_len}\r\n\r\n"
        );
        if stream.write_all(headers.as_bytes()).await.is_err() {
            return;
        }
        for (delay, chunk) in chunks {
            tokio::time::sleep(delay).await;
            if stream.write_all(&chunk).await.is_err() || stream.flush().await.is_err() {
                return;
            }
        }
    });
    format!("http://{address}/v1")
}

fn frame(payload: &str) -> Vec<u8> {
    format!("data: {payload}\n\n").into_bytes()
}

#[tokio::test]
async fn publishes_openrouter_text_deltas_as_completed_words() {
    let chunks = vec![
        (
            Duration::ZERO,
            frame(r#"{"choices":[{"delta":{"content":"Clark "}}]}"#),
        ),
        (
            Duration::from_millis(20),
            frame(r#"{"choices":[{"delta":{"content":"streams"}}]}"#),
        ),
        (
            Duration::from_millis(20),
            frame(r#"{"choices":[{"delta":{"content":" smoothly"}}]}"#),
        ),
        (
            Duration::from_millis(20),
            frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
        ),
        (Duration::ZERO, b"data: [DONE]\n\n".to_vec()),
    ];
    let base_url = progressive_endpoint(chunks).await;
    let client = LlmClient::from_parts(&base_url, "fake-model", None, Vec::new(), None).unwrap();
    let mut callbacks = Vec::new();
    let turn = client
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |delta| callbacks.push(delta.to_string()),
            |_| {},
        )
        .await
        .unwrap();

    assert_eq!(turn.text, "Clark streams smoothly");
    assert_eq!(callbacks, ["Clark ", "streams ", "smoothly"]);
}

#[tokio::test]
async fn never_publishes_single_token_identity_residue() {
    let chunks = vec![
        (
            Duration::ZERO,
            frame(r#"{"choices":[{"delta":{"content":"foreign_identity.example.com\n"}}]}"#),
        ),
        (
            Duration::ZERO,
            frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
        ),
        (Duration::ZERO, b"data: [DONE]\n\n".to_vec()),
    ];
    let base_url = progressive_endpoint(chunks).await;
    let client = LlmClient::from_parts(&base_url, "fake-model", None, Vec::new(), None).unwrap();
    let mut output = String::new();
    let error = client
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |delta| output.push_str(delta),
            |_| {},
        )
        .await
        .unwrap_err();

    assert!(output.is_empty());
    assert!(matches!(
        error,
        LlmError::OutputQuarantined {
            reason: "unprompted_identity_residue",
            ..
        }
    ));
}

#[tokio::test]
async fn productive_stream_can_outlive_response_start_deadline() {
    let chunks = [
        frame(r#"{"choices":[{"delta":{"content":"pro"}}]}"#),
        frame(r#"{"choices":[{"delta":{"content":"gress"}}]}"#),
        frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
        b"data: [DONE]\n\n".to_vec(),
    ]
    .into_iter()
    .map(|chunk| (Duration::from_millis(25), chunk))
    .collect();
    let base_url = progressive_endpoint(chunks).await;
    let client = LlmClient::from_parts_with_timeouts(
        &base_url,
        "fake-model",
        None,
        Vec::new(),
        None,
        Duration::from_millis(40),
        Duration::from_millis(60),
    )
    .unwrap();
    let started = std::time::Instant::now();
    let turn = client
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |_| {},
            |_| {},
        )
        .await
        .unwrap();

    assert_eq!(turn.text, "progress");
    assert!(started.elapsed() > Duration::from_millis(80));
}

#[tokio::test]
async fn stream_idle_deadline_resets_after_every_chunk() {
    let chunks = [
        frame(r#"{"choices":[{"delta":{"content":"still "}}]}"#),
        frame(r#"{"choices":[{"delta":{"content":"working"}}]}"#),
        frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
        b"data: [DONE]\n\n".to_vec(),
    ]
    .into_iter()
    .map(|chunk| (Duration::from_millis(30), chunk))
    .collect();
    let base_url = progressive_endpoint(chunks).await;
    let client = LlmClient::from_parts_with_timeouts(
        &base_url,
        "fake-model",
        None,
        Vec::new(),
        None,
        Duration::from_millis(40),
        Duration::from_millis(50),
    )
    .unwrap();
    let turn = client
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |_| {},
            |_| {},
        )
        .await
        .unwrap();

    assert_eq!(turn.text, "still working");
}

async fn idle_failure(content: &str) -> (AttemptError, String) {
    let chunks = vec![
        (
            Duration::ZERO,
            frame(&format!(
                r#"{{"choices":[{{"delta":{{"content":{content:?}}}}}]}}"#
            )),
        ),
        (Duration::from_millis(200), b"data: [DONE]\n\n".to_vec()),
    ];
    let base_url = progressive_endpoint(chunks).await;
    let client = LlmClient::from_parts_with_timeouts(
        &base_url,
        "fake-model",
        None,
        Vec::new(),
        None,
        Duration::from_millis(40),
        Duration::from_millis(40),
    )
    .unwrap();
    let mut output = String::new();
    let mut on_text = |text: &str| output.push_str(text);
    let mut on_reasoning = |_: &str| {};
    let error = client
        .stream_chat_once(StreamAttempt {
            messages: &[ChatMessage::user("hello")],
            tools: &[],
            cancel: &CancellationToken::new(),
            request_model: "fake-model",
            force_tool_call: false,
            forced_tool_name: None,
            idempotency_key: "stream-idle-test",
            on_text: &mut on_text,
            on_reasoning: &mut on_reasoning,
        })
        .await
        .unwrap_err();
    (error, output)
}

#[tokio::test]
async fn stream_idle_deadline_keeps_an_open_word_retry_safe() {
    let (error, output) = idle_failure("partial").await;
    let AttemptError::Transient(failure) = error else {
        panic!("expected a retry-safe stream timeout");
    };
    assert_eq!(failure.category, ProviderIncidentCategory::Timeout);
    assert!(failure.retry_safe);
    assert!(failure.output_started);
    assert!(output.is_empty());
}

#[tokio::test]
async fn stream_idle_deadline_never_retries_after_published_text() {
    let (error, output) = idle_failure("partial text ").await;
    let AttemptError::Transient(failure) = error else {
        panic!("expected a terminal stream timeout");
    };
    assert_eq!(failure.category, ProviderIncidentCategory::Timeout);
    assert!(!failure.retry_safe);
    assert!(failure.output_started);
    assert_eq!(output, "partial text ");
}
