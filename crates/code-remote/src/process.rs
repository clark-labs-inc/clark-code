use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use code_host::{Request, Response, PROTOCOL_VERSION};
use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::artifact::{shq, RemoteArtifact, RemoteArtifactError};
use crate::spec::RemoteWorkerSpec;
use crate::transport::{SshTransport, SshTransportError};

const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const STDERR_LIMIT_BYTES: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const CHILD_EXIT_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_millis(750);

const PROGRESS_BUFFER: usize = 256;

type Pending = Arc<Mutex<HashMap<String, PendingRequest>>>;

struct PendingRequest {
    frames: mpsc::Sender<Result<RemoteWorkerFrame, RemoteWorkerError>>,
    next_sequence: u64,
}

#[derive(Clone, Debug)]
pub struct RemoteWorkerProgress {
    pub sequence: u64,
    pub kind: String,
    pub data: serde_json::Value,
}

#[derive(Clone, Debug)]
pub enum RemoteWorkerFrame {
    Progress(RemoteWorkerProgress),
    Terminal(Response),
}

/// Stable native attachment to the registry's current worker process. Session
/// providers and project executors retain this slot, so an atomic reconnect
/// cannot leave them talking to the superseded SSH child.
pub struct RemoteWorkerSlot {
    current: RwLock<Arc<RemoteWorker>>,
}

impl RemoteWorkerSlot {
    pub fn new(worker: Arc<RemoteWorker>) -> Self {
        Self {
            current: RwLock::new(worker),
        }
    }

    pub async fn current(&self) -> Arc<RemoteWorker> {
        self.current.read().await.clone()
    }

    pub async fn replace(&self, worker: Arc<RemoteWorker>) -> Arc<RemoteWorker> {
        std::mem::replace(&mut *self.current.write().await, worker)
    }

    pub async fn health_check(&self) -> Result<(), RemoteWorkerError> {
        self.current().await.health_check().await
    }

    pub async fn request(&self, request: Request) -> Result<Response, RemoteWorkerError> {
        self.current().await.request(request).await
    }

    pub async fn start_request(
        &self,
        request: Request,
    ) -> Result<RemoteWorkerRequest, RemoteWorkerError> {
        self.current().await.start_request(request).await
    }

    pub async fn disconnect(&self) -> Result<(), RemoteWorkerError> {
        self.current().await.disconnect().await
    }
}

/// One correlated in-flight request. Progress and its single terminal response
/// share a bounded channel, preserving order without a WebView-side queue.
pub struct RemoteWorkerRequest {
    request_id: String,
    frames: mpsc::Receiver<Result<RemoteWorkerFrame, RemoteWorkerError>>,
    pending: Pending,
    finished: bool,
}

impl Drop for RemoteWorkerRequest {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let request_id = self.request_id.clone();
        let pending = self.pending.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                pending.lock().await.remove(&request_id);
            });
        }
    }
}

impl RemoteWorkerRequest {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub async fn next(&mut self) -> Result<RemoteWorkerFrame, RemoteWorkerError> {
        match tokio::time::timeout(REQUEST_TIMEOUT, self.frames.recv()).await {
            Ok(Some(Ok(frame @ RemoteWorkerFrame::Terminal(_)))) => {
                self.finished = true;
                Ok(frame)
            }
            Ok(Some(Ok(frame @ RemoteWorkerFrame::Progress(_)))) => Ok(frame),
            Ok(Some(Err(error))) => {
                self.finished = true;
                Err(error)
            }
            Ok(None) => {
                self.finished = true;
                Err(RemoteWorkerError::Disconnected(self.request_id.clone()))
            }
            Err(_) => {
                self.pending.lock().await.remove(&self.request_id);
                self.finished = true;
                Err(RemoteWorkerError::Timeout(self.request_id.clone()))
            }
        }
    }
}

#[derive(Clone)]
pub struct RemoteWorker {
    info: RemoteWorkerInfo,
    stdin: Arc<Mutex<BufWriter<ChildStdin>>>,
    pending: Pending,
    child: Arc<Mutex<Child>>,
    reader: Arc<Mutex<Option<JoinHandle<()>>>>,
    stderr_reader: Arc<Mutex<Option<JoinHandle<()>>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    transport: Arc<SshTransport>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWorkerInfo {
    pub host: String,
    pub arch: String,
    pub ssh_transport: String,
    pub remote_binary: String,
    pub binary_sha256: String,
    pub remote_config: String,
    pub worker: String,
    pub worker_version: String,
    pub advisor_version: String,
    pub execution_residency: String,
}

impl RemoteWorker {
    pub async fn connect(spec: RemoteWorkerSpec) -> Result<Self, RemoteWorkerError> {
        let mut credentials = HashMap::new();
        for env in &spec.credential_envs {
            let value = std::env::var(env)
                .map_err(|_| RemoteWorkerError::CredentialMissing(env.to_string()))?;
            credentials.insert(env.clone(), value);
        }
        Self::connect_with_credentials(spec, credentials).await
    }

    /// Connect with credentials already held by a trusted native host. Values
    /// still cross only the bounded SSH stdin bootstrap and are never written
    /// to config, argv, logs, or the remote filesystem.
    pub async fn connect_with_credentials(
        spec: RemoteWorkerSpec,
        credentials: HashMap<String, String>,
    ) -> Result<Self, RemoteWorkerError> {
        let credentials = credentials
            .into_iter()
            .map(|(name, value)| (name, Zeroizing::new(value)))
            .collect::<HashMap<_, _>>();
        spec.validate()
            .map_err(|error| RemoteWorkerError::Spec(error.to_string()))?;
        if credentials.len() != spec.credential_envs.len()
            || spec
                .credential_envs
                .iter()
                .any(|name| !credentials.contains_key(name))
        {
            return Err(RemoteWorkerError::Spec(
                "credential values must exactly match credential_envs".into(),
            ));
        }
        let mut credential_values = Vec::with_capacity(spec.credential_envs.len());
        for env in &spec.credential_envs {
            let value = credentials
                .get(env)
                .expect("credential names were checked")
                .clone();
            if value.is_empty() || value.len() > 4096 || value.contains(['\n', '\r']) {
                return Err(RemoteWorkerError::CredentialInvalid(env.to_string()));
            }
            credential_values.push(value);
        }
        let config = spec
            .config_bytes()
            .map_err(|error| RemoteWorkerError::Spec(error.to_string()))?;
        let transport = SshTransport::connect(&spec.host).await?;
        let artifact = match RemoteArtifact::prepare(&spec, &config, &transport).await {
            Ok(artifact) => artifact,
            Err(error) => {
                let _ = transport.shutdown().await;
                return Err(error.into());
            }
        };
        let remote_command = remote_command(&artifact, &spec.credential_envs);
        let mut command = transport.worker_command();
        command
            .arg(&remote_command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = transport.shutdown().await;
                return Err(error.into());
            }
        };
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = transport.shutdown().await;
                return Err(RemoteWorkerError::Protocol(
                    "remote worker stdin unavailable".into(),
                ));
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = transport.shutdown().await;
                return Err(RemoteWorkerError::Protocol(
                    "remote worker stdout unavailable".into(),
                ));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = transport.shutdown().await;
                return Err(RemoteWorkerError::Protocol(
                    "remote worker stderr unavailable".into(),
                ));
            }
        };
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let stderr_store = Arc::new(Mutex::new(Vec::new()));
        let reader = spawn_reader(stdout, pending.clone());
        let stderr_reader = tokio::spawn(drain_stderr(stderr, stderr_store.clone()));
        let mut worker = Self {
            info: RemoteWorkerInfo {
                host: spec.host.clone(),
                arch: artifact.arch.slug().into(),
                ssh_transport: "control_master".into(),
                remote_binary: artifact.binary_path.clone(),
                binary_sha256: artifact.binary_sha256.clone(),
                remote_config: artifact.config_path.clone(),
                worker: String::new(),
                worker_version: String::new(),
                advisor_version: String::new(),
                execution_residency: String::new(),
            },
            stdin: Arc::new(Mutex::new(BufWriter::new(stdin))),
            pending,
            child: Arc::new(Mutex::new(child)),
            reader: Arc::new(Mutex::new(Some(reader))),
            stderr_reader: Arc::new(Mutex::new(Some(stderr_reader))),
            stderr: stderr_store,
            transport,
        };
        if !credential_values.is_empty() {
            let mut stdin = worker.stdin.lock().await;
            let write_result = async {
                for value in credential_values {
                    stdin.write_all(value.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                }
                stdin.flush().await
            }
            .await;
            if let Err(error) = write_result {
                drop(stdin);
                return cleanup_failed(worker, error.into()).await;
            }
        }
        let ping = Request {
            schema_version: code_host::PROTOCOL_VERSION,
            request_id: format!("ping-{}", uuid::Uuid::new_v4().simple()),
            command: code_host::RequestCommand::Ping,
        };
        let response = match worker.request(ping).await {
            Ok(response) => response,
            Err(error) => return cleanup_failed(worker, error).await,
        };
        let data = match response {
            Response::Result { kind, data, .. } if kind == "pong" => data,
            Response::Error { code, message, .. } => {
                return cleanup_failed(
                    worker,
                    RemoteWorkerError::Protocol(format!("worker {code}: {message}")),
                )
                .await;
            }
            other => {
                return cleanup_failed(
                    worker,
                    RemoteWorkerError::Protocol(format!("unexpected ping response: {other:?}")),
                )
                .await;
            }
        };
        worker.info.worker = data
            .get("worker")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into();
        worker.info.worker_version = data
            .get("worker_version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into();
        worker.info.advisor_version = data
            .get("advisor_version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into();
        worker.info.execution_residency = data
            .get("execution_residency")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into();
        if worker.info.worker.is_empty() || worker.info.worker_version.is_empty() {
            return cleanup_failed(
                worker,
                RemoteWorkerError::Protocol("worker ping did not prove identity".into()),
            )
            .await;
        }
        if worker.info.execution_residency != "remote_worker" {
            return cleanup_failed(
                worker,
                RemoteWorkerError::Protocol(
                    "remote worker ping did not prove remote_worker residency".into(),
                ),
            )
            .await;
        }
        Ok(worker)
    }

    pub fn info(&self) -> &RemoteWorkerInfo {
        &self.info
    }

    /// Cheap liveness receipt for registry reuse. This exercises the existing
    /// correlated transport and worker loop without reprovisioning SSH state.
    pub async fn health_check(&self) -> Result<(), RemoteWorkerError> {
        let response = tokio::time::timeout(
            HEALTH_CHECK_TIMEOUT,
            self.request(Request {
                schema_version: PROTOCOL_VERSION,
                request_id: format!("health-{}", uuid::Uuid::new_v4().simple()),
                command: code_host::RequestCommand::Ping,
            }),
        )
        .await
        .map_err(|_| RemoteWorkerError::Timeout("health_check".into()))??;
        match response {
            Response::Result { kind, .. } if kind == "pong" => Ok(()),
            Response::Error { code, message, .. } => Err(RemoteWorkerError::Protocol(format!(
                "worker health {code}: {message}"
            ))),
            other => Err(RemoteWorkerError::Protocol(format!(
                "unexpected worker health response: {other:?}"
            ))),
        }
    }

    pub async fn request(&self, request: Request) -> Result<Response, RemoteWorkerError> {
        let mut request = self.start_request(request).await?;
        loop {
            match request.next().await? {
                RemoteWorkerFrame::Progress(_) => {}
                RemoteWorkerFrame::Terminal(response) => return Ok(response),
            }
        }
    }

    pub async fn start_request(
        &self,
        request: Request,
    ) -> Result<RemoteWorkerRequest, RemoteWorkerError> {
        if request.schema_version != PROTOCOL_VERSION {
            return Err(RemoteWorkerError::UnsupportedSchema(request.schema_version));
        }
        if !portable_request_id(&request.request_id) {
            return Err(RemoteWorkerError::InvalidRequestId);
        }
        let request_id = request.request_id.clone();
        let (sender, receiver) = mpsc::channel(PROGRESS_BUFFER);
        let mut pending = self.pending.lock().await;
        if pending.contains_key(&request_id) {
            return Err(RemoteWorkerError::DuplicateRequestId(request_id));
        }
        pending.insert(
            request_id.clone(),
            PendingRequest {
                frames: sender,
                next_sequence: 0,
            },
        );
        drop(pending);
        let mut line = serde_json::to_vec(&request)?;
        line.push(b'\n');
        if line.len() > MAX_LINE_BYTES {
            self.pending.lock().await.remove(&request_id);
            return Err(RemoteWorkerError::RequestTooLarge);
        }
        if let Err(error) = async {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(&line).await?;
            stdin.flush().await
        }
        .await
        {
            self.pending.lock().await.remove(&request_id);
            return Err(error.into());
        }
        Ok(RemoteWorkerRequest {
            request_id,
            frames: receiver,
            pending: self.pending.clone(),
            finished: false,
        })
    }

    pub async fn disconnect(&self) -> Result<(), RemoteWorkerError> {
        let shutdown = Request {
            schema_version: code_host::PROTOCOL_VERSION,
            request_id: format!("shutdown-{}", uuid::Uuid::new_v4().simple()),
            command: code_host::RequestCommand::Shutdown,
        };
        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, self.request(shutdown)).await;
        if let Some(reader) = self.reader.lock().await.take() {
            reader.abort();
        }
        let status = match tokio::time::timeout(CHILD_EXIT_TIMEOUT, async {
            self.child.lock().await.wait().await
        })
        .await
        {
            Ok(status) => status?,
            Err(_) => {
                let mut child = self.child.lock().await;
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = self.transport.shutdown().await;
                return Err(RemoteWorkerError::ShutdownTimeout);
            }
        };
        if let Some(stderr_reader) = self.stderr_reader.lock().await.take() {
            let _ = stderr_reader.await;
        }
        let transport_result = self.transport.shutdown().await;
        if status.success() {
            transport_result.map_err(Into::into)
        } else {
            Err(RemoteWorkerError::ProcessExit(status.code()))
        }
    }

    pub async fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().await).into_owned()
    }
}

impl Drop for RemoteWorker {
    fn drop(&mut self) {
        if Arc::strong_count(&self.child) == 1 {
            if let Ok(mut child) = self.child.try_lock() {
                let _ = child.start_kill();
            }
        }
        if Arc::strong_count(&self.reader) == 1 {
            if let Ok(mut reader) = self.reader.try_lock() {
                if let Some(reader) = reader.take() {
                    reader.abort();
                }
            }
        }
        if Arc::strong_count(&self.stderr_reader) == 1 {
            if let Ok(mut stderr_reader) = self.stderr_reader.try_lock() {
                if let Some(stderr_reader) = stderr_reader.take() {
                    stderr_reader.abort();
                }
            }
        }
    }
}

fn spawn_reader(stdout: tokio::process::ChildStdout, pending: Pending) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::with_capacity(4096);
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line).await {
                Ok(0) => break,
                Ok(_) if line.len() > MAX_LINE_BYTES => {
                    fail_pending(&pending, RemoteWorkerError::ResponseTooLarge).await;
                    break;
                }
                Ok(_) => {
                    while matches!(line.last(), Some(b'\n' | b'\r')) {
                        line.pop();
                    }
                    let response = match serde_json::from_slice::<Response>(&line) {
                        Ok(response) => response,
                        Err(error) => {
                            fail_pending(&pending, RemoteWorkerError::Protocol(error.to_string()))
                                .await;
                            break;
                        }
                    };
                    let schema_version = response.schema_version();
                    if schema_version != PROTOCOL_VERSION {
                        fail_pending(
                            &pending,
                            RemoteWorkerError::UnsupportedSchema(schema_version),
                        )
                        .await;
                        break;
                    }
                    let Some(request_id) = response.request_id().map(str::to_string) else {
                        fail_pending(
                            &pending,
                            RemoteWorkerError::Protocol("response missing request_id".into()),
                        )
                        .await;
                        break;
                    };
                    deliver_response(&pending, request_id, response).await;
                }
                Err(error) => {
                    fail_pending(
                        &pending,
                        RemoteWorkerError::Protocol(format!("remote worker read failed: {error}")),
                    )
                    .await;
                    break;
                }
            }
        }
        fail_pending(
            &pending,
            RemoteWorkerError::Disconnected("remote worker EOF".into()),
        )
        .await;
    })
}

async fn fail_pending(pending: &Pending, error: RemoteWorkerError) {
    let requests = pending
        .lock()
        .await
        .drain()
        .map(|(_, request)| request)
        .collect::<Vec<_>>();
    for request in requests {
        let _ = request.frames.try_send(Err(error.clone()));
    }
}

async fn deliver_response(pending: &Pending, request_id: String, response: Response) {
    let mut pending = pending.lock().await;
    let Some(request) = pending.get_mut(&request_id) else {
        // A timed-out or explicitly abandoned request may finish late. It has
        // no authority to attach to another request and can be discarded by id.
        return;
    };
    let frame = match response {
        Response::Progress {
            sequence,
            kind,
            data,
            ..
        } => {
            if sequence != request.next_sequence {
                let request = pending
                    .remove(&request_id)
                    .expect("pending request existed");
                let error = RemoteWorkerError::Protocol(format!(
                    "progress sequence mismatch for {request_id}: expected {}, received {sequence}",
                    request.next_sequence
                ));
                drop(pending);
                let _ = request.frames.send(Err(error)).await;
                return;
            }
            request.next_sequence = request.next_sequence.saturating_add(1);
            RemoteWorkerFrame::Progress(RemoteWorkerProgress {
                sequence,
                kind,
                data,
            })
        }
        terminal @ (Response::Result { .. } | Response::Error { .. }) => {
            let request = pending
                .remove(&request_id)
                .expect("pending request existed");
            drop(pending);
            let _ = request
                .frames
                .send(Ok(RemoteWorkerFrame::Terminal(terminal)))
                .await;
            return;
        }
    };
    match request.frames.try_send(Ok(frame)) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            let request = pending
                .remove(&request_id)
                .expect("pending request existed");
            drop(pending);
            let _ = request
                .frames
                .send(Err(RemoteWorkerError::Backpressure(request_id)))
                .await;
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            pending.remove(&request_id);
        }
    }
}

async fn drain_stderr(mut stderr: tokio::process::ChildStderr, store: Arc<Mutex<Vec<u8>>>) {
    let mut bytes = Vec::new();
    let _ = stderr.read_to_end(&mut bytes).await;
    bytes.truncate(STDERR_LIMIT_BYTES);
    *store.lock().await = bytes;
}

#[derive(Clone, Debug, Error)]
pub enum RemoteWorkerError {
    #[error("remote worker I/O failed: {0}")]
    Io(String),
    #[error("remote worker JSON failed: {0}")]
    Json(String),
    #[error("remote worker spec invalid: {0}")]
    Spec(String),
    #[error("remote worker artifact failed: {0}")]
    Artifact(String),
    #[error("remote worker SSH transport failed: {0}")]
    Transport(String),
    #[error("remote worker startup failed: {0}")]
    Startup(String),
    #[error("remote worker credential environment variable is not set: {0}")]
    CredentialMissing(String),
    #[error("remote worker credential environment variable contains an invalid value: {0}")]
    CredentialInvalid(String),
    #[error("remote worker request is too large")]
    RequestTooLarge,
    #[error("remote worker response is too large")]
    ResponseTooLarge,
    #[error("remote worker protocol failure: {0}")]
    Protocol(String),
    #[error("remote worker used unsupported protocol schema: {0}")]
    UnsupportedSchema(u32),
    #[error("remote worker request id is invalid")]
    InvalidRequestId,
    #[error("remote worker request id is already pending: {0}")]
    DuplicateRequestId(String),
    #[error("remote worker disconnected while waiting for {0}")]
    Disconnected(String),
    #[error("remote worker request timed out: {0}")]
    Timeout(String),
    #[error("remote worker progress consumer fell behind: {0}")]
    Backpressure(String),
    #[error("remote worker exited unsuccessfully: {0:?}")]
    ProcessExit(Option<i32>),
    #[error("remote worker did not exit within the shutdown deadline")]
    ShutdownTimeout,
}

impl RemoteWorkerError {
    pub fn category(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Json(_) => "json",
            Self::Spec(_) => "spec",
            Self::Artifact(_) => "artifact",
            Self::Transport(_) => "transport",
            Self::Startup(_) => "startup",
            Self::CredentialMissing(_) => "credential_missing",
            Self::CredentialInvalid(_) => "credential_invalid",
            Self::RequestTooLarge => "request_too_large",
            Self::ResponseTooLarge => "response_too_large",
            Self::Protocol(_) => "protocol",
            Self::UnsupportedSchema(_) => "unsupported_schema",
            Self::InvalidRequestId => "invalid_request_id",
            Self::DuplicateRequestId(_) => "duplicate_request_id",
            Self::Disconnected(_) => "disconnected",
            Self::Timeout(_) => "timeout",
            Self::Backpressure(_) => "backpressure",
            Self::ProcessExit(_) => "process_exit",
            Self::ShutdownTimeout => "shutdown_timeout",
        }
    }
}

impl From<std::io::Error> for RemoteWorkerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for RemoteWorkerError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error.to_string())
    }
}

impl From<RemoteArtifactError> for RemoteWorkerError {
    fn from(error: RemoteArtifactError) -> Self {
        Self::Artifact(error.to_string())
    }
}

impl From<SshTransportError> for RemoteWorkerError {
    fn from(error: SshTransportError) -> Self {
        Self::Transport(error.to_string())
    }
}

fn remote_command(artifact: &RemoteArtifact, credential_envs: &[String]) -> String {
    // SSH commands run under a non-login shell, so they do not inherit the
    // user's normal profile PATH. Give the worker (and therefore its shell
    // tools) the same conventional user-local binary locations explicitly.
    let path = format!(
        "{}/.local/bin:{}/bin:/usr/local/bin:/usr/bin:/bin",
        artifact.home, artifact.home
    );
    let worker = format!(
        "{} --config {}",
        shq(&artifact.binary_path),
        shq(&artifact.config_path)
    );
    if credential_envs.is_empty() {
        return format!("PATH={}; export PATH; exec {worker}", shq(&path));
    }
    let reads = credential_envs
        .iter()
        .map(|env| format!("IFS= read -r {env}"))
        .collect::<Vec<_>>()
        .join(" && ");
    format!(
        "{reads}; export {envs}; PATH={path}; export PATH; exec {worker}",
        path = shq(&path),
        envs = credential_envs.join(" ")
    )
}

fn portable_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

async fn cleanup_failed(
    worker: RemoteWorker,
    error: RemoteWorkerError,
) -> Result<RemoteWorker, RemoteWorkerError> {
    if let Some(reader) = worker.reader.lock().await.take() {
        reader.abort();
    }
    {
        let mut child = worker.child.lock().await;
        let _ = child.start_kill();
        let _ = tokio::time::timeout(CHILD_EXIT_TIMEOUT, child.wait()).await;
    }
    if let Some(stderr_reader) = worker.stderr_reader.lock().await.take() {
        let _ = stderr_reader.await;
    }
    let diagnostics = worker.stderr().await.trim().to_string();
    let _ = worker.transport.shutdown().await;
    drop(worker);
    if diagnostics.is_empty() {
        Err(error)
    } else {
        Err(RemoteWorkerError::Startup(format!(
            "{error}; stderr: {diagnostics}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::RemoteArch;

    #[test]
    fn credential_bootstrap_is_shell_bounded() {
        let artifact = RemoteArtifact {
            arch: RemoteArch::LinuxX86_64,
            home: "/home/ubuntu".into(),
            binary_path: "/home/ubuntu/.clark/bin/worker".into(),
            binary_sha256: "a".repeat(64),
            config_path: "/home/ubuntu/.clark/run/config.json".into(),
        };
        let command = remote_command(
            &artifact,
            &["OPENROUTER_API_KEY".into(), "CLARK_API_KEY".into()],
        );
        assert_eq!(
            command,
            "IFS= read -r OPENROUTER_API_KEY && IFS= read -r CLARK_API_KEY; export OPENROUTER_API_KEY CLARK_API_KEY; PATH='/home/ubuntu/.local/bin:/home/ubuntu/bin:/usr/local/bin:/usr/bin:/bin'; export PATH; exec '/home/ubuntu/.clark/bin/worker' --config '/home/ubuntu/.clark/run/config.json'"
        );
        assert!(!command.contains("$"));
        assert!(!command.contains('\n'));
    }

    #[test]
    fn worker_path_includes_user_local_binaries_without_profile_loading() {
        let artifact = RemoteArtifact {
            arch: RemoteArch::LinuxX86_64,
            home: "/srv/remote user".into(),
            binary_path: "/srv/remote user/.clark/bin/worker".into(),
            binary_sha256: "a".repeat(64),
            config_path: "/srv/remote user/.clark/run/config.json".into(),
        };

        let command = remote_command(&artifact, &[]);

        assert!(command.contains(
            "PATH='/srv/remote user/.local/bin:/srv/remote user/bin:/usr/local/bin:/usr/bin:/bin'"
        ));
        assert!(!command.contains('$'));
        assert!(!command.contains('\n'));
    }

    #[test]
    fn request_ids_are_portable_and_bounded() {
        assert!(portable_request_id("prompt-1:part"));
        assert!(!portable_request_id("../escape"));
        assert!(!portable_request_id(&"x".repeat(129)));
    }

    #[tokio::test]
    async fn progress_delivery_is_ordered_and_terminal_is_exactly_once() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (frames, mut receiver) = mpsc::channel(4);
        pending.lock().await.insert(
            "request-1".into(),
            PendingRequest {
                frames,
                next_sequence: 0,
            },
        );

        deliver_response(
            &pending,
            "request-1".into(),
            Response::progress("request-1", 0, "agent_event", serde_json::json!({"n": 1})),
        )
        .await;
        deliver_response(
            &pending,
            "request-1".into(),
            Response::result(Some("request-1".into()), "done", serde_json::Value::Null),
        )
        .await;

        assert!(matches!(
            receiver.recv().await.unwrap().unwrap(),
            RemoteWorkerFrame::Progress(RemoteWorkerProgress { sequence: 0, .. })
        ));
        assert!(matches!(
            receiver.recv().await.unwrap().unwrap(),
            RemoteWorkerFrame::Terminal(Response::Result { .. })
        ));
        assert!(receiver.recv().await.is_none());
        assert!(pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn completed_request_drop_cannot_remove_an_immediate_retry() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (frames, receiver) = mpsc::channel(1);
        pending.lock().await.insert(
            "request-1".into(),
            PendingRequest {
                frames,
                next_sequence: 0,
            },
        );
        let mut request = RemoteWorkerRequest {
            request_id: "request-1".into(),
            frames: receiver,
            pending: pending.clone(),
            finished: false,
        };
        deliver_response(
            &pending,
            "request-1".into(),
            Response::result(Some("request-1".into()), "done", serde_json::Value::Null),
        )
        .await;
        assert!(matches!(
            request.next().await.unwrap(),
            RemoteWorkerFrame::Terminal(Response::Result { .. })
        ));

        let (retry_frames, _retry_receiver) = mpsc::channel(1);
        pending.lock().await.insert(
            "request-1".into(),
            PendingRequest {
                frames: retry_frames,
                next_sequence: 0,
            },
        );
        drop(request);
        tokio::task::yield_now().await;
        assert!(pending.lock().await.contains_key("request-1"));
    }

    #[tokio::test]
    async fn ambiguous_progress_sequence_fails_only_its_request() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (frames, mut receiver) = mpsc::channel(4);
        pending.lock().await.insert(
            "request-1".into(),
            PendingRequest {
                frames,
                next_sequence: 0,
            },
        );

        deliver_response(
            &pending,
            "request-1".into(),
            Response::progress("request-1", 1, "agent_event", serde_json::Value::Null),
        )
        .await;

        let error = receiver.recv().await.unwrap().unwrap_err();
        assert!(
            matches!(error, RemoteWorkerError::Protocol(message) if message.contains("sequence mismatch"))
        );
        assert!(pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn abandoned_progress_consumer_is_removed_without_leaking_pending_state() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (frames, receiver) = mpsc::channel(1);
        pending.lock().await.insert(
            "request-1".into(),
            PendingRequest {
                frames,
                next_sequence: 0,
            },
        );
        drop(receiver);

        deliver_response(
            &pending,
            "request-1".into(),
            Response::progress("request-1", 0, "agent_event", serde_json::Value::Null),
        )
        .await;

        assert!(pending.lock().await.is_empty());
    }
}
