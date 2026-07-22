//! Deterministic attachment-ingestion benchmark across local and remote sessions.
//!
//! The default lanes use a scripted OpenAI-compatible endpoint, so they cost
//! nothing and grade the exact model-visible boundary. The ignored lane keeps
//! that scripted model but replaces the loopback executor with the production
//! SSH deploy/tunnel path:
//!
//! ```sh
//! cargo test -p clark-desktop --test attachment_benchmark -- --nocapture
//!
//! CLARK_SSH_TEST_HOST=a6000 \
//! CLARK_SSH_TEST_ROOT=/home/ubuntu/clark-attachment-benchmark \
//! CLARK_SSH_TEST_BIN=target/x86_64-unknown-linux-musl/release/clark-exec-server \
//! cargo test -p clark-desktop --test attachment_benchmark \
//!   attachment_benchmark_real_ssh -- --ignored --nocapture --test-threads=1
//! ```

use std::path::PathBuf;
use std::process::Command;
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
use clark_desktop_lib::ssh;
use clark_desktop_lib::ssh::RemoteSpec;
use futures::StreamExt;
use serde_json::json;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

const EXEC_TOKEN: &str = "attachment-benchmark-capability";
const LARGE_PASTE_BEGIN: &str = "LARGE_PASTE_SENTINEL_BEGIN";
const LARGE_PASTE_END: &str = "LARGE_PASTE_SENTINEL_END";
const TEXT_FILE_SENTINEL: &str = "TEXT_ATTACHMENT_SENTINEL_7f4c";
const VISION_DESCRIPTION: &str =
    "Attachment benchmark vision description: alpha image followed by beta image.";

#[derive(Clone)]
struct EvalMode {
    label: &'static str,
    remote: Option<RemoteEval>,
}

#[derive(Clone)]
struct RemoteEval {
    ws_url: String,
    token: String,
    cwd: String,
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
            upload(
                "opaque.bin",
                "application/octet-stream",
                &[0, 159, 146, 150],
            ),
        ],
    }
}

async fn start_exec_server(root: PathBuf) -> String {
    let server = exec_server::bind(exec_server::Config {
        token: EXEC_TOKEN.to_string(),
        root: Some(root),
        addr: "127.0.0.1:0".to_string(),
    })
    .await
    .expect("bind attachment benchmark exec-server");
    let addr = server.local_addr().expect("exec-server local address");
    tokio::spawn(server.serve());
    format!("ws://{addr}")
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
        "opaque.bin — binary attachment; content not available to you",
        "not as a file on disk",
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
    let mut extra = json!({
        "base_url": base_url,
        "model": "attachment-benchmark-model",
        "memories": false,
        "project_knowledge": false,
        "research": false
    });
    if let Some(remote) = &mode.remote {
        extra["remote"] = json!({
            "ws_url": remote.ws_url,
            "token": remote.token,
            "cwd": remote.cwd
        });
    }

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
        Some(mode.remote.is_some())
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
        EvalMode {
            label: "local",
            remote: None,
        },
        Some(project.path().to_path_buf()),
    )
    .await;
}

#[tokio::test]
async fn attachment_benchmark_remote_transport() {
    let project = tempfile::tempdir().expect("remote transport benchmark root");
    let root = project.path().to_path_buf();
    let ws_url = start_exec_server(root.clone()).await;
    run_attachment_benchmark(
        EvalMode {
            label: "remote_transport",
            remote: Some(RemoteEval {
                ws_url,
                token: EXEC_TOKEN.to_string(),
                cwd: root.to_string_lossy().into_owned(),
            }),
        },
        None,
    )
    .await;
}

#[tokio::test]
#[ignore = "needs a live SSH host; set CLARK_SSH_TEST_{HOST,ROOT,BIN}"]
async fn attachment_benchmark_real_ssh() {
    let host = std::env::var("CLARK_SSH_TEST_HOST").expect("CLARK_SSH_TEST_HOST");
    let base = std::env::var("CLARK_SSH_TEST_ROOT").expect("CLARK_SSH_TEST_ROOT");
    let binary = PathBuf::from(std::env::var("CLARK_SSH_TEST_BIN").expect("CLARK_SSH_TEST_BIN"));
    let binary = if binary.is_absolute() {
        binary
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join(binary)
    };
    let root = format!("{base}/run-{}", uuid::Uuid::new_v4());
    let mkdir = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            &host,
            &format!("mkdir -p '{root}'"),
        ])
        .status()
        .expect("create SSH attachment benchmark root");
    assert!(mkdir.success(), "could not create {root} on {host}");

    let connection = ssh::connect(&RemoteSpec {
        host: host.clone(),
        remote_root: root.clone(),
        local_binary: Some(binary),
    })
    .await
    .expect("connect real SSH attachment benchmark");
    run_attachment_benchmark(
        EvalMode {
            label: "real_ssh",
            remote: Some(RemoteEval {
                ws_url: connection.ws_url.clone(),
                token: connection.token.clone(),
                cwd: root.clone(),
            }),
        },
        None,
    )
    .await;
    drop(connection);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let _ = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            &host,
            &format!("rmdir '{root}'"),
        ])
        .status();
}
