//! Deterministic local attachment-ingestion benchmark.
//!
//! This uses a scripted OpenAI-compatible endpoint, so it costs nothing and
//! grades the exact model-visible boundary. Remote attachment coverage belongs
//! to the durable worker protocol used by the coding provider.
//!
//! ```sh
//! cargo test -p clark-desktop --test attachment_benchmark -- --nocapture
//!
//! ```

use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use agent_core::domain::AgentEvent;
use agent_core::domain::ContentBlock;
use agent_core::domain::PendingUpload;
use agent_core::domain::RunStatus;
use agent_core::provider::PromptInput;
use agent_core::provider::Provider;
use agent_core::provider::ProviderConfig;
use agent_core::provider::SessionOptions;
use base64::Engine as _;
use futures::StreamExt;
use serde_json::json;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

const LARGE_PASTE_BEGIN: &str = "LARGE_PASTE_SENTINEL_BEGIN";
const LARGE_PASTE_END: &str = "LARGE_PASTE_SENTINEL_END";
const TEXT_FILE_SENTINEL: &str = "TEXT_ATTACHMENT_SENTINEL_7f4c";
const VISION_DESCRIPTION: &str =
    "Attachment benchmark vision description: alpha image followed by beta image.";

#[derive(Clone)]
struct EvalMode {
    label: &'static str,
}

fn large_paste_text() -> String {
    format!(
        "{LARGE_PASTE_BEGIN}\n{}\n{LARGE_PASTE_END}",
        "codex-style-expanded-paste ".repeat(240)
    )
}

fn upload(filename: &str, content_type: &str, bytes: &[u8]) -> PendingUpload {
    PendingUpload {
        filename: filename.to_string(),
        content_type: content_type.to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

fn eval_input() -> PromptInput {
    PromptInput {
        // A pending paste must already be expanded into normal text
        // by this boundary. It is deliberately not a text-file attachment.
        blocks: vec![ContentBlock::text(format!(
            "attachment-eval-request\n{}",
            large_paste_text()
        ))],
        attachments: vec![
            upload("notes.txt", "text/plain", TEXT_FILE_SENTINEL.as_bytes()),
            upload("alpha.png", "image/png", b"alpha-image-bytes"),
            upload("beta.png", "image/png", b"beta-image-bytes"),
        ],
    }
}

fn sse_text(text: &str) -> String {
    [
        format!(
            r#"data: {{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
            json!(text)
        ),
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#.to_string(),
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

async fn scripted_model() -> (String, JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind attachment benchmark model");
    let addr = listener.local_addr().expect("scripted model address");
    let bodies = [
        sse_text(VISION_DESCRIPTION),
        sse_text("attachment benchmark complete"),
    ];
    let handle = tokio::spawn(async move {
        let mut captured = Vec::with_capacity(bodies.len());
        for body in bodies {
            let (mut socket, _) = listener.accept().await.expect("accept model request");
            captured.push(read_request_json(&mut socket).await);
            socket
                .write_all(&http_response(&body))
                .await
                .expect("write scripted response");
        }
        captured
    });
    (format!("http://{addr}/v1"), handle)
}

async fn read_request_json(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut content_length = None;
    loop {
        let read = socket.read(&mut chunk).await.expect("read model request");
        assert!(read > 0, "model request ended before its body arrived");
        bytes.extend_from_slice(&chunk[..read]);
        let Some(headers_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        if content_length.is_none() {
            let headers = String::from_utf8_lossy(&bytes[..headers_end]);
            content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
        }
        let length = content_length.expect("model request content-length");
        let body_start = headers_end + 4;
        if bytes.len() >= body_start + length {
            return serde_json::from_slice(&bytes[body_start..body_start + length])
                .expect("valid model request JSON");
        }
    }
}

fn user_content(request: &Value) -> &Value {
    request["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .map(|message| &message["content"])
        .expect("user message")
}

fn grade_requests(mode: &str, requests: &[Value]) {
    assert_eq!(
        requests.len(),
        2,
        "{mode}: one vision call + one coding call"
    );

    let vision_parts = user_content(&requests[0])
        .as_array()
        .expect("vision request uses content parts");
    let image_urls = vision_parts
        .iter()
        .filter(|part| part["type"] == "image_url")
        .filter_map(|part| part["image_url"]["url"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(image_urls.len(), 2, "{mode}: images are batched");
    assert!(image_urls[0].starts_with("data:image/png;base64,"));
    assert!(image_urls[1].starts_with("data:image/png;base64,"));

    let coding_text = user_content(&requests[1])
        .as_str()
        .expect("coding model receives plain text");
    for expected in [
        "attachment-eval-request",
        LARGE_PASTE_BEGIN,
        LARGE_PASTE_END,
        TEXT_FILE_SENTINEL,
        VISION_DESCRIPTION,
    ] {
        assert!(
            coding_text.contains(expected),
            "{mode}: missing {expected:?}"
        );
    }
    assert!(
        !coding_text.contains("[Pasted Content"),
        "{mode}: composer placeholder leaked into model input"
    );
    assert!(
        !coding_text.contains("alpha-image-bytes") && !coding_text.contains("beta-image-bytes"),
        "{mode}: raw image bytes leaked into coding-model text"
    );
}

async fn run_attachment_benchmark(mode: EvalMode, local_cwd: Option<PathBuf>) {
    let (base_url, captured) = scripted_model().await;
    let extra = json!({
        "base_url": base_url,
        "model": "attachment-benchmark-model",
        "memories": false,
        "project_knowledge": false,
        "research": false
    });
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("attachment-benchmark-key".to_string()),
            extra,
            ..Default::default()
        })
        .await
        .expect("connect attachment benchmark provider");
    let session = provider
        .new_session(SessionOptions {
            cwd: local_cwd.map(|path| path.to_string_lossy().into_owned()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .expect("create attachment benchmark session");
    assert_eq!(
        session.environment.as_ref().map(|env| env.remote),
        Some(false)
    );

    let input = eval_input();
    let payload_bytes = input
        .attachments
        .iter()
        .map(|attachment| attachment.data_base64.len())
        .sum::<usize>()
        + large_paste_text().len();
    let started = Instant::now();
    let mut stream = provider
        .prompt(&session.id, input)
        .await
        .expect("submit attachment benchmark turn");
    let status = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = stream.next().await {
            if let AgentEvent::RunFinished { outcome, .. } = event {
                return outcome.status;
            }
        }
        panic!("{}: run stream ended without RunFinished", mode.label);
    })
    .await
    .expect("attachment benchmark timed out");
    assert_eq!(
        status,
        RunStatus::Done,
        "{}: provider run failed",
        mode.label
    );

    let requests = captured.await.expect("scripted model task");
    grade_requests(mode.label, &requests);
    println!(
        "{}",
        json!({
            "eval": "attachments",
            "mode": mode.label,
            "status": "pass",
            "elapsed_ms": started.elapsed().as_millis(),
            "model_calls": requests.len(),
            "payload_bytes": payload_bytes,
            "checks": 11
        })
    );
}

#[tokio::test]
async fn attachment_benchmark_local() {
    let project = tempfile::tempdir().expect("local attachment benchmark root");
    run_attachment_benchmark(
        EvalMode { label: "local" },
        Some(project.path().to_path_buf()),
    )
    .await;
}
