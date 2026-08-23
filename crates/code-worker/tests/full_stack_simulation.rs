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
/// given SSE body. Requests beyond the script fail the test via a typed error
/// body, never a hang.
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

fn final_answer_turn(text: &str) -> ScriptedTurn {
    tool_call_turn("final_answer", &json!({ "content": text }))
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
            let turn = script.lock().await.pop_front().unwrap_or(ScriptedTurn {
                stall: Duration::ZERO,
                body: sse(&[json!({"choices":[{"delta":{"tool_calls":[{
                    "index": 0,
                    "id": "call-overflow",
                    "function": {"name": "final_answer",
                        "arguments": json!({"content": "SCRIPT EXHAUSTED"}).to_string()},
                }]}}]})]),
            });
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

#[tokio::test(flavor = "multi_thread")]
async fn full_stack_worker_simulation() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-q", "--initial-branch=main"]);
    std::fs::write(project.join("README.md"), "sim project\n").unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/lib.rs"), "pub fn sim() {}\n").unwrap();
    std::fs::write(project.join("docs.md"), "guide\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "initial"]);
    let project = project.canonicalize().unwrap();

    let model = spawn_model(vec![
        // Journey turn 1: write a file (permission-gated -> respond flow).
        tool_call_turn(
            "write_file",
            &json!({ "path": "notes/plan.md", "content": "# Plan\nstep one\n" }),
        ),
        // Journey turn 2: an auto-approved shell command.
        tool_call_turn("bash", &json!({ "command": "echo built > build.log" })),
        // Journey turn 3: settle.
        final_answer_turn("Wrote the plan and produced build.log."),
        // Cancel phase: a completion that stalls until after the cancel lands.
        ScriptedTurn {
            stall: Duration::from_secs(20),
            body: sse(&[]),
        },
        // Post-cancel health turn: settle immediately.
        final_answer_turn("Recovered after cancellation."),
    ]);

    let config = write_config(temp.path(), &project, &model.base_url, 8, 1024 * 1024);
    let mut worker = WorkerClient::spawn(&config).await;

    // ------------------------------------------------------------------ boot
    let pong = worker.ping("boot-ping").await;
    let pong = result_data(&pong, "boot ping");
    assert_eq!(
        pong["execution_residency"],
        json!("remote_worker"),
        "the ping receipt proves tool/model residency: {pong}"
    );

    // ------------------------------------------------- executor wire surface
    let (_, walk) = worker
        .call(WorkerClient::executor(
            "walk-bounded",
            "fs/walk",
            json!({ "path": project.to_string_lossy(), "max_entries": 1 }),
        ))
        .await;
    let walk = result_data(&walk, "bounded walk");
    assert_eq!(walk["truncated"], json!(true));
    assert_eq!(walk["entries"].as_array().unwrap().len(), 1);

    // A real secret outside the root: the read must fail as a TYPED caller
    // error, and no variant of the response may carry the bytes.
    std::fs::write(
        project.parent().unwrap().join("outside.txt"),
        "TOP-SECRET-OUTSIDE",
    )
    .unwrap();
    let (_, escape) = worker
        .call(WorkerClient::executor(
            "confinement-escape",
            "fs/read",
            json!({ "path": project.join("../outside.txt").to_string_lossy() }),
        ))
        .await;
    let escape_code = error_code(&escape, "parent-dir escape");
    assert_eq!(
        escape_code, "invalid_input",
        "escapes must be typed rejections"
    );
    assert!(
        !serde_json::to_string(&escape)
            .unwrap()
            .contains("TOP-SECRET"),
        "confinement failure leaked file content"
    );

    // --------------------------------------------------------- open session
    let (_, opened) = worker
        .call(WorkerClient::invoke(
            "open-1",
            "coding",
            "session.open",
            Some("sim"),
            json!({ "session_id": "sim-session" }),
        ))
        .await;
    let opened = result_data(&opened, "session.open");
    assert_eq!(opened["session_id"], json!("sim-session"));

    // ------------------------------------------------ the full working turn
    // The prompt streams; write_file raises a permission request that we must
    // answer over the wire from a SECOND in-flight request while the first is
    // still streaming. Drive both by hand rather than through `call`.
    let prompt = WorkerClient::invoke(
        "prompt-1",
        "coding",
        "session.prompt",
        Some("sim"),
        json!({
            "session_id": "sim-session",
            "input": { "blocks": [{ "type": "text", "text": "write the plan file, then build" }] }
        }),
    );
    let mut prompt_rx = worker.send(&prompt).await;

    let mut progress: Vec<Response> = Vec::new();
    let mut responded_permission = false;
    let terminal = loop {
        let frame = tokio::time::timeout(STEP_TIMEOUT, prompt_rx.recv())
            .await
            .expect("prompt frame within budget")
            .expect("prompt stream open");
        match frame {
            Response::Progress { .. } => {
                progress.push(frame);
                if responded_permission {
                    continue;
                }
                let (snapshot, _) = fold_events(&progress);
                if let Some(pending) = snapshot.pending_permission {
                    // Choose the allow-once option, exactly as the desktop does.
                    let option = pending
                        .options
                        .iter()
                        .find(|option| option.kind == PermissionOptionKind::AllowOnce)
                        .unwrap_or_else(|| pending.options.first().expect("options exist"));
                    let respond = WorkerClient::invoke(
                        "respond-1",
                        "coding",
                        "session.respond",
                        Some("sim"),
                        json!({
                            "session_id": "sim-session",
                            "response": {
                                "kind": "permission",
                                "request": pending.id,
                                "option": option.id,
                            }
                        }),
                    );
                    let (_, respond_terminal) = worker.call(respond).await;
                    result_data(&respond_terminal, "session.respond");
                    responded_permission = true;
                }
            }
            terminal => break terminal,
        }
    };

    assert!(
        responded_permission,
        "the gated write_file never raised a permission request"
    );
    let outcome = result_data(&terminal, "prompt terminal");
    assert_eq!(outcome["outcome"]["status"], json!("done"), "{outcome}");
    assert!(
        outcome.get("snapshot").is_none(),
        "terminal frames must not ship the whole conversation"
    );

    // Cross-layer truth: the tools really ran in the real repository.
    assert_eq!(
        std::fs::read_to_string(project.join("notes/plan.md")).unwrap(),
        "# Plan\nstep one\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("build.log"))
            .unwrap()
            .trim(),
        "built"
    );

    // Stream-level invariants over the whole run.
    let (snapshot, events) = fold_events(&progress);
    let started = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::RunStarted { .. }))
        .count();
    let finished = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::RunFinished { .. }))
        .count();
    assert_eq!(
        (started, finished),
        (1, 1),
        "exactly one run start and finish"
    );
    assert!(
        matches!(events.last(), Some(AgentEvent::RunFinished { .. })),
        "the terminal event closes the stream"
    );
    let checkpoint = events.iter().find_map(|event| match event {
        AgentEvent::Checkpoint { id, .. } => Some(id.clone()),
        _ => None,
    });
    let checkpoint = checkpoint.expect("a git repo run must take a checkpoint");
    let refs = git(&project, &["for-each-ref", "refs/agent/checkpoints/"]);
    assert!(
        refs.contains(&checkpoint),
        "checkpoint ref must exist in the real repository: {refs}"
    );
    assert!(snapshot.pending_permission.is_none(), "gate resolved");
    assert!(
        snapshot.timeline.iter().any(|item| {
            serde_json::to_string(item)
                .unwrap()
                .contains("Wrote the plan")
        }),
        "final answer reached the projected timeline"
    );

    // The scripted model saw the tool results round-trip.
    let requests = model.requests.lock().await;
    assert!(requests.len() >= 3, "three journey completions expected");
    let second = serde_json::to_string(&requests[1]).unwrap();
    assert!(
        second.contains("notes/plan.md") || second.contains("plan.md"),
        "turn 2 must carry the write_file result back to the model"
    );
    drop(requests);

    // ------------------------------------------------------------ cancel run
    let stall_prompt = WorkerClient::invoke(
        "prompt-cancel",
        "coding",
        "session.prompt",
        Some("sim"),
        json!({
            "session_id": "sim-session",
            "input": { "blocks": [{ "type": "text", "text": "this one gets cancelled" }] }
        }),
    );
    let mut cancel_rx = worker.send(&stall_prompt).await;
    // Give the request time to reach the stalled completion.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let cancel = Request {
        schema_version: PROTOCOL_VERSION,
        request_id: "cancel-1".into(),
        command: RequestCommand::Cancel {
            target_request_id: "prompt-cancel".into(),
        },
    };
    let (_, cancel_ack) = worker.call(cancel).await;
    result_data(&cancel_ack, "cancel acknowledged");
    let cancelled = loop {
        let frame = tokio::time::timeout(STEP_TIMEOUT, cancel_rx.recv())
            .await
            .expect("cancel terminal within budget")
            .expect("cancel stream open");
        match frame {
            Response::Progress { .. } => continue,
            terminal => break terminal,
        }
    };
    assert_eq!(
        error_code(&cancelled, "cancelled prompt"),
        "cancelled",
        "cancellation is a typed terminal"
    );

    // The session survives cancellation.
    let (_, after) = worker
        .call(WorkerClient::invoke(
            "prompt-after-cancel",
            "coding",
            "session.prompt",
            Some("sim"),
            json!({
                "session_id": "sim-session",
                "input": { "blocks": [{ "type": "text", "text": "are you alive" }] }
            }),
        ))
        .await;
    let after = result_data(&after, "post-cancel prompt");
    assert_eq!(after["outcome"]["status"], json!("done"));

    // ----------------------------------------------- crash + receipt replay
    let (_, first_read) = worker
        .call(WorkerClient::executor(
            "replay-1",
            "fs/read",
            json!({ "path": project.join("README.md").to_string_lossy() }),
        ))
        .await;
    let first_read = result_data(&first_read, "pre-crash read");

    let stderr_before = worker.stderr.lock().await.clone();
    assert!(
        !stderr_before.contains("panicked"),
        "worker panicked during the journey:\n{stderr_before}"
    );
    worker.child.start_kill().expect("kill worker");
    let _ = worker.child.wait().await;

    let mut worker = WorkerClient::spawn(&config).await;
    result_data(&worker.ping("post-restart-ping").await, "restart ping");

    // Same id, same params: the durable receipt replays the exact result.
    let (_, replayed) = worker
        .call(WorkerClient::executor(
            "replay-1",
            "fs/read",
            json!({ "path": project.join("README.md").to_string_lossy() }),
        ))
        .await;
    let replayed = result_data(&replayed, "replayed read");
    assert_eq!(
        replayed, first_read,
        "a restarted worker must replay the identical receipt"
    );

    // Same id, different params: typed conflict, not silent re-execution.
    let (_, conflict) = worker
        .call(WorkerClient::executor(
            "replay-1",
            "fs/read",
            json!({ "path": project.join("build.log").to_string_lossy() }),
        ))
        .await;
    assert_eq!(
        error_code(&conflict, "conflicting replay"),
        "request_id_conflict"
    );

    // Sessions are process state: after a crash the old session is gone and
    // says so in a typed way (the desktop reopens through session recovery).
    let (_, stale) = worker
        .call(WorkerClient::invoke(
            "prompt-stale",
            "coding",
            "session.prompt",
            Some("sim"),
            json!({
                "session_id": "sim-session",
                "input": { "blocks": [{ "type": "text", "text": "hello?" }] }
            }),
        ))
        .await;
    let stale_code = error_code(&stale, "stale session prompt");
    assert_eq!(stale_code, "invalid_input");

    // -------------------------------------------------------------- shutdown
    let shutdown = Request {
        schema_version: PROTOCOL_VERSION,
        request_id: "shutdown-1".into(),
        command: RequestCommand::Shutdown,
    };
    let (_, down) = worker.call(shutdown).await;
    result_data(&down, "shutdown acknowledged");
    let status = tokio::time::timeout(STEP_TIMEOUT, worker.child.wait())
        .await
        .expect("worker exits after shutdown")
        .expect("worker exit status");
    assert!(status.success(), "clean shutdown exit: {status:?}");

    let stderr = worker.stderr.lock().await.clone();
    assert!(
        !stderr.contains("panicked"),
        "restarted worker panicked:\n{stderr}"
    );
}

/// Saturation behavior needs a single-permit worker: a full worker must keep
/// answering health pings (or the desktop replaces it mid-work), refuse extra
/// work with a typed `busy`, and survive an oversized request line.
#[tokio::test(flavor = "multi_thread")]
async fn saturated_worker_stays_healthy_and_survives_bad_input() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("README.md"), "saturation fixture\n").unwrap();
    let project = project.canonicalize().unwrap();

    let model = spawn_model(vec![ScriptedTurn {
        stall: Duration::from_secs(4),
        body: sse(&[json!({"choices":[{"delta":{"tool_calls":[{
            "index": 0,
            "id": "call-final",
            "function": {"name": "final_answer",
                "arguments": json!({"content": "slow but done"}).to_string()},
        }]}}]})]),
    }]);
    // One permit and a small request bound, so both edges are reachable.
    let config = write_config(temp.path(), &project, &model.base_url, 1, 8 * 1024);
    let mut worker = WorkerClient::spawn(&config).await;

    let (_, opened) = worker
        .call(WorkerClient::invoke(
            "open-sat",
            "coding",
            "session.open",
            Some("sim"),
            json!({ "session_id": "sat-session" }),
        ))
        .await;
    result_data(&opened, "session.open");

    // Occupy the only permit with the stalled prompt.
    let prompt = WorkerClient::invoke(
        "prompt-sat",
        "coding",
        "session.prompt",
        Some("sim"),
        json!({
            "session_id": "sat-session",
            "input": { "blocks": [{ "type": "text", "text": "take your time" }] }
        }),
    );
    let mut prompt_rx = worker.send(&prompt).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1. Health pings must answer while saturated — a busy worker is alive.
    let pong = worker.ping("sat-ping").await;
    result_data(&pong, "ping under saturation");

    // 2. Real work beyond the permit is refused with a typed busy.
    let (_, busy) = worker
        .call(WorkerClient::executor(
            "sat-read",
            "fs/read",
            json!({ "path": project.join("README.md").to_string_lossy() }),
        ))
        .await;
    assert_eq!(error_code(&busy, "extra work while saturated"), "busy");

    // 3. An oversized request line is that caller's error, not a process exit.
    let oversized = format!(
        "{{\"schema_version\":{PROTOCOL_VERSION},\"request_id\":\"huge\",\"junk\":\"{}\"}}",
        "x".repeat(64 * 1024)
    );
    let mut huge_rx = worker.open_route("").await;
    worker.send_line(&oversized).await;
    let refusal = tokio::time::timeout(STEP_TIMEOUT, huge_rx.recv())
        .await
        .expect("oversized refusal within budget")
        .expect("stream open");
    assert_eq!(
        error_code(&refusal, "oversized request"),
        "request_too_large"
    );

    // The stalled prompt still completes afterwards: nothing was killed.
    let settled = loop {
        let frame = tokio::time::timeout(STEP_TIMEOUT, prompt_rx.recv())
            .await
            .expect("stalled prompt settles")
            .expect("prompt stream open");
        match frame {
            Response::Progress { .. } => continue,
            terminal => break terminal,
        }
    };
    let settled = result_data(&settled, "stalled prompt terminal");
    assert_eq!(settled["outcome"]["status"], json!("done"));

    let stderr = worker.stderr.lock().await.clone();
    assert!(
        !stderr.contains("panicked"),
        "worker panicked under saturation:\n{stderr}"
    );
}
