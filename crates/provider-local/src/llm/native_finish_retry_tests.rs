use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

fn sse_response(frames: &[&str]) -> Vec<u8> {
    let body = frames
        .iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .chain(std::iter::once("data: [DONE]\n\n".to_string()))
        .collect::<String>();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn native_network_error(prefix: Option<&str>) -> Vec<u8> {
    let content = prefix.map(|prefix| {
        format!(
            r#"{{"id":"gen-network","model":"stealth/ox-alpha","provider":"Stealth","choices":[{{"delta":{{"content":{prefix:?}}},"finish_reason":null,"native_finish_reason":null}}]}}"#
        )
    });
    let failure = r#"{"id":"gen-network","model":"stealth/ox-alpha","provider":"Stealth","choices":[{"delta":{"content":""},"finish_reason":"stop","native_finish_reason":"network_error"}]}"#;
    match content {
        Some(content) => sse_response(&[content.as_str(), failure]),
        None => sse_response(&[failure]),
    }
}

fn success() -> Vec<u8> {
    sse_response(&[
        r#"{"id":"gen-success","model":"stealth/ox-alpha","provider":"Stealth","choices":[{"delta":{"content":"done"},"finish_reason":null,"native_finish_reason":null}]}"#,
        r#"{"id":"gen-success","model":"stealth/ox-alpha","provider":"Stealth","choices":[{"delta":{},"finish_reason":"stop","native_finish_reason":"stop"}]}"#,
    ])
}

async fn read_request(stream: &mut TcpStream) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut body_len = None;
    loop {
        let count = stream.read(&mut chunk).await.unwrap();
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
        let Some(headers_end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
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
            return;
        }
    }
}

async fn endpoint(responses: Vec<Vec<u8>>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let server_calls = calls.clone();
    tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            server_calls.fetch_add(1, Ordering::SeqCst);
            stream.write_all(&response).await.unwrap();
            stream.flush().await.unwrap();
        }
    });
    (format!("http://{address}/v1"), calls)
}

async fn run(base_url: &str, output: &mut String) -> Result<AssistantTurn, LlmError> {
    LlmClient::from_parts(base_url, "stealth/ox-alpha", None, Vec::new(), None)
        .unwrap()
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |text| output.push_str(text),
            |_| {},
        )
        .await
}

#[tokio::test]
async fn retries_two_native_network_failures_then_recovers() {
    let (base_url, calls) = endpoint(vec![
        native_network_error(None),
        native_network_error(None),
        success(),
    ])
    .await;
    let mut output = String::new();
    let turn = run(&base_url, &mut output).await.unwrap();

    assert_eq!(turn.text, "done");
    assert_eq!(output, "done");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let metadata = turn.response_metadata.expect("transport receipt");
    assert_eq!(metadata.request_attempts, Some(3));
    assert_eq!(metadata.transient_retries, Some(2));
    assert_eq!(metadata.native_finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn native_network_retry_exhaustion_is_typed_and_bounded() {
    let responses = (0..=MAX_NATIVE_NETWORK_RETRIES)
        .map(|_| native_network_error(None))
        .collect();
    let (base_url, calls) = endpoint(responses).await;
    let error = run(&base_url, &mut String::new()).await.unwrap_err();
    let LlmError::Recoverable(context) = error else {
        panic!("expected structured recoverable failure");
    };

    assert_eq!(context.category, ProviderIncidentCategory::ConnectionLost);
    assert_eq!(
        context.provider_error_type.as_deref(),
        Some("native_network_error")
    );
    assert_eq!(context.attempts, 3);
    assert_eq!(context.max_attempts, 3);
    assert_eq!(context.retries.transient, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn native_network_error_after_published_output_is_not_replayed() {
    let (base_url, calls) = endpoint(vec![
        native_network_error(Some("visible output ")),
        success(),
    ])
    .await;
    let mut output = String::new();
    let error = run(&base_url, &mut output).await.unwrap_err();
    let LlmError::Recoverable(context) = error else {
        panic!("expected structured recoverable failure");
    };

    assert_eq!(output, "visible output ");
    assert!(context.output_started);
    assert_eq!(context.retries.transient, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
