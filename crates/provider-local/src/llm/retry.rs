use std::time::Duration;

use futures::StreamExt;
use reqwest::header::RETRY_AFTER;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    drain_lines, retry_after_from_metadata, Accumulator, AssistantTurn, ChatMessage, LlmClient,
    LlmError, ToolSchema,
};

const MAX_RATE_LIMIT_RETRIES: usize = 12;
const MAX_RATE_LIMIT_DELAY: Duration = Duration::from_secs(30);
const MAX_TRANSIENT_RETRIES: usize = 3;
const MAX_TRANSIENT_DELAY: Duration = Duration::from_secs(8);
const MAX_SERVER_TRANSIENT_DELAY: Duration = Duration::from_secs(30);
const MAX_AUTH_RETRIES: usize = 1;

/// Classify the provider's context-overflow dialect while the response is
/// still at the model transport boundary.
fn is_context_overflow_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("context_length_exceeded")
        || lower.contains("context length")
        || lower.contains("context window")
        || lower.contains("exceeds the maximum number of tokens")
        || (lower.contains("too many tokens") && !lower.contains("rate"))
}

struct RetryableFailure {
    message: String,
    retry_after: Option<Duration>,
    retry_safe: bool,
}

enum AttemptError {
    Terminal(LlmError),
    RateLimited(RetryableFailure),
    Transient(RetryableFailure),
    PlatformKeyRejected(String),
}

impl From<LlmError> for AttemptError {
    fn from(error: LlmError) -> Self {
        Self::Terminal(error)
    }
}

impl LlmClient {
    /// Stream one chat completion, transparently replaying rate-limited calls
    /// only while the attempt has emitted no user-visible output.
    pub async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        cancel: &CancellationToken,
        mut on_text: impl FnMut(&str),
        mut on_reasoning: impl FnMut(&str),
    ) -> Result<AssistantTurn, LlmError> {
        // One key identifies this logical model turn across gateway and client
        // retries. The Clark gateway forwards it upstream, so a response-start
        // timeout can be replayed without creating a second billable generation.
        let idempotency_key = format!("clark-code-{}", Uuid::new_v4());
        let mut rate_limit_retries = 0;
        let mut transient_retries = 0;
        let mut auth_retries = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(LlmError::Cancelled);
            }
            match self
                .stream_chat_once(
                    messages,
                    tools,
                    cancel,
                    &idempotency_key,
                    &mut on_text,
                    &mut on_reasoning,
                )
                .await
            {
                Ok(turn) => return Ok(turn),
                Err(AttemptError::Terminal(error)) => return Err(error),
                Err(AttemptError::RateLimited(failure))
                    if failure.retry_safe && rate_limit_retries < MAX_RATE_LIMIT_RETRIES =>
                {
                    let delay = rate_limit_delay(failure.retry_after, rate_limit_retries);
                    rate_limit_retries += 1;
                    tracing::warn!(
                        model = self.model,
                        retry = rate_limit_retries,
                        max_retries = MAX_RATE_LIMIT_RETRIES,
                        delay_ms = delay.as_millis() as u64,
                        "model rate limited before output; retrying the same turn",
                    );
                    if !delay.is_zero() {
                        tokio::select! {
                            _ = cancel.cancelled() => return Err(LlmError::Cancelled),
                            _ = tokio::time::sleep(delay) => {}
                        }
                    }
                }
                Err(AttemptError::RateLimited(failure)) => {
                    return Err(LlmError::RateLimited(failure.message));
                }
                Err(AttemptError::Transient(failure))
                    if failure.retry_safe && transient_retries < MAX_TRANSIENT_RETRIES =>
                {
                    let delay = transient_delay(failure.retry_after, transient_retries);
                    transient_retries += 1;
                    tracing::warn!(
                        model = self.model,
                        retry = transient_retries,
                        max_retries = MAX_TRANSIENT_RETRIES,
                        delay_ms = delay.as_millis() as u64,
                        error = failure.message,
                        "model request failed before output; retrying the same turn",
                    );
                    if !delay.is_zero() {
                        tokio::select! {
                            _ = cancel.cancelled() => return Err(LlmError::Cancelled),
                            _ = tokio::time::sleep(delay) => {}
                        }
                    }
                }
                Err(AttemptError::Transient(failure)) => {
                    return Err(LlmError::Transport(failure.message));
                }
                Err(AttemptError::PlatformKeyRejected(_message))
                    if auth_retries < MAX_AUTH_RETRIES =>
                {
                    auth_retries += 1;
                    tracing::warn!(
                        retry = auth_retries,
                        max_retries = MAX_AUTH_RETRIES,
                        "Clark API rejected the current platform key; retrying once",
                    );
                }
                Err(AttemptError::PlatformKeyRejected(message)) => {
                    return Err(LlmError::PlatformKeyRejected(message));
                }
            }
        }
    }

    async fn stream_chat_once(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        cancel: &CancellationToken,
        idempotency_key: &str,
        on_text: &mut impl FnMut(&str),
        on_reasoning: &mut impl FnMut(&str),
    ) -> Result<AssistantTurn, AttemptError> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut request = self
            .http
            .post(&url)
            .header("Idempotency-Key", idempotency_key)
            .json(&self.body(messages, tools));
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        for (name, value) in &self.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(LlmError::Cancelled.into()),
            response = request.send() => response.map_err(|error| {
                AttemptError::Transient(RetryableFailure {
                    message: format!("model request failed: {error}"),
                    retry_after: None,
                    retry_safe: true,
                })
            })?,
        };
        let status = response.status();
        if !status.is_success() {
            let retry_after = retry_after_header(response.headers());
            let body = tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Cancelled.into()),
                body = response.text() => body.unwrap_or_default(),
            };
            if status.as_u16() == 403 && body.to_lowercase().contains("credit") {
                return Err(LlmError::InsufficientCredits.into());
            }
            let message = format!(
                "model endpoint returned {status}: {}",
                body.chars().take(500).collect::<String>()
            );
            if status.as_u16() == 401 {
                return Err(AttemptError::PlatformKeyRejected(message));
            }
            if status.as_u16() == 429 {
                return Err(AttemptError::RateLimited(RetryableFailure {
                    message,
                    retry_after: retry_after.or_else(|| retry_after_body(&body)),
                    retry_safe: true,
                }));
            }
            if is_context_overflow_message(&message) {
                return Err(LlmError::ContextOverflow(message).into());
            }
            if is_transient_status(status.as_u16()) {
                return Err(AttemptError::Transient(RetryableFailure {
                    message,
                    retry_after: retry_after.or_else(|| retry_after_body(&body)),
                    retry_safe: true,
                }));
            }
            return Err(LlmError::Provider(message).into());
        }

        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut accumulator = Accumulator::default();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Cancelled.into()),
                next = stream.next() => next,
            };
            match next {
                None => break,
                Some(Err(error)) => {
                    return Err(AttemptError::Transient(RetryableFailure {
                        message: format!("model stream error: {error}"),
                        retry_after: None,
                        retry_safe: !accumulator.emitted_output(),
                    }));
                }
                Some(Ok(bytes)) => {
                    buffer.extend_from_slice(&bytes);
                    if drain_lines(&mut buffer, &mut accumulator, on_text, on_reasoning) {
                        break;
                    }
                }
            }
        }
        if let Some(error) = accumulator.stream_error.take() {
            if error.is_rate_limited() {
                return Err(AttemptError::RateLimited(RetryableFailure {
                    message: error.message,
                    retry_after: error.retry_after,
                    retry_safe: !accumulator.emitted_output(),
                }));
            }
            if is_context_overflow_message(&error.message) {
                return Err(LlmError::ContextOverflow(error.message).into());
            }
            if error.is_transient() {
                return Err(AttemptError::Transient(RetryableFailure {
                    message: error.message,
                    retry_after: error.retry_after,
                    retry_safe: !accumulator.emitted_output(),
                }));
            }
            return Err(LlmError::Provider(error.message).into());
        }
        Ok(accumulator.finish())
    }
}

fn retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn retry_after_body(body: &str) -> Option<Duration> {
    serde_json::from_str::<Value>(body)
        .ok()?
        .get("error")?
        .get("metadata")?
        .as_object()
        .and_then(retry_after_from_metadata)
}

fn rate_limit_delay(server_hint: Option<Duration>, retry: usize) -> Duration {
    server_hint
        .unwrap_or_else(|| Duration::from_secs(2_u64 << retry.min(4)))
        .min(MAX_RATE_LIMIT_DELAY)
}

fn transient_delay(server_hint: Option<Duration>, retry: usize) -> Duration {
    match server_hint {
        Some(delay) => delay.min(MAX_SERVER_TRANSIENT_DELAY),
        None => Duration::from_millis(500_u64 << retry.min(4)).min(MAX_TRANSIENT_DELAY),
    }
}

fn is_transient_status(status: u16) -> bool {
    status == 408 || status == 425 || status == 524 || (500..=504).contains(&status)
}

#[cfg(test)]
mod tests {
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
        let body = r#"{"error":{"message":"invalid Clark platform key"}}"#;
        format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
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
    async fn retries_http_524_before_output() {
        let (base_url, calls) = endpoint(vec![http_gateway_timeout(), success()]).await;
        let mut output = String::new();
        let turn = run(&base_url, &mut output).await.unwrap();
        assert_eq!(turn.text, "done");
        assert_eq!(output, "done");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retries_keep_one_idempotency_key_for_the_logical_turn() {
        let (base_url, mut requests) =
            endpoint_capturing_requests(vec![http_gateway_timeout(), success()]).await;
        run(&base_url, &mut String::new()).await.unwrap();
        let keys = [
            requests.recv().await.unwrap(),
            requests.recv().await.unwrap(),
        ]
        .map(|request| {
            String::from_utf8_lossy(&request)
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("idempotency-key")
                        .then(|| value.trim().to_string())
                })
                .expect("idempotency key")
        });
        assert!(keys[0].starts_with("clark-code-"));
        assert_eq!(keys[0], keys[1]);
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
    async fn cancellation_interrupts_rate_limit_backoff() {
        let (base_url, calls) = endpoint(vec![http_rate_limit(30)]).await;
        let client =
            LlmClient::from_parts(&base_url, "fake-model", None, Vec::new(), None).unwrap();
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
            .stream_chat_once(
                &[ChatMessage::user("hello")],
                &[],
                &CancellationToken::new(),
                "deadline-test",
                &mut on_text,
                &mut on_reasoning,
            )
            .await
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(250));

        match error {
            AttemptError::Transient(failure) => {
                assert!(failure.message.contains("model request failed"));
                assert!(failure.retry_safe);
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
        assert_eq!(calls.load(Ordering::SeqCst), MAX_RATE_LIMIT_RETRIES + 1);
    }

    #[tokio::test]
    async fn does_not_retry_after_partial_output() {
        let partial = r#"data: {"choices":[{"delta":{"content":"partial"}}]}"#;
        let (base_url, calls) = endpoint(vec![in_band_rate_limit(partial)]).await;
        let mut output = String::new();
        let error = run(&base_url, &mut output).await.unwrap_err();
        assert!(error.to_string().contains("429 rate_limit_exceeded"));
        assert_eq!(output, "partial");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn does_not_retry_transient_failure_after_partial_output() {
        let partial = r#"data: {"choices":[{"delta":{"content":"partial"}}]}"#;
        let (base_url, calls) = endpoint(vec![in_band_provider_unavailable(partial)]).await;
        let mut output = String::new();
        let error = run(&base_url, &mut output).await.unwrap_err();
        assert!(matches!(error, LlmError::Transport(_)));
        assert_eq!(output, "partial");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
