#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

#[derive(Clone, Debug)]
pub struct CapturedRequest(Value);

impl CapturedRequest {
    pub fn messages_for_role(&self, role: &str) -> Vec<String> {
        self.0
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some(role))
            .filter_map(|message| message_text(message.get("content")?))
            .collect()
    }

    pub fn tool_results(&self) -> Vec<String> {
        self.messages_for_role("tool")
    }
}

fn message_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let parts = content.as_array()?;
    Some(
        parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
    )
}

pub struct GitFixture {
    _temp: tempfile::TempDir,
    pub main: PathBuf,
    pub detached: PathBuf,
    pub spaced: PathBuf,
}

impl GitFixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("temporary Git simulation root");
        let main = temp.path().join("main");
        let detached = temp.path().join("detached");
        let spaced = temp.path().join("linked worktree");
        std::fs::create_dir_all(&main).expect("create main checkout");

        git(&main, &["init", "-q", "--initial-branch=main"]);
        git(&main, &["config", "user.name", "Clark Test"]);
        git(&main, &["config", "user.email", "clark@example.com"]);
        std::fs::write(main.join("tracked.txt"), "main\n").expect("seed tracked file");
        git(&main, &["add", "tracked.txt"]);
        git(&main, &["commit", "-qm", "initial"]);
        git(
            &main,
            &[
                "remote",
                "add",
                "origin",
                "https://token@example.com/Clark/Simulation.git",
            ],
        );
        git(
            &main,
            &[
                "worktree",
                "add",
                "--detach",
                "-q",
                detached.to_str().expect("UTF-8 detached path"),
                "HEAD",
            ],
        );
        git(
            &main,
            &[
                "worktree",
                "add",
                "--detach",
                "-q",
                spaced.to_str().expect("UTF-8 spaced path"),
                "HEAD",
            ],
        );

        Self {
            _temp: temp,
            main: canonical(&main),
            detached: canonical(&detached),
            spaced: canonical(&spaced),
        }
    }

    pub fn make_detached_dirty(&self) {
        std::fs::write(self.detached.join("tracked.txt"), "worktree edit\n")
            .expect("edit tracked file");
        std::fs::write(self.detached.join("untracked.txt"), "new\n").expect("write untracked file");
    }

    #[cfg(unix)]
    pub fn install_hostile_helpers(&self) -> HostileHelpers {
        use std::os::unix::fs::PermissionsExt;

        let marker_dir = self
            .main
            .parent()
            .expect("fixture parent")
            .join("helper-markers");
        std::fs::create_dir_all(&marker_dir).expect("create helper marker directory");
        let fsmonitor_marker = marker_dir.join("fsmonitor-ran");
        let credential_marker = marker_dir.join("credential-ran");
        let fsmonitor = marker_dir.join("fsmonitor.sh");
        let credential = marker_dir.join("credential.sh");
        std::fs::write(
            &fsmonitor,
            format!(
                "#!/bin/sh\ntouch '{}'\nsleep 30\n",
                fsmonitor_marker.display()
            ),
        )
        .expect("write hostile fsmonitor helper");
        std::fs::write(
            &credential,
            format!(
                "#!/bin/sh\ntouch '{}'\nsleep 30\n",
                credential_marker.display()
            ),
        )
        .expect("write hostile credential helper");
        std::fs::set_permissions(&fsmonitor, std::fs::Permissions::from_mode(0o755))
            .expect("make fsmonitor executable");
        std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o755))
            .expect("make credential helper executable");
        git(
            &self.main,
            &[
                "config",
                "core.fsmonitor",
                fsmonitor.to_str().expect("UTF-8 fsmonitor path"),
            ],
        );
        git(
            &self.main,
            &[
                "config",
                "credential.helper",
                credential.to_str().expect("UTF-8 credential path"),
            ],
        );
        HostileHelpers {
            fsmonitor_marker,
            credential_marker,
        }
    }
}

#[cfg(unix)]
pub struct HostileHelpers {
    pub fsmonitor_marker: PathBuf,
    pub credential_marker: PathBuf,
}

pub fn git(cwd: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run Git in simulation");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().expect("canonical fixture path")
}

pub fn tool_call_body(id: &str, name: &str, arguments: Value) -> String {
    let arguments = serde_json::to_string(&arguments).expect("serialize tool arguments");
    let delta = json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": id,
                    "function": {"name": name, "arguments": arguments}
                }]
            }
        }]
    });
    sse([
        delta.to_string(),
        json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}).to_string(),
    ])
}

pub fn final_body(text: &str) -> String {
    sse([
        json!({"choices": [{"delta": {"content": text}}]}).to_string(),
        json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}).to_string(),
    ])
}

fn sse(events: impl IntoIterator<Item = String>) -> String {
    events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .chain(["data: [DONE]\n\n".to_string()])
        .collect()
}

pub async fn scripted_model(bodies: Vec<String>) -> (String, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind scripted model");
    let addr = listener.local_addr().expect("scripted model address");
    let handle = tokio::spawn(async move {
        let mut captured = Vec::with_capacity(bodies.len());
        for body in bodies {
            let (mut socket, _) = listener.accept().await.expect("accept model request");
            captured.push(CapturedRequest(read_request_json(&mut socket).await));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write scripted model response");
            socket.flush().await.expect("flush scripted model response");
        }
        captured
    });
    (format!("http://{addr}/v1"), handle)
}

async fn read_request_json(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut content_length = None;
    loop {
        let count = socket.read(&mut chunk).await.expect("read model request");
        assert_ne!(count, 0, "model request ended before its JSON body");
        bytes.extend_from_slice(&chunk[..count]);
        if content_length.is_none() {
            if let Some(headers_end) = headers_end(&bytes) {
                let headers = String::from_utf8_lossy(&bytes[..headers_end]);
                content_length = headers.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                });
            }
        }
        if let (Some(headers_end), Some(content_length)) = (headers_end(&bytes), content_length) {
            let body_start = headers_end + 4;
            if bytes.len() >= body_start + content_length {
                return serde_json::from_slice(&bytes[body_start..body_start + content_length])
                    .expect("scripted model request JSON");
            }
        }
    }
}

fn headers_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}
