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
async fn forwards_openrouter_tool_arguments_as_provider_deltas() {
    let chunks = vec![
        (
            Duration::ZERO,
            frame(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"answer-1","function":{"name":"final_","arguments":"{\"content\":\"Clark "}}]}}]}"#,
            ),
        ),
        (
            Duration::from_millis(20),
            frame(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"answer","arguments":"streams\"}"}}]},"finish_reason":"tool_calls"}]}"#,
            ),
        ),
        (Duration::ZERO, b"data: [DONE]\n\n".to_vec()),
    ];
    let base_url = progressive_endpoint(chunks).await;
    let client = LlmClient::from_parts(&base_url, "fake-model", None, Vec::new(), None).unwrap();
    let cancel = CancellationToken::new();
    let mut callbacks = Vec::new();
    let turn = client
        .stream_chat_observed_with_tool_choice(
            &[ChatMessage::user("hello")],
            &[],
            StreamChatOptions {
                cancel: &cancel,
                force_tool_call: true,
                forced_tool_name: None,
            },
            StreamObservers::new(
                |_: &str| {},
                |_: &str| {},
                |delta| callbacks.push(delta),
                |_| {},
            ),
        )
        .await
        .unwrap();

    assert_eq!(turn.tool_calls[0].function.name, "final_answer");
    assert_eq!(
        turn.tool_calls[0].function.arguments,
        r#"{"content":"Clark streams"}"#
    );
    assert_eq!(callbacks.len(), 2);
    assert_eq!(callbacks[0].name_delta.as_deref(), Some("final_"));
    assert_eq!(
        callbacks[0].arguments_delta.as_deref(),
        Some(r#"{"content":"Clark "#)
    );
    assert_eq!(callbacks[1].name_delta.as_deref(), Some("answer"));
    assert_eq!(callbacks[1].arguments_delta.as_deref(), Some("streams\"}"));
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
    let client = LlmClient::from_parts_with_response_start_timeout(
        &base_url,
        "fake-model",
        None,
        Vec::new(),
        None,
        Duration::from_millis(40),
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
async fn provider_owns_stream_silence_and_the_user_can_still_cancel() {
    let base_url = progressive_endpoint(vec![(
        Duration::from_millis(250),
        b"data: [DONE]\n\n".to_vec(),
    )])
    .await;
    let client = LlmClient::from_parts_with_response_start_timeout(
        &base_url,
        "fake-model",
        None,
        Vec::new(),
        None,
        Duration::from_millis(40),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        client
            .stream_chat(
                &[ChatMessage::user("hello")],
                &[],
                &task_cancel,
                |_| {},
                |_| {},
            )
            .await
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(!task.is_finished(), "stream silence must not end the run");
    cancel.cancel();
    assert!(matches!(task.await.unwrap(), Err(LlmError::Cancelled)));
}
