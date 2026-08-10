use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

fn sse_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn http_rate_limit(retry_after: u64) -> Vec<u8> {
    let body = r#"{"error":{"code":429,"message":"Provider returned error","metadata":{"retry_after_seconds":0}}}"#;
    format!(
        "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: {retry_after}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    )
    .into_bytes()
}

fn http_gateway_timeout() -> Vec<u8> {
    let body = "error code: 524\n";
    format!(
        "HTTP/1.1 524 Origin Time-out\r\nContent-Type: text/plain\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    )
    .into_bytes()
}

fn http_unauthorized() -> Vec<u8> {
    let body = r#"{"error":{"message":"invalid managed platform key"}}"#;
    format!(
        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    )
    .into_bytes()
}

fn http_forbidden_credentials() -> Vec<u8> {
    let body = r#"{"error":{"message":"credit service credentials were rejected"}}"#;
    format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    )
    .into_bytes()
}

fn http_payment_required() -> Vec<u8> {
    let body = r#"{"error":{"message":"Insufficient Clark Code credits. Add credits to continue.","type":"usage_limit_reached"}}"#;
    format!(
        "HTTP/1.1 402 Payment Required\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    )
    .into_bytes()
}

const FALLBACK_MODEL: &str = "managed-model-safe";
const FALLBACK_REASON: &str = "unsupported_model_option";

fn http_unknown_managed_model() -> Vec<u8> {
    let body = r#"{"error":{"message":"Unknown managed model option.","type":"invalid_request_error","param":"model","code":"invalid_request_error"}}"#;
    format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    )
    .into_bytes()
}

fn with_model_fallback(mut client: LlmClient) -> LlmClient {
    client.model_fallback = Some(crate::config::ModelFallbackPolicy {
        model: FALLBACK_MODEL.into(),
        reason: FALLBACK_REASON.into(),
        error_type: "invalid_request_error".into(),
        error_param: "model".into(),
        error_message: "Unknown managed model option.".into(),
    });
    client
}

fn http_unrelated_bad_request() -> Vec<u8> {
    let body = r#"{"error":{"message":"Unsupported reasoning value.","type":"invalid_request_error","param":"reasoning_effort","code":"invalid_request_error"}}"#;
    format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    )
    .into_bytes()
}

fn in_band_rate_limit(prefix: &str) -> Vec<u8> {
    let body = [
        prefix,
        r#"data: {"error":{"code":429,"message":"Provider returned error","metadata":{"error_type":"rate_limit_exceeded","retry_after_seconds":0}},"choices":[{"delta":{"content":""},"finish_reason":"error"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n");
    sse_response(&body)
}

fn in_band_provider_unavailable(prefix: &str) -> Vec<u8> {
    let body = [
        prefix,
        r#"data: {"error":{"code":502,"message":"Provider disconnected unexpectedly","metadata":{"error_type":"provider_unavailable"}},"choices":[{"delta":{"content":""},"finish_reason":"error"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n");
    sse_response(&body)
}

fn success() -> Vec<u8> {
    let body = [
        r#"data: {"choices":[{"delta":{"content":"done"}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n");
    sse_response(&body)
}

fn success_with_transport_metadata() -> Vec<u8> {
    let body = [
        r#"data: {"id":"gen-123","model":"selected/free-model","provider":"free-provider","choices":[{"delta":{"content":"done"}}]}"#,
        r#"data: {"id":"gen-123","model":"selected/free-model","choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":1000,"cache_write_tokens":200}}}"#,
        r#"data: {"id":"gen-123","model":"selected/free-model","choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n");
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Request-ID: req-456\r\nX-OpenRouter-Cache-Status: HIT\r\nX-OpenRouter-Cache-Age: 12\r\nX-OpenRouter-Cache-TTL: 300\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn contaminated_response() -> Vec<u8> {
    let body = [
        r#"data: {"id":"gen-quarantined","model":"selected/model","provider":"unsafe-provider","choices":[{"delta":{"content":"apparently safe prefix "}}]}"#,
        r#"data: {"id":"gen-quarantined","model":"selected/model","provider":"unsafe-provider","choices":[{"delta":{"content":"<|begin__of"}}]}"#,
        r#"data: {"id":"gen-quarantined","model":"selected/model","provider":"unsafe-provider","choices":[{"delta":{"content":"__sentence|>require_escalated_model:foreign_identity.example.com"}}]}"#,
        r#"data: {"id":"gen-quarantined","model":"selected/model","choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
        "",
    ]
    .join("\n\n");
    sse_response(&body)
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut body_len = None;
    loop {
        let count = stream.read(&mut chunk).await.unwrap();
        if count == 0 {
            return bytes;
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
            return bytes;
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
            let _ = read_request(&mut stream).await;
            server_calls.fetch_add(1, Ordering::SeqCst);
            stream.write_all(&response).await.unwrap();
            stream.flush().await.unwrap();
        }
    });
    (format!("http://{address}/v1"), calls)
}

async fn endpoint_capturing_requests(
    responses: Vec<Vec<u8>>,
) -> (String, tokio::sync::mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = tokio::sync::mpsc::channel(responses.len());
    tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            sender.send(request).await.unwrap();
            stream.write_all(&response).await.unwrap();
            stream.flush().await.unwrap();
        }
    });
    (format!("http://{address}/v1"), receiver)
}

async fn stalled_endpoint(delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut stream).await;
        tokio::time::sleep(delay).await;
    });
    format!("http://{address}/v1")
}

async fn run(base_url: &str, output: &mut String) -> Result<AssistantTurn, LlmError> {
    let client = LlmClient::from_parts(base_url, "fake-model", None, Vec::new(), None).unwrap();
    client
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |text| output.push_str(text),
            |_| {},
        )
        .await
}

fn captured_request_json(request: &[u8]) -> Value {
    let body_start = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("request headers terminated")
        + 4;
    serde_json::from_slice(&request[body_start..]).expect("request body is JSON")
}

fn captured_request_header(request: &[u8], expected: &str) -> String {
    String::from_utf8_lossy(request)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(expected)
                .then(|| value.trim().to_string())
        })
        .unwrap_or_else(|| panic!("missing {expected} header"))
}

#[test]
fn fallback_delay_is_exponential_and_capped() {
    let delays = (0..8)
        .map(|retry| rate_limit_delay(None, retry))
        .collect::<Vec<_>>();
    assert_eq!(
        delays,
        vec![
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ]
    );
    assert_eq!(
        rate_limit_delay(Some(Duration::from_secs(7)), 0),
        Duration::from_secs(7)
    );
    assert_eq!(
        rate_limit_delay(Some(Duration::from_secs(45)), 0),
        MAX_RATE_LIMIT_DELAY
    );
}

#[test]
fn transient_delay_honors_bounded_server_cooldowns() {
    let delays = (0..8)
        .map(|retry| transient_delay(None, retry))
        .collect::<Vec<_>>();
    assert_eq!(
        delays,
        vec![
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(8),
            Duration::from_secs(8),
            Duration::from_secs(8),
        ]
    );
    assert_eq!(
        transient_delay(Some(Duration::from_secs(23)), 0),
        Duration::from_secs(23)
    );
    assert_eq!(
        transient_delay(Some(Duration::from_secs(90)), 0),
        MAX_SERVER_TRANSIENT_DELAY
    );
}

#[tokio::test]
async fn retries_in_band_rate_limit_before_output() {
    let (base_url, calls) = endpoint(vec![in_band_rate_limit(""), success()]).await;
    let mut output = String::new();
    let turn = run(&base_url, &mut output).await.unwrap();
    assert_eq!(turn.text, "done");
    assert_eq!(output, "done");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retries_http_rate_limit_before_output() {
    let (base_url, calls) = endpoint(vec![http_rate_limit(0), success()]).await;
    let mut output = String::new();
    let turn = run(&base_url, &mut output).await.unwrap();
    assert_eq!(turn.text, "done");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn configured_model_rejection_falls_back_once() {
    let (base_url, mut requests) =
        endpoint_capturing_requests(vec![http_unknown_managed_model(), success()]).await;
    let requested_model = "managed-model-large";
    let client = with_model_fallback(
        LlmClient::from_parts(&base_url, requested_model, None, Vec::new(), None).unwrap(),
    )
    .with_session_id("fallback-session");

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

    assert_eq!(turn.text, "done");
    let metadata = turn.response_metadata.expect("fallback receipt");
    assert_eq!(metadata.requested_model.as_deref(), Some(requested_model));
    assert_eq!(metadata.fallback_model.as_deref(), Some(FALLBACK_MODEL));
    assert_eq!(metadata.fallback_reason.as_deref(), Some(FALLBACK_REASON));
    assert_eq!(metadata.request_attempts, Some(2));
    assert_eq!(metadata.rate_limit_retries, Some(0));
    assert_eq!(metadata.transient_retries, Some(0));
    assert_eq!(metadata.authentication_retries, Some(0));
    let receipt_idempotency_key = metadata.idempotency_key.expect("fallback request identity");

    let first = requests.recv().await.unwrap();
    let second = requests.recv().await.unwrap();
    assert_eq!(captured_request_json(&first)["model"], requested_model);
    assert_eq!(captured_request_json(&second)["model"], FALLBACK_MODEL);
    assert_eq!(
        captured_request_header(&first, "x-session-id"),
        "fallback-session"
    );
    assert_eq!(
        captured_request_header(&second, "x-session-id"),
        "fallback-session"
    );
    let first_idempotency_key = captured_request_header(&first, "idempotency-key");
    let second_idempotency_key = captured_request_header(&second, "idempotency-key");
    assert_ne!(first_idempotency_key, second_idempotency_key);
    assert_eq!(receipt_idempotency_key, second_idempotency_key);
}

#[tokio::test]
async fn fallback_model_does_not_fallback_to_itself() {
    let (base_url, calls) = endpoint(vec![http_unknown_managed_model()]).await;
    let client = with_model_fallback(
        LlmClient::from_parts(&base_url, FALLBACK_MODEL, None, Vec::new(), None).unwrap(),
    );

    let error = client
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |_| {},
            |_| {},
        )
        .await
        .unwrap_err();

    assert!(matches!(error, LlmError::Provider(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unrelated_bad_request_does_not_trigger_model_fallback() {
    let (base_url, calls) = endpoint(vec![http_unrelated_bad_request()]).await;
    let client = with_model_fallback(
        LlmClient::from_parts(&base_url, "managed-model-large", None, Vec::new(), None).unwrap(),
    );

    let error = client
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |_| {},
            |_| {},
        )
        .await
        .unwrap_err();

    assert!(matches!(error, LlmError::Provider(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn managed_flash_model_survives_eight_consecutive_pool_rate_limits() {
    let mut responses = vec![http_rate_limit(0); 8];
    responses.push(success());
    let (base_url, calls) = endpoint(responses).await;
    let client = LlmClient::from_parts(&base_url, "vendor/managed-flash", None, Vec::new(), None)
        .unwrap()
        .with_session_id("managed-pool-eval-session");
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

    assert_eq!(turn.text, "done");
    assert_eq!(calls.load(Ordering::SeqCst), 9);
    let metadata = turn.response_metadata.expect("retry receipt");
    assert_eq!(metadata.request_attempts, Some(9));
    assert_eq!(metadata.rate_limit_retries, Some(8));
    assert_eq!(metadata.transient_retries, Some(0));
    assert_eq!(
        metadata.session_id.as_deref(),
        Some("managed-pool-eval-session")
    );
}

#[tokio::test]
async fn retries_http_524_before_output() {
    let (base_url, calls) = endpoint(vec![http_gateway_timeout(), success()]).await;
    let mut output = String::new();
    let turn = run(&base_url, &mut output).await.unwrap();
    assert_eq!(turn.text, "done");
    assert_eq!(output, "done");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retries_keep_idempotency_key_and_session_id_for_the_logical_turn() {
    let (base_url, mut requests) =
        endpoint_capturing_requests(vec![http_gateway_timeout(), success()]).await;
    let client = LlmClient::from_parts(&base_url, "fake-model", None, Vec::new(), None)
        .unwrap()
        .with_session_id("conversation-uuid");
    client
        .stream_chat(
            &[ChatMessage::user("hello")],
            &[],
            &CancellationToken::new(),
            |_| {},
            |_| {},
        )
        .await
        .unwrap();
    let headers = [
        requests.recv().await.unwrap(),
        requests.recv().await.unwrap(),
    ]
    .map(|request| {
        let request = String::from_utf8_lossy(&request);
        let header = |expected: &str| {
            request
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case(expected)
                        .then(|| value.trim().to_string())
                })
                .unwrap_or_else(|| panic!("missing {expected} header"))
        };
        (header("idempotency-key"), header("x-session-id"))
    });
    assert!(headers[0].0.starts_with("model-request-"));
    assert_eq!(headers[0].0, headers[1].0);
    assert_eq!(headers[0].1, "conversation-uuid");
    assert_eq!(headers[0].1, headers[1].1);
}

#[tokio::test]
async fn successful_turn_maps_cache_and_provider_request_identities() {
    let (base_url, calls) = endpoint(vec![success_with_transport_metadata()]).await;
    let mut output = String::new();
    let turn = run(&base_url, &mut output).await.unwrap();
    let metadata = turn.response_metadata.expect("response metadata captured");
    assert_eq!(metadata.requested_model.as_deref(), Some("fake-model"));
    assert_eq!(
        metadata.resolved_model.as_deref(),
        Some("selected/free-model")
    );
    assert_eq!(metadata.provider.as_deref(), Some("free-provider"));
    assert_eq!(metadata.generation_id.as_deref(), Some("gen-123"));
    assert_eq!(metadata.provider_request_id.as_deref(), Some("req-456"));
    assert!(metadata
        .idempotency_key
        .as_deref()
        .is_some_and(|value| value.starts_with("model-request-")));
    assert!(metadata.provider_route.is_some());
    assert_eq!(metadata.http_version.as_deref(), Some("HTTP/1.1"));
    assert_eq!(metadata.request_attempts, Some(1));
    assert_eq!(metadata.rate_limit_retries, Some(0));
    assert_eq!(metadata.transient_retries, Some(0));
    assert_eq!(metadata.authentication_retries, Some(0));
    assert_eq!(metadata.cache_status.as_deref(), Some("HIT"));
    assert_eq!(metadata.cache_age_seconds, Some(12));
    assert_eq!(metadata.cache_ttl_seconds, Some(300));
    assert_eq!(metadata.cached_prompt_tokens, Some(1000));
    assert_eq!(metadata.cache_write_tokens, Some(200));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn contaminated_turn_is_atomic_and_never_reaches_callbacks() {
    let (base_url, calls) = endpoint(vec![contaminated_response()]).await;
    let mut output = String::new();
    let error = run(&base_url, &mut output).await.unwrap_err();
    assert!(
        output.is_empty(),
        "no prefix from a rejected turn may escape"
    );
    let LlmError::OutputQuarantined { reason, metadata } = error else {
        panic!("expected typed quarantine error");
    };
    assert_eq!(reason, "reserved_protocol_marker");
    assert_eq!(metadata.generation_id.as_deref(), Some("gen-quarantined"));
    assert_eq!(metadata.resolved_model.as_deref(), Some("selected/model"));
    assert_eq!(metadata.provider.as_deref(), Some("unsafe-provider"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retries_in_band_provider_unavailable_before_output() {
    let (base_url, calls) = endpoint(vec![in_band_provider_unavailable(""), success()]).await;
    let mut output = String::new();
    let turn = run(&base_url, &mut output).await.unwrap();
    assert_eq!(turn.text, "done");
    assert_eq!(output, "done");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retries_current_platform_key_once_then_returns_typed_rejection() {
    let (base_url, calls) = endpoint(vec![http_unauthorized(), http_unauthorized()]).await;
    let mut output = String::new();
    let error = run(&base_url, &mut output).await.unwrap_err();
    assert!(matches!(error, LlmError::PlatformKeyRejected(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn maps_payment_required_to_billing_instead_of_provider_failure() {
    let (base_url, calls) = endpoint(vec![http_payment_required()]).await;
    let mut output = String::new();
    let error = run(&base_url, &mut output).await.unwrap_err();
    assert!(matches!(error, LlmError::InsufficientCredits));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn maps_forbidden_credentials_to_key_rejection_not_billing() {
    let (base_url, calls) = endpoint(vec![
        http_forbidden_credentials(),
        http_forbidden_credentials(),
    ])
    .await;
    let mut output = String::new();
    let error = run(&base_url, &mut output).await.unwrap_err();
    assert!(matches!(error, LlmError::PlatformKeyRejected(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancellation_interrupts_rate_limit_backoff() {
    let (base_url, calls) = endpoint(vec![http_rate_limit(30)]).await;
    let client = LlmClient::from_parts(&base_url, "fake-model", None, Vec::new(), None).unwrap();
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
    while calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    cancel.cancel();
    assert!(matches!(task.await.unwrap(), Err(LlmError::Cancelled)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn request_deadline_bounds_a_stalled_provider() {
    let base_url = stalled_endpoint(Duration::from_millis(500)).await;
    let client = LlmClient::from_parts_with_timeout(
        &base_url,
        "fake-model",
        None,
        Vec::new(),
        None,
        Duration::from_millis(40),
    )
    .unwrap();
    let mut on_text = |_: &str| {};
    let mut on_reasoning = |_: &str| {};
    let started = std::time::Instant::now();
    let error = client
        .stream_chat_once(StreamAttempt {
            messages: &[ChatMessage::user("hello")],
            tools: &[],
            cancel: &CancellationToken::new(),
            request_model: "fake-model",
            force_tool_call: false,
            idempotency_key: "deadline-test",
            on_text: &mut on_text,
            on_reasoning: &mut on_reasoning,
        })
        .await
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_millis(250));

    match error {
        AttemptError::Transient(failure) => {
            assert!(failure.message.contains("model request failed"));
            assert!(failure.retry_safe);
            assert_eq!(failure.category, ProviderIncidentCategory::Timeout);
            assert!(!failure.output_started);
        }
        _ => panic!("expected a retry-safe transient timeout"),
    }
}

#[tokio::test]
async fn stops_after_bounded_rate_limit_retries() {
    let responses = (0..=MAX_RATE_LIMIT_RETRIES)
        .map(|_| in_band_rate_limit(""))
        .collect();
    let (base_url, calls) = endpoint(responses).await;
    let error = run(&base_url, &mut String::new()).await.unwrap_err();
    assert!(error.to_string().contains("429 rate_limit_exceeded"));
    let LlmError::Recoverable(context) = error else {
        panic!("expected structured recoverable failure");
    };
    assert_eq!(context.category, ProviderIncidentCategory::RateLimit);
    assert_eq!(context.provider_status, Some(429));
    assert_eq!(context.retries.rate_limit, MAX_RATE_LIMIT_RETRIES as u32);
    assert_eq!(context.attempts, (MAX_RATE_LIMIT_RETRIES + 1) as u32);
    assert_eq!(calls.load(Ordering::SeqCst), MAX_RATE_LIMIT_RETRIES + 1);
}

#[tokio::test]
async fn does_not_retry_after_partial_output() {
    let partial = r#"data: {"choices":[{"delta":{"content":"partial"}}]}"#;
    let (base_url, calls) = endpoint(vec![in_band_rate_limit(partial)]).await;
    let mut output = String::new();
    let error = run(&base_url, &mut output).await.unwrap_err();
    assert!(error.to_string().contains("429 rate_limit_exceeded"));
    assert!(
        output.is_empty(),
        "an incomplete turn must remain quarantined"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn does_not_retry_transient_failure_after_partial_output() {
    let partial = r#"data: {"choices":[{"delta":{"content":"partial"}}]}"#;
    let (base_url, calls) = endpoint(vec![in_band_provider_unavailable(partial)]).await;
    let mut output = String::new();
    let error = run(&base_url, &mut output).await.unwrap_err();
    let LlmError::Recoverable(context) = error else {
        panic!("expected structured recoverable failure");
    };
    assert_eq!(
        context.category,
        ProviderIncidentCategory::UpstreamUnavailable
    );
    assert!(context.output_started);
    assert_eq!(context.retries.transient, 0);
    assert!(
        output.is_empty(),
        "an incomplete turn must remain quarantined"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
