//! `clark-exec-server` — the remote half of Clark Code's remote-projects feature.
//!
//! It is, almost literally, "[`LocalExecutor`] wrapped in a WebSocket server":
//! it binds a **loopback** port on the remote host, speaks the [`exec_protocol`]
//! JSON-RPC dialect, and serves every request by delegating to the same
//! [`exec_core`] primitives the desktop runs locally. The desktop reaches it
//! only through an `ssh -L` port-forward, so the loopback bind + the SSH tunnel
//! are the transport security; a per-session capability **token** (checked on
//! the opening `auth` request) is defense-in-depth against another local user on
//! the remote poking the forwarded port.
//!
//! Filesystem ops are request/response. A command runs as
//! `process/start` → streamed `process/output` notifications → one
//! `process/exit`. The server **buffers output by sequence number**, so if the
//! tunnel drops mid-build the desktop can reconnect, re-`auth`, and
//! `process/resume` from the last `seq` it saw — no lost output, no rerun.
//!
//! The crate is intentionally dependency-light (no HTTP client, no `agent-core`)
//! so the cross-compiled remote binary stays small.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use exec_core::{Executor, LocalExecutor};
use exec_protocol::{
    b64_decode, b64_encode, error_code, method, AuthParams, AuthResult, MetaResult, Notification,
    PathParams, ProcessExitParams, ProcessIdParams, ProcessOutputParams, ProcessResumeParams,
    ProcessStartParams, ReadDirResult, ReadResult, Request, Response, Stream, WalkResult,
    WireDirEntry, WireWalkEntry, WriteParams, PROTOCOL_VERSION,
};
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::sync::CancellationToken;

/// Outgoing-frame channel: every producer (request responses, output streamers)
/// funnels `Message`s to the single per-connection writer task.
type Outbound = mpsc::UnboundedSender<Message>;
type WsRead = futures::stream::SplitStream<WebSocketStream<TcpStream>>;

/// How the server was launched.
#[derive(Clone)]
pub struct Config {
    /// Per-session capability token the client must present on `auth`.
    pub token: String,
    /// If set, every path op is confined to this root (lexical containment).
    /// `None` disables the check (used by in-process tests).
    pub root: Option<PathBuf>,
    /// Listen address; use `127.0.0.1:0` to bind an ephemeral loopback port.
    pub addr: String,
}

/// State shared across every connection (processes outlive the socket that
/// started them, so the registry lives here, not per-connection).
struct Shared {
    config: Config,
    procs: Mutex<HashMap<String, Arc<ProcShared>>>,
}

/// A bound, not-yet-serving server. Call [`Server::local_addr`] to learn the
/// chosen port, then [`Server::serve`].
pub struct Server {
    listener: TcpListener,
    shared: Arc<Shared>,
}

/// Bind the configured address.
pub async fn bind(config: Config) -> std::io::Result<Server> {
    let listener = TcpListener::bind(&config.addr).await?;
    Ok(Server {
        listener,
        shared: Arc::new(Shared {
            config,
            procs: Mutex::new(HashMap::new()),
        }),
    })
}

impl Server {
    /// The actual bound address (resolves the `:0` ephemeral port).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accept connections until the listener errors fatally. One task per
    /// connection; a panicking connection never takes down the server.
    pub async fn serve(self) {
        loop {
            match self.listener.accept().await {
                Ok((stream, _peer)) => {
                    let shared = self.shared.clone();
                    tokio::spawn(handle_conn(stream, shared));
                }
                // Transient per-accept errors (e.g. fd exhaustion) shouldn't kill
                // the loop; back off a tick and keep serving.
                Err(_e) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }
}

async fn handle_conn(stream: TcpStream, shared: Arc<Shared>) {
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(w) => w,
        Err(_) => return,
    };
    let (mut sink, mut read) = ws.split();

    // One writer task owns the sink; everything else sends through `tx`.
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // Streamers spawned for this connection are scoped to it; cancel on drop so
    // they stop trying to write to a dead socket (the process keeps running).
    let conn_token = CancellationToken::new();

    if authenticate(&mut read, &tx, &shared.config).await {
        let fs = LocalExecutor;
        while let Some(Ok(msg)) = read.next().await {
            let text = match msg {
                Message::Text(t) => t,
                Message::Close(_) => break,
                _ => continue,
            };
            let req: Request = match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(_) => continue,
            };
            handle_request(req, &shared, &fs, &tx, &conn_token).await;
        }
    }

    conn_token.cancel();
    drop(tx);
    let _ = writer.await;
}

/// The connection must open with a valid `auth`; anything else closes it.
async fn authenticate(read: &mut WsRead, tx: &Outbound, config: &Config) -> bool {
    while let Some(Ok(msg)) = read.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(_) | Message::Pong(_) => continue,
            _ => return false,
        };
        let req: Request = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(_) => return false,
        };
        if req.method != method::AUTH {
            let _ = tx.send(text_msg(&Response::err(
                req.id,
                error_code::UNAUTHORIZED,
                "the first request must be `auth`",
            )));
            return false;
        }
        let params: AuthParams = match serde_json::from_value(req.params) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(text_msg(&Response::err(
                    req.id,
                    error_code::INVALID_PARAMS,
                    format!("invalid auth params: {e}"),
                )));
                return false;
            }
        };
        if params.protocol_version != PROTOCOL_VERSION {
            let _ = tx.send(text_msg(&Response::err(
                req.id,
                error_code::UNAUTHORIZED,
                format!(
                    "protocol version mismatch (server {PROTOCOL_VERSION}, client {})",
                    params.protocol_version
                ),
            )));
            return false;
        }
        if !constant_time_eq(params.token.as_bytes(), config.token.as_bytes()) {
            let _ = tx.send(text_msg(&Response::err(
                req.id,
                error_code::UNAUTHORIZED,
                "invalid token",
            )));
            return false;
        }
        let result = AuthResult {
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        let _ = tx.send(text_msg(&Response::ok(req.id, to_value(&result))));
        return true;
    }
    false
}

async fn handle_request(
    req: Request,
    shared: &Arc<Shared>,
    fs: &LocalExecutor,
    tx: &Outbound,
    conn_token: &CancellationToken,
) {
    let id = req.id;
    match req.method.as_str() {
        // Re-auth on an already-authenticated socket is a harmless no-op.
        method::AUTH => {
            let _ = tx.send(text_msg(&Response::ok(id, serde_json::json!({}))));
        }
        method::FS_READ
        | method::FS_WRITE
        | method::FS_CREATE_DIR
        | method::FS_READ_DIR
        | method::FS_METADATA
        | method::FS_WALK => {
            let resp = match fs_dispatch(&req.method, req.params, fs, &shared.config.root).await {
                Ok(v) => Response::ok(id, v),
                Err((code, msg)) => Response::err(id, code, msg),
            };
            let _ = tx.send(text_msg(&resp));
        }
        method::PROCESS_START => handle_start(id, req.params, shared, tx, conn_token),
        method::PROCESS_RESUME => handle_resume(id, req.params, shared, tx, conn_token),
        method::PROCESS_CANCEL => handle_cancel(id, req.params, shared, tx),
        other => {
            let _ = tx.send(text_msg(&Response::err(
                id,
                error_code::METHOD_NOT_FOUND,
                format!("unknown method: {other}"),
            )));
        }
    }
}

async fn fs_dispatch(
    method_name: &str,
    params: serde_json::Value,
    fs: &LocalExecutor,
    root: &Option<PathBuf>,
) -> Result<serde_json::Value, (i64, String)> {
    match method_name {
        method::FS_READ => {
            let p: PathParams = parse(params)?;
            let path = checked_path(&p.path, root)?;
            let bytes = fs.read(&path).await.map_err(exec_err)?;
            Ok(to_value(&ReadResult {
                data: b64_encode(&bytes),
            }))
        }
        method::FS_WRITE => {
            let p: WriteParams = parse(params)?;
            let path = checked_path(&p.path, root)?;
            let data = b64_decode(&p.data).map_err(|e| (error_code::INVALID_PARAMS, e))?;
            fs.write(&path, &data).await.map_err(exec_err)?;
            Ok(serde_json::json!({}))
        }
        method::FS_CREATE_DIR => {
            let p: PathParams = parse(params)?;
            let path = checked_path(&p.path, root)?;
            fs.create_dir_all(&path).await.map_err(exec_err)?;
            Ok(serde_json::json!({}))
        }
        method::FS_READ_DIR => {
            let p: PathParams = parse(params)?;
            let path = checked_path(&p.path, root)?;
            let entries = fs.read_dir(&path).await.map_err(exec_err)?;
            Ok(to_value(&ReadDirResult {
                entries: entries
                    .into_iter()
                    .map(|e| WireDirEntry {
                        name: e.name,
                        is_dir: e.is_dir,
                    })
                    .collect(),
            }))
        }
        method::FS_METADATA => {
            let p: PathParams = parse(params)?;
            let path = checked_path(&p.path, root)?;
            let m = fs.metadata(&path).await.map_err(exec_err)?;
            Ok(to_value(&MetaResult {
                modified_ms: m.modified.and_then(to_ms),
                len: m.len,
                is_dir: m.is_dir,
            }))
        }
        method::FS_WALK => {
            let p: PathParams = parse(params)?;
            let root_path = checked_path(&p.path, root)?;
            let entries = fs.walk(&root_path).await.map_err(exec_err)?;
            Ok(to_value(&WalkResult {
                entries: entries
                    .into_iter()
                    .map(|w| WireWalkEntry {
                        path: w.path.to_string_lossy().to_string(),
                        modified_ms: w.modified.and_then(to_ms),
                        len: w.len,
                    })
                    .collect(),
            }))
        }
        _ => Err((error_code::METHOD_NOT_FOUND, "unknown fs method".into())),
    }
}

// ---- process registry + streaming ------------------------------------------

struct ProcShared {
    process_id: String,
    state: Mutex<ProcState>,
    /// Wakes streamers when output is appended or the process exits.
    tick: broadcast::Sender<()>,
    /// Cancels the running process (`process/cancel`).
    cancel: CancellationToken,
}

#[derive(Default)]
struct ProcState {
    output: Vec<ProcessOutputParams>,
    exit: Option<ProcessExitParams>,
}

fn handle_start(
    id: u64,
    params: serde_json::Value,
    shared: &Arc<Shared>,
    tx: &Outbound,
    conn_token: &CancellationToken,
) {
    let p: ProcessStartParams = match parse(params) {
        Ok(p) => p,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };
    let cwd = match checked_path(&p.cwd, &shared.config.root) {
        Ok(c) => c,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };

    let (tick, _) = broadcast::channel(16);
    let proc = Arc::new(ProcShared {
        process_id: p.process_id.clone(),
        state: Mutex::new(ProcState::default()),
        tick,
        cancel: CancellationToken::new(),
    });

    {
        let mut procs = shared.procs.lock().unwrap();
        if procs.contains_key(&p.process_id) {
            let _ = tx.send(text_msg(&Response::err(
                id,
                error_code::EXEC_FAILED,
                "process_id already in use",
            )));
            return;
        }
        procs.insert(p.process_id.clone(), proc.clone());
    }

    let _ = tx.send(text_msg(&Response::ok(id, serde_json::json!({}))));

    tokio::spawn(run_process(
        proc.clone(),
        shared.clone(),
        p.command,
        cwd,
        Duration::from_millis(p.timeout_ms),
    ));
    spawn_streamer(proc, tx.clone(), 0, conn_token.clone());
}

fn handle_resume(
    id: u64,
    params: serde_json::Value,
    shared: &Arc<Shared>,
    tx: &Outbound,
    conn_token: &CancellationToken,
) {
    let p: ProcessResumeParams = match parse(params) {
        Ok(p) => p,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };
    let proc = shared.procs.lock().unwrap().get(&p.process_id).cloned();
    match proc {
        Some(proc) => {
            let _ = tx.send(text_msg(&Response::ok(id, serde_json::json!({}))));
            spawn_streamer(proc, tx.clone(), p.after_seq, conn_token.clone());
        }
        None => {
            let _ = tx.send(text_msg(&Response::err(
                id,
                error_code::UNKNOWN_PROCESS,
                "unknown or expired process",
            )));
        }
    }
}

fn handle_cancel(id: u64, params: serde_json::Value, shared: &Arc<Shared>, tx: &Outbound) {
    let p: ProcessIdParams = match parse(params) {
        Ok(p) => p,
        Err((code, msg)) => {
            let _ = tx.send(text_msg(&Response::err(id, code, msg)));
            return;
        }
    };
    if let Some(proc) = shared.procs.lock().unwrap().get(&p.process_id).cloned() {
        proc.cancel.cancel();
    }
    let _ = tx.send(text_msg(&Response::ok(id, serde_json::json!({}))));
}

/// Replays buffered output past `after_seq` to one connection, then follows the
/// process live until it exits. Scoped to the connection via `conn_token`.
fn spawn_streamer(
    proc: Arc<ProcShared>,
    tx: Outbound,
    after_seq: u64,
    conn_token: CancellationToken,
) {
    tokio::spawn(async move {
        let mut rx = proc.tick.subscribe();
        let mut cursor = after_seq;
        loop {
            let (chunks, exit) = {
                let st = proc.state.lock().unwrap();
                let chunks: Vec<ProcessOutputParams> = st
                    .output
                    .iter()
                    .filter(|c| c.seq > cursor)
                    .cloned()
                    .collect();
                (chunks, st.exit.clone())
            };
            for c in chunks {
                cursor = c.seq;
                if tx
                    .send(text_msg(&Notification::new(
                        method::PROCESS_OUTPUT,
                        to_value(&c),
                    )))
                    .is_err()
                {
                    return;
                }
            }
            if let Some(ex) = exit {
                if ex.seq > cursor {
                    let _ = tx.send(text_msg(&Notification::new(
                        method::PROCESS_EXIT,
                        to_value(&ex),
                    )));
                }
                return;
            }
            tokio::select! {
                _ = conn_token.cancelled() => return,
                _ = rx.recv() => {} // tick or lag: re-drain from cursor either way
            }
        }
    });
}

async fn run_process(
    proc: Arc<ProcShared>,
    shared: Arc<Shared>,
    command: String,
    cwd: PathBuf,
    timeout: Duration,
) {
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(&command)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            append_exit(&proc, None, Some(format!("failed to spawn shell: {e}")));
            schedule_gc(shared, proc.process_id.clone());
            return;
        }
    };

    let (otx, mut orx) = mpsc::channel::<(Stream, Vec<u8>)>(64);
    let outcome = tokio::spawn(pump(child, otx, proc.cancel.clone(), timeout));

    // Append output as it streams; the channel closes once both readers hit EOF.
    while let Some((stream, data)) = orx.recv().await {
        append_output(&proc, stream, data);
    }
    match outcome.await {
        Ok(Outcome::Exited(code)) => append_exit(&proc, code, None),
        Ok(Outcome::Error(msg)) => append_exit(&proc, None, Some(msg)),
        Err(_) => append_exit(&proc, None, Some("process task panicked".into())),
    }
    schedule_gc(shared, proc.process_id.clone());
}

enum Outcome {
    Exited(Option<i32>),
    Error(String),
}

/// Owns the child: streams its pipes to `otx`, and races completion against
/// cancel/timeout. Returns once the child is reaped.
async fn pump(
    mut child: tokio::process::Child,
    otx: mpsc::Sender<(Stream, Vec<u8>)>,
    cancel: CancellationToken,
    timeout: Duration,
) -> Outcome {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let r1 = tokio::spawn(read_stream(stdout, Stream::Stdout, otx.clone()));
    let r2 = tokio::spawn(read_stream(stderr, Stream::Stderr, otx));

    // `wait_fut` borrows `child`; confine it to this block so we can kill below.
    let interrupted = {
        let wait_fut = std::pin::pin!(child.wait());
        tokio::select! {
            status = wait_fut => {
                let _ = r1.await;
                let _ = r2.await;
                return match status {
                    Ok(s) => Outcome::Exited(s.code()),
                    Err(e) => Outcome::Error(format!("command failed: {e}")),
                };
            }
            _ = cancel.cancelled() => "command cancelled".to_string(),
            _ = tokio::time::sleep(timeout) =>
                format!("command timed out after {} ms", timeout.as_millis()),
        }
    };
    let _ = child.start_kill();
    let _ = child.wait().await;
    let _ = r1.await;
    let _ = r2.await;
    Outcome::Error(interrupted)
}

async fn read_stream<R>(reader: Option<R>, stream: Stream, otx: mpsc::Sender<(Stream, Vec<u8>)>)
where
    R: AsyncReadExt + Unpin,
{
    let Some(mut reader) = reader else { return };
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if otx.send((stream, buf[..n].to_vec())).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn append_output(proc: &ProcShared, stream: Stream, data: Vec<u8>) {
    {
        let mut st = proc.state.lock().unwrap();
        let seq = st.output.len() as u64 + 1;
        st.output.push(ProcessOutputParams {
            process_id: proc.process_id.clone(),
            seq,
            stream,
            data: b64_encode(&data),
        });
    }
    let _ = proc.tick.send(());
}

fn append_exit(proc: &ProcShared, code: Option<i32>, error: Option<String>) {
    {
        let mut st = proc.state.lock().unwrap();
        // One past the last output seq: the terminal cursor value.
        let seq = st.output.len() as u64 + 1;
        st.exit = Some(ProcessExitParams {
            process_id: proc.process_id.clone(),
            seq,
            code,
            error,
        });
    }
    let _ = proc.tick.send(());
}

/// Keep a finished process around briefly so a reconnecting client can still
/// `process/resume` and collect its tail + exit, then free the buffer.
fn schedule_gc(shared: Arc<Shared>, process_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        shared.procs.lock().unwrap().remove(&process_id);
    });
}

// ---- small helpers ----------------------------------------------------------

fn text_msg<T: Serialize>(v: &T) -> Message {
    Message::Text(serde_json::to_string(v).unwrap_or_default().into())
}

fn to_value<T: Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

fn parse<T: DeserializeOwned>(v: serde_json::Value) -> Result<T, (i64, String)> {
    serde_json::from_value(v)
        .map_err(|e| (error_code::INVALID_PARAMS, format!("invalid params: {e}")))
}

fn exec_err(e: String) -> (i64, String) {
    (error_code::EXEC_FAILED, e)
}

fn to_ms(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// Lexically confine `path` to `root` (resolves `.`/`..` without touching the
/// filesystem). Defense-in-depth: the desktop's sandbox already contained it.
fn checked_path(path: &str, root: &Option<PathBuf>) -> Result<PathBuf, (i64, String)> {
    let p = PathBuf::from(path);
    let Some(root) = root else { return Ok(p) };
    let norm = lexically_normalize(&p);
    if norm.starts_with(root) {
        Ok(norm)
    } else {
        Err((
            error_code::EXEC_FAILED,
            format!("path escapes project root: {path}"),
        ))
    }
}

fn lexically_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Length-independent byte compare so token checks don't leak via timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
