//! `RemoteExecutor` — the desktop-side client that satisfies the [`Executor`]
//! trait by forwarding each primitive to a `clark-exec-server` over the
//! [`exec_protocol`] WebSocket (reached through an `ssh -L` loopback forward).
//!
//! One connection is multiplexed: filesystem ops are request/response correlated
//! by `id`; a command runs as `process/start` then a stream of `process/output`
//! notifications routed (by `process_id`) into the in-flight [`exec`] call until
//! its `process/exit`. The server buffers output by sequence number; if the
//! socket drops, the client reconnects, re-authenticates, and resumes the same
//! process after the last delivered sequence without rerunning the command.

mod background;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use exec_core::{
    BackgroundStatus, DirEntry, ExecOutput, ExecResult, Executor, FileMeta, SystemCapabilityCensus,
    WalkEntry,
};
use exec_protocol::{
    b64_decode, b64_encode, method, AuthParams, AuthResult, CanonicalizeResult, MetaResult,
    Notification, PathParams, ProcessExitParams, ProcessIdParams, ProcessOutputParams,
    ProcessResumeParams, ProcessStartParams, ReadDirResult, ReadResult, RenameParams, Request,
    Response, Stream, SystemCapabilityCensusResult, TargetServiceParams, TargetServiceResult,
    WalkResult, WriteNewResult, WriteParams, PROTOCOL_VERSION,
};
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex, RwLock};
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
    closed: AtomicBool,
    shutdown: CancellationToken,
}

impl Conn {
    /// Send a request and await its response (Err if the server returns an error
    /// or the connection drops).
    async fn call(
        &self,
        method_name: &str,
        params: serde_json::Value,
    ) -> ExecResult<serde_json::Value> {
        if self.closed.load(Ordering::Acquire) {
            return Err("exec-server connection closed".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let req = Request::new(id, method_name, params);
        if self.outgoing.send(text_msg(&req)).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err("exec-server connection closed".into());
        }
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
        self.closed.store(true, Ordering::Release);
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

struct RemoteState {
    url: String,
    token: String,
    conn: RwLock<Arc<Conn>>,
    reconnecting: AsyncMutex<()>,
}

/// An [`Executor`] backed by a remote `clark-exec-server`.
#[derive(Clone)]
pub struct RemoteExecutor {
    state: Arc<RemoteState>,
}

impl RemoteExecutor {
    /// Connect to `url` (`ws://127.0.0.1:<forwarded-port>`) and authenticate with
    /// the session `token`. Fails on a bad token or a protocol-version mismatch.
    pub async fn connect(url: &str, token: &str) -> ExecResult<Self> {
        let conn = open_connection(url, token).await?;
        Ok(Self {
            state: Arc::new(RemoteState {
                url: url.to_string(),
                token: token.to_string(),
                conn: RwLock::new(conn),
                reconnecting: AsyncMutex::new(()),
            }),
        })
    }

    async fn connection(&self) -> ExecResult<Arc<Conn>> {
        let current = self.state.conn.read().await.clone();
        if !current.closed.load(Ordering::Acquire) {
            return Ok(current);
        }
        self.reconnect(&current).await
    }

    async fn reconnect(&self, stale: &Arc<Conn>) -> ExecResult<Arc<Conn>> {
        let _guard = self.state.reconnecting.lock().await;
        let current = self.state.conn.read().await.clone();
        if !Arc::ptr_eq(&current, stale) && !current.closed.load(Ordering::Acquire) {
            return Ok(current);
        }
        let next = open_connection(&self.state.url, &self.state.token).await?;
        *self.state.conn.write().await = next.clone();
        Ok(next)
    }

    #[cfg(test)]
    async fn disconnect_for_test(&self) {
        self.state.conn.read().await.shutdown.cancel();
    }

    fn path(p: &Path) -> String {
        let path = p.to_string_lossy().to_string();
        #[cfg(windows)]
        {
            // A Windows desktop can target either a POSIX SSH host or a native
            // Windows exec server. Preserve drive, UNC, and verbatim paths;
            // only normalize separator-mixed paths that have no Windows
            // prefix and therefore belong to the remote POSIX namespace.
            if matches!(p.components().next(), Some(std::path::Component::Prefix(_))) {
                windows_native_wire_path(&path)
            } else {
                path.replace('\\', "/")
            }
        }
        #[cfg(not(windows))]
        {
            path
        }
    }

    async fn call(
        &self,
        method_name: &str,
        params: serde_json::Value,
    ) -> ExecResult<serde_json::Value> {
        self.connection().await?.call(method_name, params).await
    }

    async fn resume_connection(
        &self,
        stale: &Arc<Conn>,
        process_id: &str,
        after_seq: u64,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> ExecResult<(Arc<Conn>, mpsc::UnboundedReceiver<ProcEvent>, ProcGuard)> {
        let window = timeout.min(Duration::from_secs(5));
        let deadline = tokio::time::Instant::now() + window;
        let mut delay = Duration::from_millis(50);
        let mut previous = stale.clone();
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err("exec-server connection could not be restored".into());
            }
            let attempt = tokio::select! {
                _ = cancel.cancelled() => return Err("command cancelled".into()),
                result = self.reconnect(&previous) => result,
            };
            if let Ok(conn) = attempt {
                let (tx, rx) = mpsc::unbounded_channel();
                conn.procs
                    .lock()
                    .unwrap()
                    .insert(process_id.to_string(), tx);
                let guard = ProcGuard {
                    conn: conn.clone(),
                    process_id: process_id.to_string(),
                };
                match conn
                    .call(
                        method::PROCESS_RESUME,
                        to_value(&ProcessResumeParams {
                            process_id: process_id.to_string(),
                            after_seq,
                        }),
                    )
                    .await
                {
                    Ok(_) => return Ok((conn, rx, guard)),
                    Err(error)
                        if error.contains("connection closed")
                            || error.contains("connecting to exec-server") =>
                    {
                        previous = conn;
                    }
                    Err(error) => return Err(format!("resuming remote command: {error}")),
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err("command cancelled".into()),
                _ = tokio::time::sleep(delay) => {}
            }
            delay = (delay * 2).min(Duration::from_millis(800));
        }
    }

    async fn exec_streaming_mode(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
        on_output: exec_core::OnOutput<'_>,
        pty: bool,
    ) -> ExecResult<ExecOutput> {
        let mut conn = self.connection().await?;
        let process_id = uuid::Uuid::new_v4().to_string();
        let (etx, mut erx) = mpsc::unbounded_channel::<ProcEvent>();
        conn.procs.lock().unwrap().insert(process_id.clone(), etx);
        let mut _guard = ProcGuard {
            conn: conn.clone(),
            process_id: process_id.clone(),
        };

        conn.call(
            method::PROCESS_START,
            to_value(&ProcessStartParams {
                process_id: process_id.clone(),
                command: command.to_string(),
                cwd: Self::path(cwd),
                timeout_ms: timeout.as_millis() as u64,
                pty,
            }),
        )
        .await?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut last_seq = 0;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Ok(active) = self.connection().await {
                        let _ = active
                            .call(
                                method::PROCESS_CANCEL,
                                to_value(&ProcessIdParams { process_id: process_id.clone() }),
                            )
                            .await;
                    }
                    return Err("command cancelled".into());
                }
                ev = erx.recv() => match ev {
                    Some(ProcEvent::Output(p)) => {
                        if p.seq <= last_seq {
                            continue;
                        }
                        last_seq = p.seq;
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
                    None => {
                        let (next, rx, next_guard) = self
                            .resume_connection(&conn, &process_id, last_seq, timeout, cancel)
                            .await?;
                        conn = next;
                        erx = rx;
                        _guard = next_guard;
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn windows_native_wire_path(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = value.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        value.to_string()
    }
}

fn rebase_walk_path(root: &Path, remote_root: &Path, wire_path: &str) -> PathBuf {
    #[cfg(windows)]
    let entry = PathBuf::from(windows_native_wire_path(wire_path));
    #[cfg(not(windows))]
    let entry = PathBuf::from(wire_path);

    entry
        .strip_prefix(remote_root)
        .map_or(entry.clone(), |relative| root.join(relative))
}

async fn open_connection(url: &str, token: &str) -> ExecResult<Arc<Conn>> {
    let websocket_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(exec_core::MAX_EXEC_PROTOCOL_MESSAGE_BYTES))
        .max_frame_size(Some(exec_core::MAX_EXEC_PROTOCOL_MESSAGE_BYTES));
    let (ws, _resp) =
        tokio_tungstenite::connect_async_with_config(url, Some(websocket_config), false)
            .await
            .map_err(|e| format!("connecting to exec-server {url}: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    let (otx, mut orx) = mpsc::unbounded_channel::<Message>();
    let conn = Arc::new(Conn {
        outgoing: otx,
        pending: Mutex::new(HashMap::new()),
        procs: Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(1),
        closed: AtomicBool::new(false),
        shutdown: CancellationToken::new(),
    });

    let writer_conn = conn.clone();
    let writer_shutdown = conn.shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = writer_shutdown.cancelled() => break,
                    message = orx.recv() => match message {
                        Some(message) => {
                            if sink.send(message).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
            }
        }
        let _ = sink.close().await;
        writer_conn.fail_all();
    });

    let reader_conn = conn.clone();
    let reader_shutdown = conn.shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = reader_shutdown.cancelled() => break,
                message = stream.next() => match message {
                    Some(Ok(Message::Text(text))) => route(&reader_conn, text.as_str()),
                    Some(Ok(_)) => {}
                    _ => break,
                }
            }
        }
        reader_conn.fail_all();
    });

    let auth = AuthParams {
        token: token.to_string(),
        protocol_version: PROTOCOL_VERSION,
    };
    let result = conn.call(method::AUTH, to_value(&auth)).await?;
    // Validate the shape (and surface server version on success).
    let _: AuthResult = from_value(result)?;
    Ok(conn)
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
    async fn system_capability_census(&self) -> ExecResult<SystemCapabilityCensus> {
        let value = self
            .call(method::ENV_CAPABILITY_CENSUS, serde_json::json!({}))
            .await?;
        let census: SystemCapabilityCensusResult = from_value(value)?;
        Ok(SystemCapabilityCensus {
            platform: census.platform,
            architecture: census.architecture,
            executable_names: census.executable_names,
            environment_variable_names: census.environment_variable_names,
            credential_surfaces: census.credential_surfaces,
            executables_truncated: census.executables_truncated,
            environment_names_truncated: census.environment_names_truncated,
        })
    }

    fn containment(&self) -> exec_core::ExecutionContainment {
        exec_core::ExecutionContainment::External
    }

    fn is_local(&self) -> bool {
        false
    }

    async fn read(&self, path: &Path) -> ExecResult<Vec<u8>> {
        let v = self
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
        self.call(
            method::FS_WRITE,
            to_value(&WriteParams {
                path: Self::path(path),
                data: b64_encode(data),
            }),
        )
        .await?;
        Ok(())
    }

    async fn write_private(&self, path: &Path, data: &[u8]) -> ExecResult<()> {
        self.call(
            method::FS_WRITE_PRIVATE,
            to_value(&WriteParams {
                path: Self::path(path),
                data: b64_encode(data),
            }),
        )
        .await?;
        Ok(())
    }

    async fn write_private_new(&self, path: &Path, data: &[u8]) -> ExecResult<bool> {
        let value = self
            .call(
                method::FS_WRITE_PRIVATE_NEW,
                to_value(&WriteParams {
                    path: Self::path(path),
                    data: b64_encode(data),
                }),
            )
            .await?;
        let result: WriteNewResult = from_value(value)?;
        Ok(result.created)
    }

    async fn sync_file(&self, path: &Path) -> ExecResult<()> {
        self.call(
            method::FS_SYNC_FILE,
            to_value(&PathParams {
                path: Self::path(path),
            }),
        )
        .await?;
        Ok(())
    }

    async fn sync_directory(&self, path: &Path) -> ExecResult<()> {
        self.call(
            method::FS_SYNC_DIRECTORY,
            to_value(&PathParams {
                path: Self::path(path),
            }),
        )
        .await?;
        Ok(())
    }

    async fn target_service_call(
        &self,
        service: &str,
        root: &Path,
        request: &[u8],
    ) -> ExecResult<Vec<u8>> {
        let value = self
            .call(
                method::TARGET_SERVICE_CALL,
                to_value(&TargetServiceParams {
                    service: service.to_owned(),
                    root: Self::path(root),
                    request: b64_encode(request),
                }),
            )
            .await?;
        let result: TargetServiceResult = from_value(value)?;
        b64_decode(&result.response)
    }

    async fn create_dir_all(&self, path: &Path) -> ExecResult<()> {
        self.call(
            method::FS_CREATE_DIR,
            to_value(&PathParams {
                path: Self::path(path),
            }),
        )
        .await?;
        Ok(())
    }

    async fn remove_file(&self, path: &Path) -> ExecResult<()> {
        self.call(
            method::FS_REMOVE_FILE,
            to_value(&PathParams {
                path: Self::path(path),
            }),
        )
        .await?;
        Ok(())
    }

    async fn remove_dir_all(&self, path: &Path) -> ExecResult<()> {
        self.call(
            method::FS_REMOVE_DIR,
            to_value(&PathParams {
                path: Self::path(path),
            }),
        )
        .await?;
        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> ExecResult<()> {
        self.call(
            method::FS_RENAME,
            to_value(&RenameParams {
                from: Self::path(from),
                to: Self::path(to),
            }),
        )
        .await?;
        Ok(())
    }

    async fn read_dir(&self, path: &Path) -> ExecResult<Vec<DirEntry>> {
        let v = self
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
                is_symlink: e.is_symlink,
            })
            .collect())
    }

    async fn metadata(&self, path: &Path) -> ExecResult<FileMeta> {
        let v = self
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
            is_symlink: m.is_symlink,
        })
    }

    async fn canonicalize(&self, path: &Path) -> ExecResult<PathBuf> {
        let value = self
            .call(
                method::FS_CANONICALIZE,
                to_value(&PathParams {
                    path: Self::path(path),
                }),
            )
            .await?;
        let result: CanonicalizeResult = from_value(value)?;
        Ok(PathBuf::from(result.path))
    }

    async fn home_dir(&self, _cwd: &Path) -> ExecResult<PathBuf> {
        let value = self.call(method::ENV_HOME, serde_json::json!({})).await?;
        let result: CanonicalizeResult = from_value(value)?;
        Ok(PathBuf::from(result.path))
    }

    async fn walk(&self, root: &Path) -> ExecResult<Vec<WalkEntry>> {
        let remote_root = PathBuf::from(Self::path(root));
        let v = self
            .call(
                method::FS_WALK,
                to_value(&PathParams {
                    path: remote_root.to_string_lossy().into_owned(),
                }),
            )
            .await?;
        let r: WalkResult = from_value(v)?;
        Ok(r.entries
            .into_iter()
            .map(|w| WalkEntry {
                path: rebase_walk_path(root, &remote_root, &w.path),
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
        self.exec_streaming_mode(command, cwd, timeout, cancel, on_output, false)
            .await
    }

    async fn exec_streaming_pty(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: &CancellationToken,
        on_output: exec_core::OnOutput<'_>,
    ) -> ExecResult<ExecOutput> {
        self.exec_streaming_mode(command, cwd, timeout, cancel, on_output, true)
            .await
    }

    async fn background_start(&self, command: &str, cwd: &Path) -> ExecResult<String> {
        let conn = self.connection().await?;
        background::start(&conn, command, cwd).await
    }

    async fn background_status(
        &self,
        process_id: &str,
        after_seq: u64,
    ) -> ExecResult<BackgroundStatus> {
        let conn = self.connection().await?;
        background::status(&conn, process_id, after_seq).await
    }

    async fn background_write(&self, process_id: &str, data: &[u8], close: bool) -> ExecResult<()> {
        let conn = self.connection().await?;
        background::write(&conn, process_id, data, close).await
    }

    async fn background_kill(&self, process_id: &str) -> ExecResult<()> {
        let conn = self.connection().await?;
        background::kill(&conn, process_id).await
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn capability_census_round_trips_names_without_values() {
        let dir = tempfile::tempdir().unwrap();
        let server = exec_server::bind(exec_server::Config {
            token: "census-token".into(),
            root: Some(dir.path().to_path_buf()),
            home: Some(dir.path().to_path_buf()),
            addr: "127.0.0.1:0".into(),
        })
        .await
        .unwrap();
        let address = server.local_addr().unwrap();
        tokio::spawn(server.serve());

        let remote = RemoteExecutor::connect(&format!("ws://{address}"), "census-token")
            .await
            .unwrap();
        let census = remote.system_capability_census().await.unwrap();
        assert_eq!(census.platform, std::env::consts::OS);
        assert_eq!(census.architecture, std::env::consts::ARCH);
        assert!(census
            .environment_variable_names
            .iter()
            .all(|name| !name.contains('=')));
        assert!(census
            .executable_names
            .windows(2)
            .all(|pair| pair[0] < pair[1]));
    }

    #[tokio::test]
    async fn streaming_command_resumes_without_duplicate_output_after_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let server = exec_server::bind(exec_server::Config {
            token: "resume-token".into(),
            root: Some(dir.path().to_path_buf()),
            home: None,
            addr: "127.0.0.1:0".into(),
        })
        .await
        .unwrap();
        let address = server.local_addr().unwrap();
        tokio::spawn(server.serve());

        let remote = RemoteExecutor::connect(&format!("ws://{address}"), "resume-token")
            .await
            .unwrap();
        let first_chunk = Arc::new(Notify::new());
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let running = {
            let remote = remote.clone();
            let first_chunk = first_chunk.clone();
            let delivered = delivered.clone();
            let root = dir.path().to_path_buf();
            tokio::spawn(async move {
                remote
                    .exec_streaming(
                        "printf A; sleep 0.25; printf B; sleep 0.25; printf C",
                        &root,
                        Duration::from_secs(5),
                        &CancellationToken::new(),
                        &|is_stderr, bytes| {
                            if !is_stderr {
                                delivered.lock().unwrap().extend_from_slice(bytes);
                                if bytes.contains(&b'A') {
                                    first_chunk.notify_one();
                                }
                            }
                        },
                    )
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(2), first_chunk.notified())
            .await
            .expect("first output arrives");
        remote.disconnect_for_test().await;

        let output = tokio::time::timeout(Duration::from_secs(6), running)
            .await
            .expect("command resumes")
            .unwrap()
            .unwrap();
        assert_eq!(output.stdout, b"ABC");
        assert_eq!(*delivered.lock().unwrap(), b"ABC");
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn wire_paths_preserve_native_windows_prefixes() {
        assert_eq!(
            RemoteExecutor::path(Path::new(r"\\?\C:\Clark QA\repo")),
            r"C:\Clark QA\repo"
        );
        assert_eq!(
            RemoteExecutor::path(Path::new(r"C:\Clark QA\repo")),
            r"C:\Clark QA\repo"
        );
        assert_eq!(
            RemoteExecutor::path(Path::new(r"\\?\UNC\server\share\repo")),
            r"\\server\share\repo"
        );
    }

    #[test]
    fn wire_paths_normalize_non_windows_remote_separators() {
        assert_eq!(
            RemoteExecutor::path(Path::new(r"/home/clark\repo")),
            "/home/clark/repo"
        );
    }

    #[test]
    fn walk_paths_keep_the_callers_verbatim_root_identity() {
        let root = Path::new(r"\\?\C:\Clark QA\repo");
        let remote_root = Path::new(r"C:\Clark QA\repo");
        assert_eq!(
            rebase_walk_path(root, remote_root, r"C:\Clark QA\repo\src\main.rs"),
            root.join(r"src\main.rs")
        );
    }
}
