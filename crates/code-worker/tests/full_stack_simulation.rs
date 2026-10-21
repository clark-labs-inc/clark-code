//! Full-stack worker simulation: the real `agent-code-worker` binary, driven
//! over its real stdio JSONL protocol, running the real provider-local agent
//! loop against a scripted OpenAI-compatible SSE model, executing real tools
//! in a real temporary Git repository.
//!
//! This is the integration seam no per-crate contract covers: config loading →
//! plugin registry → idempotency receipts → coding sessions → provider loop →
//! permission gate → tool execution → checkpointing → event projection, all
//! through the production wire format. Costs nothing: the "model" is a local
//! TCP fixture.
//!
//! Layout: one journey worker (multi-permit) runs the working-day scenario and
//! the kill/replay phase; a second single-permit worker proves saturation and
//! oversized-request behavior that a multi-permit config cannot reach.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use agent_core::projection::{apply, Snapshot};
use agent_core::{AgentEvent, PermissionOptionKind};
use code_host::{Request, RequestCommand, Response, PROTOCOL_VERSION};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};

const STEP_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Scripted OpenAI-compatible SSE model
// ---------------------------------------------------------------------------

/// One scripted completion: optionally stall before answering, then stream the
/// given SSE body. Requests beyond the script return a visible exhaustion
/// sentinel so the journey assertions fail without waiting on another response.
struct ScriptedTurn {
    stall: Duration,
    body: String,
}

fn sse(chunks: &[Value]) -> String {
    let mut lines = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}"))
        .collect::<Vec<_>>();
    lines.push(r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#.into());
    lines.push("data: [DONE]".into());
    lines.join("\n\n")
}

fn tool_call_turn(name: &str, arguments: &Value) -> ScriptedTurn {
    ScriptedTurn {
        stall: Duration::ZERO,
        body: sse(&[json!({"choices":[{"delta":{"tool_calls":[{
            "index": 0,
            "id": format!("call-{name}"),
            "function": {"name": name, "arguments": arguments.to_string()},
        }]}}]})]),
    }
}

fn assistant_turn(text: &str) -> ScriptedTurn {
    ScriptedTurn {
        stall: Duration::ZERO,
        body: [
            format!(
                "data: {}",
                json!({"choices":[{"delta":{"content": text}}]})
            ),
            r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#.into(),
            "data: [DONE]".into(),
        ]
        .join("\n\n"),
    }
}

struct ModelFixture {
    base_url: String,
    /// Bodies of every /chat/completions request, for cross-layer assertions.
    requests: Arc<Mutex<Vec<Value>>>,
}

fn spawn_model(script: Vec<ScriptedTurn>) -> ModelFixture {
    let requests: Arc<Mutex<Vec<Value>>> = Arc::default();
    let seen = requests.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    tokio::spawn(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind model");
        let port = listener.local_addr().expect("model addr").port();
        ready_tx.send(port).expect("report model port");
        // Turns are assigned in ACCEPT order but served concurrently: a
        // scripted stall (the cancellation scenario) must not serialize the
        // requests that follow it.
        let script = Arc::new(Mutex::new(std::collections::VecDeque::from(script)));
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let turn = script
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| assistant_turn("SCRIPT EXHAUSTED"));
            let seen = seen.clone();
            tokio::spawn(async move {
                let raw = read_http_request(&mut socket).await;
                if let Some(body) = http_body_json(&raw) {
                    seen.lock().await.push(body);
                }
                tokio::time::sleep(turn.stall).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    turn.body.len(),
                    turn.body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    let port = ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("model fixture came up");
    ModelFixture {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        requests,
    }
}

async fn read_http_request(socket: &mut TcpStream) -> Vec<u8> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut content_length = None;
    loop {
        let Ok(read) = socket.read(&mut chunk).await else {
            return buffer;
        };
        if read == 0 {
            return buffer;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let Some(headers_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        if content_length.is_none() {
            let headers = String::from_utf8_lossy(&buffer[..headers_end]);
            content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
        }
        if let Some(length) = content_length {
            if buffer.len() >= headers_end + 4 + length {
                return buffer;
            }
        }
    }
}

fn http_body_json(raw: &[u8]) -> Option<Value> {
    let headers_end = raw.windows(4).position(|window| window == b"\r\n\r\n")?;
    serde_json::from_slice(&raw[headers_end + 4..]).ok()
}

// ---------------------------------------------------------------------------
// Worker client over real stdio JSONL
// ---------------------------------------------------------------------------

struct WorkerClient {
    child: Child,
    stdin: tokio::process::ChildStdin,
    /// Frames routed by request id by the reader task.
    routes: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Response>>>>,
    stderr: Arc<Mutex<String>>,
}

impl WorkerClient {
    async fn spawn(config_path: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent-code-worker"))
            .arg("--config")
            .arg(config_path)
            .env("SIM_MODEL_KEY", "sim-secret")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn agent-code-worker");
        let stdin = child.stdin.take().expect("worker stdin");
        let stdout = child.stdout.take().expect("worker stdout");
        let stderr_pipe = child.stderr.take().expect("worker stderr");

        let routes: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Response>>>> = Arc::default();
        let reader_routes = routes.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(response) = serde_json::from_str::<Response>(&line) else {
                    panic!("worker emitted an undecodable frame: {line}");
                };
                let key = response.request_id().unwrap_or("").to_string();
                if let Some(route) = reader_routes.lock().await.get(&key) {
                    let _ = route.send(response);
                }
            }
        });
        let stderr: Arc<Mutex<String>> = Arc::default();
        let stderr_sink = stderr.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr_pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                stderr_sink.lock().await.push_str(&line);
                stderr_sink.lock().await.push('\n');
            }
        });
        Self {
            child,
            stdin,
            routes,
            stderr,
        }
    }

    async fn send_line(&mut self, line: &str) {
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write request");
        self.stdin.write_all(b"\n").await.expect("write newline");
        self.stdin.flush().await.expect("flush request");
    }

    async fn open_route(&self, request_id: &str) -> mpsc::UnboundedReceiver<Response> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.routes.lock().await.insert(request_id.into(), tx);
        rx
    }

    async fn send(&mut self, request: &Request) -> mpsc::UnboundedReceiver<Response> {
        let rx = self.open_route(&request.request_id).await;
        let line = serde_json::to_string(request).expect("encode request");
        self.send_line(&line).await;
        rx
    }

    /// Send and await the terminal frame, collecting progress along the way.
    async fn call(&mut self, request: Request) -> (Vec<Response>, Response) {
        let id = request.request_id.clone();
        let mut rx = self.send(&request).await;
        let mut progress = Vec::new();
        loop {
            let frame = tokio::time::timeout(STEP_TIMEOUT, rx.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting on {id}"))
                .unwrap_or_else(|| panic!("worker stream closed during {id}"));
            match frame {
                Response::Progress { .. } => progress.push(frame),
                terminal => return (progress, terminal),
            }
        }
    }

    fn invoke(
        id: &str,
        plugin: &str,
        operation: &str,
        project: Option<&str>,
        input: Value,
    ) -> Request {
        Request {
            schema_version: PROTOCOL_VERSION,
            request_id: id.into(),
            command: RequestCommand::Invoke {
                plugin: plugin.into(),
                operation: operation.into(),
                project_id: project.map(str::to_string),
                input,
            },
        }
    }

    fn executor(id: &str, method: &str, params: Value) -> Request {
        Self::invoke(
            id,
            "project",
            "executor.call",
            Some("sim"),
            json!({ "method": method, "params": params }),
        )
    }

    async fn ping(&mut self, id: &str) -> Response {
        let (_, terminal) = self
            .call(Request {
                schema_version: PROTOCOL_VERSION,
                request_id: id.into(),
                command: RequestCommand::Ping,
            })
            .await;
        terminal
    }
}

fn result_data(terminal: &Response, context: &str) -> Value {
    match terminal {
        Response::Result { data, .. } => data.clone(),
        other => panic!("{context}: expected a result, got {other:?}"),
    }
}

fn error_code(terminal: &Response, context: &str) -> String {
    match terminal {
        Response::Error { code, .. } => code.clone(),
        other => panic!("{context}: expected an error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Fixture repo + config
// ---------------------------------------------------------------------------

fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "sim")
        .env("GIT_AUTHOR_EMAIL", "sim@example.local")
        .env("GIT_COMMITTER_NAME", "sim")
        .env("GIT_COMMITTER_EMAIL", "sim@example.local")
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn write_config(
    dir: &Path,
    project_root: &Path,
    base_url: &str,
    max_concurrent: usize,
    max_request_bytes: usize,
) -> PathBuf {
    let config = json!({
        "schema_version": 1,
        "worker_name": "sim-worker",
        "projects": [{ "id": "sim", "root": project_root }],
        "trajectory_root": dir.join("trajectory"),
        "enabled_plugins": ["project", "coding"],
        "max_concurrent_requests": max_concurrent,
        "max_request_bytes": max_request_bytes,
        "execution_residency": "remote_worker",
        "provider": {
            "base_url": base_url,
            "model": "sim-model",
            "api_key_env": "SIM_MODEL_KEY",
            "allowed_tools": ["write_file", "bash", "edit_file"],
            "allowed_command_prefixes": ["echo "]
        }
    });
    let path = dir.join("worker-config.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    path
}

/// Decode every `agent_event` progress frame and fold it through the real
/// reducer, asserting stream-level invariants as it goes.
fn fold_events(progress: &[Response]) -> (Snapshot, Vec<AgentEvent>) {
    let mut snapshot = Snapshot::default();
    let mut events = Vec::new();
    let mut last_sequence = None;
    for frame in progress {
        let Response::Progress {
            sequence,
            kind,
            data,
            ..
        } = frame
        else {
            unreachable!("progress vector only holds progress frames");
        };
        if let Some(previous) = last_sequence {
            assert!(
                *sequence > previous,
                "progress sequence regressed: {previous} -> {sequence}"
            );
        }
        last_sequence = Some(*sequence);
        assert_eq!(kind, "agent_event", "unexpected progress kind {kind}");
        let event: AgentEvent =
            serde_json::from_value(data.clone()).expect("agent_event decodes as AgentEvent");
        apply(&mut snapshot, &event);
        events.push(event);
    }
    (snapshot, events)
}

// ---------------------------------------------------------------------------
// The simulation
// ---------------------------------------------------------------------------

#[path = "full_stack_simulation/journey.rs"]
mod journey;
#[path = "full_stack_simulation/saturation.rs"]
mod saturation;
