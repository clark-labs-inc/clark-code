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

mod process;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use exec_core::{Executor, LocalExecutor};
use exec_protocol::{
    b64_decode, b64_encode, error_code, method, AuthParams, AuthResult, CanonicalizeResult,
    MetaResult, PathParams, ReadDirResult, ReadResult, RenameParams, Request, Response, WalkResult,
    WireDirEntry, WireWalkEntry, WriteParams, PROTOCOL_VERSION,
};
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
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
    /// Optional target-user home exposed to filesystem primitives for
    /// compatible skill/instruction discovery and Clark-managed user packs.
    /// Model file tools remain project-confined by the provider sandbox.
    pub home: Option<PathBuf>,
    /// Listen address; use `127.0.0.1:0` to bind an ephemeral loopback port.
    pub addr: String,
}

/// State shared across every connection (processes outlive the socket that
/// started them, so the registry lives here, not per-connection).
struct Shared {
    config: Config,
    procs: Mutex<HashMap<String, Arc<process::ProcShared>>>,
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
        | method::FS_REMOVE_FILE
        | method::FS_REMOVE_DIR
        | method::FS_RENAME
        | method::FS_READ_DIR
        | method::FS_METADATA
        | method::FS_CANONICALIZE
        | method::FS_WALK
        | method::ENV_HOME => {
            let resp = match fs_dispatch(&req.method, req.params, fs, &shared.config).await {
                Ok(v) => Response::ok(id, v),
                Err((code, msg)) => Response::err(id, code, msg),
            };
            let _ = tx.send(text_msg(&resp));
        }
        method::PROCESS_START => process::handle_start(id, req.params, shared, tx, conn_token),
        method::PROCESS_RESUME => process::handle_resume(id, req.params, shared, tx, conn_token),
        method::PROCESS_STATUS => process::handle_status(id, req.params, shared, tx),
        method::PROCESS_INPUT => process::handle_input(id, req.params, shared, tx),
        method::PROCESS_CANCEL => process::handle_cancel(id, req.params, shared, tx),
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
    config: &Config,
) -> Result<serde_json::Value, (i64, String)> {
    match method_name {
        method::ENV_HOME => {
            let home = config
                .home
                .clone()
                .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                .ok_or_else(|| {
                    (
                        error_code::EXEC_FAILED,
                        "target environment has no HOME directory".to_string(),
                    )
                })?;
            let home = fs.canonicalize(&home).await.unwrap_or(home);
            Ok(to_value(&CanonicalizeResult {
                path: home.to_string_lossy().into_owned(),
            }))
        }
        method::FS_READ => {
            let p: PathParams = parse(params)?;
            let path = checked_fs_path(&p.path, config)?;
            let bytes = fs.read(&path).await.map_err(exec_err)?;
            Ok(to_value(&ReadResult {
                data: b64_encode(&bytes),
            }))
        }
        method::FS_WRITE => {
            let p: WriteParams = parse(params)?;
            let path = checked_fs_path(&p.path, config)?;
            let data = b64_decode(&p.data).map_err(|e| (error_code::INVALID_PARAMS, e))?;
            fs.write(&path, &data).await.map_err(exec_err)?;
            Ok(serde_json::json!({}))
        }
        method::FS_CREATE_DIR => {
            let p: PathParams = parse(params)?;
            let path = checked_fs_path(&p.path, config)?;
            fs.create_dir_all(&path).await.map_err(exec_err)?;
            Ok(serde_json::json!({}))
        }
        method::FS_REMOVE_FILE => {
            let p: PathParams = parse(params)?;
            let path = checked_fs_path(&p.path, config)?;
            fs.remove_file(&path).await.map_err(exec_err)?;
            Ok(serde_json::json!({}))
        }
        method::FS_REMOVE_DIR => {
            let p: PathParams = parse(params)?;
            let path = checked_fs_path(&p.path, config)?;
            fs.remove_dir_all(&path).await.map_err(exec_err)?;
            Ok(serde_json::json!({}))
        }
        method::FS_RENAME => {
            let p: RenameParams = parse(params)?;
            let from = checked_fs_path(&p.from, config)?;
            let to = checked_fs_path(&p.to, config)?;
            fs.rename(&from, &to).await.map_err(exec_err)?;
            Ok(serde_json::json!({}))
        }
        method::FS_READ_DIR => {
            let p: PathParams = parse(params)?;
            let path = checked_fs_path(&p.path, config)?;
            let entries = fs.read_dir(&path).await.map_err(exec_err)?;
            Ok(to_value(&ReadDirResult {
                entries: entries
                    .into_iter()
                    .map(|e| WireDirEntry {
                        name: e.name,
                        is_dir: e.is_dir,
                        is_symlink: e.is_symlink,
                    })
                    .collect(),
            }))
        }
        method::FS_METADATA => {
            let p: PathParams = parse(params)?;
            let path = checked_fs_path(&p.path, config)?;
            let m = fs.metadata(&path).await.map_err(exec_err)?;
            Ok(to_value(&MetaResult {
                modified_ms: m.modified.and_then(to_ms),
                len: m.len,
                is_dir: m.is_dir,
                is_symlink: m.is_symlink,
            }))
        }
        method::FS_CANONICALIZE => {
            let p: PathParams = parse(params)?;
            let path = checked_fs_path(&p.path, config)?;
            let canonical = fs.canonicalize(&path).await.map_err(exec_err)?;
            if !canonical_allowed(fs, &canonical, config).await {
                return Err((
                    error_code::EXEC_FAILED,
                    format!(
                        "canonical path escapes the remote project and home roots: {}",
                        canonical.display()
                    ),
                ));
            }
            Ok(to_value(&CanonicalizeResult {
                path: canonical.to_string_lossy().to_string(),
            }))
        }
        method::FS_WALK => {
            let p: PathParams = parse(params)?;
            let root_path = checked_fs_path(&p.path, config)?;
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
    if norm.starts_with(root) || resolved_within(&norm, root) {
        Ok(norm)
    } else {
        Err((
            error_code::EXEC_FAILED,
            format!("path escapes project root: {path}"),
        ))
    }
}

fn checked_fs_path(path: &str, config: &Config) -> Result<PathBuf, (i64, String)> {
    let normalized = lexically_normalize(&PathBuf::from(path));
    if config.root.is_none() && config.home.is_none()
        || config
            .root
            .as_ref()
            .is_some_and(|root| normalized.starts_with(root))
        || config
            .home
            .as_ref()
            .is_some_and(|home| normalized.starts_with(home))
        || [config.root.as_ref(), config.home.as_ref()]
            .into_iter()
            .flatten()
            .any(|root| resolved_within(&normalized, root))
    {
        return Ok(normalized);
    }
    Err((
        error_code::EXEC_FAILED,
        format!("path escapes project root or target home: {path}"),
    ))
}

fn resolved_within(path: &Path, root: &Path) -> bool {
    let Ok(canonical_root) = std::fs::canonicalize(root) else {
        return false;
    };
    let mut existing = path;
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            return false;
        };
        existing = parent;
    }
    std::fs::canonicalize(existing).is_ok_and(|canonical| canonical.starts_with(canonical_root))
}

async fn canonical_allowed(fs: &LocalExecutor, path: &Path, config: &Config) -> bool {
    if config.root.is_none() && config.home.is_none() {
        return true;
    }
    for root in [config.root.as_ref(), config.home.as_ref()]
        .into_iter()
        .flatten()
    {
        if fs
            .canonicalize(root)
            .await
            .is_ok_and(|canonical_root| path.starts_with(canonical_root))
        {
            return true;
        }
    }
    false
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
