//! `RemoteExecutor` — the desktop-side client that satisfies the [`Executor`]
//! trait by forwarding each primitive to a `clark-exec-server` over the
//! [`exec_protocol`] WebSocket (reached through an `ssh -L` loopback forward).
//!
//! One connection is multiplexed: filesystem ops are request/response correlated
//! by `id`; a command runs as `process/start` then a stream of `process/output`
//! notifications routed (by `process_id`) into the in-flight [`exec`] call until
//! its `process/exit`. The server buffers output by sequence number, so a future
//! reconnect can `process/resume` — that resilience layer lands with the SSH
//! tunnel (Phase 3); here the connection is single-shot.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use exec_core::{DirEntry, ExecOutput, ExecResult, Executor, FileMeta, WalkEntry};
use exec_protocol::{
    b64_decode, b64_encode, method, AuthParams, AuthResult, MetaResult, Notification, PathParams,
    ProcessExitParams, ProcessIdParams, ProcessOutputParams, ProcessStartParams, ReadDirResult,
    ReadResult, Request, Response, Stream, WalkResult, WriteParams, PROTOCOL_VERSION,
};
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

/// A process notification routed to the [`exec`](Executor::exec) call awaiting it.
enum ProcEvent {
    Output(ProcessOutputParams),
    Exit(ProcessExitParams),
}

/// The multiplexed connection: outgoing frames funnel through `outgoing`;
/// responses resolve `pending`; process notifications fan out via `procs`.
struct Conn {
    outgoing: mpsc::UnboundedSender<Message>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Response>>>,
    procs: Mutex<HashMap<String, mpsc::UnboundedSender<ProcEvent>>>,
    next_id: AtomicU64,
}

impl Conn {
    /// Send a request and await its response (Err if the server returns an error
    /// or the connection drops).
    async fn call(
        &self,
        method_name: &str,
        params: serde_json::Value,
    ) -> ExecResult<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let req = Request::new(id, method_name, params);
        self.outgoing
            .send(text_msg(&req))
            .map_err(|_| "exec-server connection closed".to_string())?;
        match rx.await {
            Ok(resp) => match resp.error {
                Some(err) => Err(err.message),
                None => Ok(resp.result.unwrap_or(serde_json::Value::Null)),
            },
            Err(_) => Err("exec-server connection closed".to_string()),
        }
    }

    /// Tear down on disconnect: every awaiting caller and exec loop gets a clean
    /// "connection closed" instead of hanging forever.
    fn fail_all(&self) {
        self.pending.lock().unwrap().clear();
        self.procs.lock().unwrap().clear();
    }
}

/// Drops the process listener when an `exec` call returns, so the routing map
/// never leaks finished runs.
struct ProcGuard {
    conn: Arc<Conn>,
    process_id: String,
}

impl Drop for ProcGuard {
    fn drop(&mut self) {
        self.conn.procs.lock().unwrap().remove(&self.process_id);
    }
}

/// An [`Executor`] backed by a remote `clark-exec-server`.
pub struct RemoteExecutor {
    conn: Arc<Conn>,
}

impl RemoteExecutor {
    /// Connect to `url` (`ws://127.0.0.1:<forwarded-port>`) and authenticate with
    /// the session `token`. Fails on a bad token or a protocol-version mismatch.
    pub async fn connect(url: &str, token: &str) -> ExecResult<Self> {
        let (ws, _resp) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| format!("connecting to exec-server {url}: {e}"))?;
        let (mut sink, mut stream) = ws.split();

        let (otx, mut orx) = mpsc::unbounded_channel::<Message>();
        tokio::spawn(async move {
            while let Some(m) = orx.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let conn = Arc::new(Conn {
            outgoing: otx,
            pending: Mutex::new(HashMap::new()),
            procs: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });

        let reader_conn = conn.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                if let Message::Text(t) = msg {
                    route(&reader_conn, t.as_str());
                }
            }
            reader_conn.fail_all();
        });

        let exec = RemoteExecutor { conn };
        let auth = AuthParams {
            token: token.to_string(),
            protocol_version: PROTOCOL_VERSION,
        };
        let result = exec.conn.call(method::AUTH, to_value(&auth)).await?;
        // Validate the shape (and surface server version on success).
        let _: AuthResult = from_value(result)?;
        Ok(exec)
    }

    fn path(p: &Path) -> String {
        p.to_string_lossy().to_string()
    }
}

/// Demultiplex an incoming frame: a response resolves its pending call; a
/// notification is delivered to the matching process listener.
fn route(conn: &Conn, text: &str) {
    if let Ok(resp) = serde_json::from_str::<Response>(text) {
        if let Some(tx) = conn.pending.lock().unwrap().remove(&resp.id) {
            let _ = tx.send(resp);
            return;
        }
    }
    let Ok(note) = serde_json::from_str::<Notification>(text) else {
        return;
    };
    match note.method.as_str() {
        method::PROCESS_OUTPUT => {
            if let Ok(p) = serde_json::from_value::<ProcessOutputParams>(note.params) {
                if let Some(tx) = conn.procs.lock().unwrap().get(&p.process_id) {
                    let _ = tx.send(ProcEvent::Output(p));
                }
            }
        }
        method::PROCESS_EXIT => {
            if let Ok(p) = serde_json::from_value::<ProcessExitParams>(note.params) {
                if let Some(tx) = conn.procs.lock().unwrap().get(&p.process_id) {
                    let _ = tx.send(ProcEvent::Exit(p));
                }
            }
        }
        _ => {}
    }
}

#[async_trait]
impl Executor for RemoteExecutor {
    fn is_local(&self) -> bool {
        false
    }

    async fn read(&self, path: &Path) -> ExecResult<Vec<u8>> {
        let v = self
            .conn
            .call(
                method::FS_READ,
                to_value(&PathParams {
                    path: Self::path(path),
                }),
            )
            .await?;
        let r: ReadResult = from_value(v)?;
        b64_decode(&r.data)
    }

    async fn write(&self, path: &Path, data: &[u8]) -> ExecResult<()> {
        self.conn
            .call(
                method::FS_WRITE,
                to_value(&WriteParams {
                    path: Self::path(path),
                    data: b64_encode(data),
                }),
            )
            .await?;
        Ok(())
    }

    async fn create_dir_all(&self, path: &Path) -> ExecResult<()> {
        self.conn
            .call(
                method::FS_CREATE_DIR,
                to_value(&PathParams {
                    path: Self::path(path),
                }),
            )
            .await?;
        Ok(())
    }

    async fn read_dir(&self, path: &Path) -> ExecResult<Vec<DirEntry>> {
        let v = self
            .conn
            .call(
                method::FS_READ_DIR,
                to_value(&PathParams {
                    path: Self::path(path),
                }),
            )
            .await?;
        let r: ReadDirResult = from_value(v)?;
        Ok(r.entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                is_dir: e.is_dir,
            })
            .collect())
    }

    async fn metadata(&self, path: &Path) -> ExecResult<FileMeta> {
        let v = self
            .conn
            .call(
                method::FS_METADATA,
                to_value(&PathParams {
                    path: Self::path(path),
                }),
            )
            .await?;
        let m: MetaResult = from_value(v)?;
        Ok(FileMeta {
            modified: m.modified_ms.map(ms_to_time),
            len: m.len,
            is_dir: m.is_dir,
        })
    }

    async fn walk(&self, root: &Path) -> ExecResult<Vec<WalkEntry>> {
        let v = self
            .conn
            .call(
                method::FS_WALK,
                to_value(&PathParams {
                    path: Self::path(root),
                }),
            )
            .await?;
        let r: WalkResult = from_value(v)?;
        Ok(r.entries
            .into_iter()
            .map(|w| WalkEntry {
                path: PathBuf::from(w.path),
                modified: w.modified_ms.map(ms_to_time),
                len: w.len,
            })
            .collect())
    }

    async fn exec(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> ExecResult<ExecOutput> {
        self.exec_streaming(command, cwd, timeout, cancel, &|_, _| {})
            .await
    }

    /// The server already streams `process/output` notifications as the command
    /// runs — forward each chunk to `on_output` instead of only accumulating,
    /// so the UI can show live output over the tunnel.
    async fn exec_streaming(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
        on_output: exec_core::OnOutput<'_>,
    ) -> ExecResult<ExecOutput> {
        let process_id = uuid::Uuid::new_v4().to_string();
        let (etx, mut erx) = mpsc::unbounded_channel::<ProcEvent>();
        self.conn
            .procs
            .lock()
            .unwrap()
            .insert(process_id.clone(), etx);
        // Removes the listener no matter how this call returns.
        let _guard = ProcGuard {
            conn: self.conn.clone(),
            process_id: process_id.clone(),
        };

        self.conn
            .call(
                method::PROCESS_START,
                to_value(&ProcessStartParams {
                    process_id: process_id.clone(),
                    command: command.to_string(),
                    cwd: Self::path(cwd),
                    timeout_ms: timeout.as_millis() as u64,
                }),
            )
            .await?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    // Best-effort kill on the remote; mirror LocalExecutor's message.
                    let _ = self
                        .conn
                        .call(
                            method::PROCESS_CANCEL,
                            to_value(&ProcessIdParams { process_id: process_id.clone() }),
                        )
                        .await;
                    return Err("command cancelled".into());
                }
                ev = erx.recv() => match ev {
                    Some(ProcEvent::Output(p)) => {
                        let bytes = b64_decode(&p.data).unwrap_or_default();
                        match p.stream {
                            Stream::Stdout => {
                                on_output(false, &bytes);
                                stdout.extend_from_slice(&bytes);
                            }
                            Stream::Stderr => {
                                on_output(true, &bytes);
                                stderr.extend_from_slice(&bytes);
                            }
                        }
                    }
                    Some(ProcEvent::Exit(p)) => {
                        return match p.error {
                            Some(err) => Err(err),
                            None => Ok(ExecOutput { stdout, stderr, code: p.code }),
                        };
                    }
                    None => return Err("exec-server connection closed".into()),
                }
            }
        }
    }
}

fn text_msg<T: Serialize>(v: &T) -> Message {
    Message::Text(serde_json::to_string(v).unwrap_or_default().into())
}

fn to_value<T: Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

fn from_value<T: DeserializeOwned>(v: serde_json::Value) -> ExecResult<T> {
    serde_json::from_value(v).map_err(|e| format!("malformed exec-server response: {e}"))
}

fn ms_to_time(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}
