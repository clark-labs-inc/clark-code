//! `agent_core::Provider` adapter for a complete coding-agent worker owned by the
//! native runtime. The worker owns the model loop, tools, checkout, policy, and
//! trajectory; this adapter only translates its ordered control stream.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_core::provider::{EventStream, SessionEnvironment};
use agent_core::{
    AgentEvent, ClientResponse, Error, PromptInput, Provider, ProviderCapabilities, ProviderConfig,
    ProviderId, RunFailureKind, RunId, RunOutcome, RunStatus, Session, SessionId, SessionOptions,
};
use async_trait::async_trait;
use code_host::{Request, RequestCommand, Response, PROTOCOL_VERSION};
use code_remote::{RemoteWorkerFrame, RemoteWorkerSlot};
use futures::stream::{self, BoxStream};
use futures::{stream::FuturesUnordered, StreamExt};
use serde_json::{json, Value};
use tokio::sync::Mutex;

const PROVIDER_ID: &str = "local";

type WorkerFrames = BoxStream<'static, Result<RemoteWorkerFrame, String>>;

#[async_trait]
trait WorkerClient: Send + Sync {
    async fn request(&self, request: Request) -> Result<Response, String>;
    async fn start(&self, request: Request) -> Result<WorkerFrames, String>;
}

#[async_trait]
impl WorkerClient for RemoteWorkerSlot {
    async fn request(&self, request: Request) -> Result<Response, String> {
        RemoteWorkerSlot::request(self, request)
            .await
            .map_err(|error| error.to_string())
    }

    async fn start(&self, request: Request) -> Result<WorkerFrames, String> {
        let request = self
            .start_request(request)
            .await
            .map_err(|error| error.to_string())?;
        Ok(stream::unfold(request, |mut request| async move {
            let frame = request.next().await.map_err(|error| error.to_string());
            Some((frame, request))
        })
        .boxed())
    }
}

/// One provider attachment to a shared, native-owned worker. A desktop host
/// host creates one adapter per conversation while the worker safely
/// multiplexes the underlying sessions by their private worker ids.
pub struct RemoteWorkerProvider {
    worker: Arc<dyn WorkerClient>,
    project_id: String,
    project_root: PathBuf,
    worker_session: Option<String>,
    capabilities: ProviderCapabilities,
    active: Arc<Mutex<HashMap<RunId, String>>>,
    connected: bool,
}

impl RemoteWorkerProvider {
    pub fn new(worker: Arc<RemoteWorkerSlot>, project_id: String, project_root: PathBuf) -> Self {
        Self::with_client(worker, project_id, project_root)
    }

    fn with_client(
        worker: Arc<dyn WorkerClient>,
        project_id: String,
        project_root: PathBuf,
    ) -> Self {
        Self {
            worker,
            project_id,
            project_root,
            worker_session: None,
            capabilities: ProviderCapabilities::default(),
            active: Arc::new(Mutex::new(HashMap::new())),
            connected: false,
        }
    }

    fn request(&self, operation: &str, input: Value) -> Request {
        Request {
            schema_version: PROTOCOL_VERSION,
            request_id: format!("{operation}-{}", uuid::Uuid::new_v4().simple()),
            command: RequestCommand::Invoke {
                plugin: "coding".into(),
                operation: operation.into(),
                project_id: Some(self.project_id.clone()),
                input,
            },
        }
    }

    async fn update_read_roots(
        &self,
        operation: &str,
        roots: Vec<String>,
    ) -> agent_core::Result<()> {
        if !self.active.lock().await.is_empty() {
            return Err(Error::Unsupported(
                "finish the active run before changing repository context".into(),
            ));
        }
        let worker_session = self.worker_session.as_deref().ok_or(Error::NotConnected)?;
        let response = self
            .worker
            .request(self.request(
                operation,
                json!({"session_id": worker_session, "roots": roots}),
            ))
            .await
            .map_err(Error::Transport)?;
        terminal_data(response, "plugin_result")?;
        Ok(())
    }
}

#[async_trait]
impl Provider for RemoteWorkerProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    async fn connect(&mut self, config: ProviderConfig) -> agent_core::Result<()> {
        if config.endpoint.is_some()
            || config.command.is_some()
            || config.cwd.is_some()
            || config.auth_token.is_some()
            || !config.headers.is_empty()
            || !config.extra.is_null()
        {
            return Err(Error::Protocol(
                "remote worker provider configuration must be resolved by the native registry"
                    .into(),
            ));
        }
        self.connected = true;
        Ok(())
    }

    async fn new_session(&mut self, mut options: SessionOptions) -> agent_core::Result<Session> {
        if !self.connected {
            return Err(Error::NotConnected);
        }
        if self.worker_session.is_some() {
            return Err(Error::Protocol(
                "remote worker provider is already bound to a session".into(),
            ));
        }
        if options
            .cwd
            .as_deref()
            .is_some_and(|cwd| std::path::Path::new(cwd) != self.project_root)
        {
            return Err(Error::Protocol(
                "remote session root does not match the native worker registration".into(),
            ));
        }
        options.cwd = Some(self.project_root.to_string_lossy().into_owned());
        let request = self.request(
            "session.open",
            json!({
                "session_id": format!("session-{}", uuid::Uuid::new_v4().simple()),
                "options": options,
            }),
        );
        let response = self
            .worker
            .request(request)
            .await
            .map_err(Error::Transport)?;
        let data = terminal_data(response, "plugin_result")?;
        let worker_session = required_string(&data, "session_id")?;
        let capabilities: ProviderCapabilities = serde_json::from_value(
            data.get("capabilities")
                .cloned()
                .ok_or_else(|| Error::Protocol("worker session omitted capabilities".into()))?,
        )?;
        self.worker_session = Some(worker_session.clone());
        self.capabilities = capabilities.clone();
        Ok(Session {
            id: SessionId::new(worker_session),
            provider: self.id(),
            capabilities,
            mode: options.mode,
            collaboration_mode: options.collaboration_mode.unwrap_or_default(),
            environment: Some(SessionEnvironment {
                checkout_root: Some(self.project_root.to_string_lossy().into_owned()),
                workspace_roots: vec![self.project_root.to_string_lossy().into_owned()],
                remote: true,
                ..SessionEnvironment::default()
            }),
        })
    }

    async fn load_session(&mut self, id: SessionId) -> agent_core::Result<Session> {
        Err(Error::Unsupported(format!(
            "remote worker sessions reopen through typed transcript replay, not load_session: {id}"
        )))
    }

    async fn validate_prompt(
        &self,
        _session: &SessionId,
        input: &PromptInput,
    ) -> agent_core::Result<()> {
        if self.worker_session.is_none() {
            return Err(Error::NotConnected);
        }
        if input.blocks.is_empty() && input.attachments.is_empty() {
            return Err(Error::Protocol(
                "remote prompt requires content or an attachment".into(),
            ));
        }
        Ok(())
    }

    async fn prompt(
        &mut self,
        public_session: &SessionId,
        input: PromptInput,
    ) -> agent_core::Result<EventStream> {
        self.validate_prompt(public_session, &input).await?;
        if !self.active.lock().await.is_empty() {
            return Err(Error::Protocol(
                "remote worker provider already has an active run".into(),
            ));
        }
        let worker_session = self.worker_session.clone().ok_or(Error::NotConnected)?;
        let request = self.request(
            "session.prompt",
            json!({"session_id": worker_session, "input": input}),
        );
        let request_id = request.request_id.clone();
        let mut frames = self.worker.start(request).await.map_err(Error::Transport)?;
        let public_run = RunId::new(format!("remote-{}", uuid::Uuid::new_v4().simple()));
        self.active
            .lock()
            .await
            .insert(public_run.clone(), request_id);
        let (tx, rx) = async_channel::bounded(64);
        tx.send(AgentEvent::RunStarted {
            run: public_run.clone(),
        })
        .await
        .map_err(|_| Error::Transport("remote event stream closed".into()))?;
        let active = self.active.clone();
        let public_session = public_session.clone();
        tokio::spawn(async move {
            let mut finished = false;
            while let Some(frame) = frames.next().await {
                match frame {
                    Ok(RemoteWorkerFrame::Progress(progress)) if progress.kind == "agent_event" => {
                        match serde_json::from_value::<AgentEvent>(progress.data) {
                            Ok(AgentEvent::RunStarted { .. }) => {}
                            Ok(mut event) => {
                                remap_event(&mut event, &public_run, &public_session);
                                finished |= matches!(event, AgentEvent::RunFinished { .. });
                                if tx.send(event).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                emit_failure(&tx, &public_run, &error.to_string()).await;
                                break;
                            }
                        }
                    }
                    Ok(RemoteWorkerFrame::Progress(_)) => {}
                    Ok(RemoteWorkerFrame::Terminal(Response::Result { kind, .. }))
                        if kind == "plugin_result" =>
                    {
                        if !finished {
                            emit_failure(
                                &tx,
                                &public_run,
                                "remote worker completed without a terminal run event",
                            )
                            .await;
                        }
                        break;
                    }
                    Ok(RemoteWorkerFrame::Terminal(Response::Error { code, message, .. })) => {
                        if code == "cancelled" {
                            emit_cancelled(&tx, &public_run).await;
                        } else {
                            emit_failure(&tx, &public_run, &format!("worker {code}: {message}"))
                                .await;
                        }
                        break;
                    }
                    Ok(RemoteWorkerFrame::Terminal(_)) => {
                        emit_failure(&tx, &public_run, "unexpected remote terminal response").await;
                        break;
                    }
                    Err(error) => {
                        emit_failure(&tx, &public_run, &error).await;
                        break;
                    }
                }
            }
            active.lock().await.remove(&public_run);
        });
        Ok(rx.boxed())
    }

    async fn cancel(&mut self, _session: &SessionId, run: &RunId) -> agent_core::Result<()> {
        let target_request_id = self
            .active
            .lock()
            .await
            .get(run)
            .cloned()
            .ok_or_else(|| Error::Protocol(format!("remote run is not active: {run}")))?;
        let response = self
            .worker
            .request(Request {
                schema_version: PROTOCOL_VERSION,
                request_id: format!("cancel-{}", uuid::Uuid::new_v4().simple()),
                command: RequestCommand::Cancel { target_request_id },
            })
            .await
            .map_err(Error::Transport)?;
        terminal_data(response, "cancelled")?;
        Ok(())
    }

    async fn add_read_roots(
        &mut self,
        _session: &SessionId,
        roots: Vec<String>,
    ) -> agent_core::Result<()> {
        self.update_read_roots("session.add_read_roots", roots)
            .await
    }

    async fn remove_read_roots(
        &mut self,
        _session: &SessionId,
        roots: Vec<String>,
    ) -> agent_core::Result<()> {
        self.update_read_roots("session.remove_read_roots", roots)
            .await
    }

    async fn close_session(&mut self, _session: &SessionId) -> agent_core::Result<()> {
        let active = self.active.lock().await.clone();
        let mut cancellations = FuturesUnordered::new();
        for target_request_id in active.into_values() {
            let worker = self.worker.clone();
            cancellations.push(async move {
                worker
                    .request(Request {
                        schema_version: PROTOCOL_VERSION,
                        request_id: format!("cancel-{}", uuid::Uuid::new_v4().simple()),
                        command: RequestCommand::Cancel { target_request_id },
                    })
                    .await
            });
        }
        while cancellations.next().await.is_some() {}
        let Some(worker_session) = self.worker_session.take() else {
            return Ok(());
        };
        let response = self
            .worker
            .request(self.request("session.close", json!({"session_id": worker_session})))
            .await
            .map_err(Error::Transport)?;
        terminal_data(response, "plugin_result")?;
        Ok(())
    }

    async fn respond(
        &mut self,
        session: &SessionId,
        response: ClientResponse,
    ) -> agent_core::Result<()> {
        let worker_session = self.worker_session.as_deref().ok_or(Error::NotConnected)?;
        if session.as_str() != worker_session {
            return Err(Error::SessionNotFound(session.to_string()));
        }
        let response = self
            .worker
            .request(self.request(
                "session.respond",
                json!({"session_id": worker_session, "response": response}),
            ))
            .await
            .map_err(Error::Transport)?;
        terminal_data(response, "plugin_result")?;
        Ok(())
    }
}

fn terminal_data(response: Response, expected_kind: &str) -> agent_core::Result<Value> {
    match response {
        Response::Result { kind, data, .. } if kind == expected_kind => Ok(data),
        Response::Error { code, message, .. } => {
            Err(Error::Transport(format!("worker {code}: {message}")))
        }
        Response::Progress { .. } => Err(Error::Protocol(
            "worker request returned progress as a terminal response".into(),
        )),
        Response::Result { kind, .. } => Err(Error::Protocol(format!(
            "worker returned {kind}, expected {expected_kind}"
        ))),
    }
}

fn required_string(value: &Value, key: &str) -> agent_core::Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Protocol(format!("worker response omitted {key}")))
}

fn remap_event(event: &mut AgentEvent, run: &RunId, session: &SessionId) {
    match event {
        AgentEvent::RunStarted { run: value }
        | AgentEvent::Checkpoint { run: value, .. }
        | AgentEvent::MessageChunk { run: value, .. }
        | AgentEvent::MessagePhase { run: value, .. }
        | AgentEvent::SpecialistPresentation { run: value, .. }
        | AgentEvent::ToolCall { run: value, .. }
        | AgentEvent::ToolCallUpdate { run: value, .. }
        | AgentEvent::ExecutionChecklistUpdated { run: value, .. }
        | AgentEvent::ProposedPlanUpdated { run: value, .. }
        | AgentEvent::GoalUpdated { run: value, .. }
        | AgentEvent::RunUsageUpdated { run: value, .. }
        | AgentEvent::Artifact { run: value, .. }
        | AgentEvent::FanOut { run: value, .. }
        | AgentEvent::ProviderIncidentUpdated { run: value, .. }
        | AgentEvent::ContextCompacted { run: value, .. }
        | AgentEvent::RunFinished { run: value, .. } => *value = run.clone(),
        AgentEvent::PermissionRequest { request } => request.session = session.clone(),
        AgentEvent::ModeChanged { session: value, .. } => *value = session.clone(),
        AgentEvent::Trace {
            run: Some(value), ..
        }
        | AgentEvent::Error {
            run: Some(value), ..
        } => *value = run.clone(),
        AgentEvent::Surface { .. }
        | AgentEvent::Trace { run: None, .. }
        | AgentEvent::Error { run: None, .. } => {}
    }
}

async fn emit_failure(tx: &async_channel::Sender<AgentEvent>, run: &RunId, message: &str) {
    let _ = tx
        .send(AgentEvent::Error {
            code: "remote_worker_failed".into(),
            message: message.into(),
            run: Some(run.clone()),
        })
        .await;
    let _ = tx
        .send(AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status: RunStatus::Failed,
                stop_reason: Some("remote_worker_failed".into()),
                error: Some(message.into()),
                failure_kind: Some(RunFailureKind::RuntimeInterrupted),
                usage: None,
                execution: None,
            },
        })
        .await;
}

async fn emit_cancelled(tx: &async_channel::Sender<AgentEvent>, run: &RunId) {
    let _ = tx
        .send(AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status: RunStatus::Cancelled,
                stop_reason: Some("cancelled".into()),
                error: None,
                failure_kind: None,
                usage: None,
                execution: None,
            },
        })
        .await;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use agent_core::{CollaborationMode, ContentBlock, Role};
    use futures::StreamExt;

    use super::*;

    struct FakeWorker {
        frames: StdMutex<Option<Vec<Result<RemoteWorkerFrame, String>>>>,
        requests: StdMutex<Vec<String>>,
    }

    impl FakeWorker {
        fn new(events: Vec<AgentEvent>) -> Self {
            let mut frames = events
                .into_iter()
                .enumerate()
                .map(|(sequence, event)| {
                    Ok(RemoteWorkerFrame::Progress(
                        code_remote::RemoteWorkerProgress {
                            sequence: sequence as u64,
                            kind: "agent_event".into(),
                            data: serde_json::to_value(event).unwrap(),
                        },
                    ))
                })
                .collect::<Vec<_>>();
            frames.push(Ok(RemoteWorkerFrame::Terminal(Response::result(
                Some("prompt".into()),
                "plugin_result",
                json!({"complete": true}),
            ))));
            Self {
                frames: StdMutex::new(Some(frames)),
                requests: StdMutex::new(Vec::new()),
            }
        }

        fn cancelled() -> Self {
            Self {
                frames: StdMutex::new(Some(vec![Ok(RemoteWorkerFrame::Terminal(
                    Response::error(
                        Some("prompt".into()),
                        "cancelled",
                        "plugin invocation cancelled",
                    ),
                ))])),
                requests: StdMutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl WorkerClient for FakeWorker {
        async fn request(&self, request: Request) -> Result<Response, String> {
            match request.command {
                RequestCommand::Invoke {
                    operation, input, ..
                } => {
                    self.requests.lock().unwrap().push(operation.clone());
                    match operation.as_str() {
                        "session.open" => Ok(Response::result(
                            Some(request.request_id),
                            "plugin_result",
                            json!({
                                "session_id": input["session_id"],
                                "capabilities": capabilities()
                            }),
                        )),
                        "session.close" => Ok(Response::result(
                            Some(request.request_id),
                            "plugin_result",
                            json!({"closed": true}),
                        )),
                        "session.respond" => Ok(Response::result(
                            Some(request.request_id),
                            "plugin_result",
                            json!({"accepted": true}),
                        )),
                        "session.add_read_roots" | "session.remove_read_roots" => {
                            Ok(Response::result(
                                Some(request.request_id),
                                "plugin_result",
                                json!({"updated": true}),
                            ))
                        }
                        other => Err(format!("unexpected operation: {other}")),
                    }
                }
                RequestCommand::Cancel { .. } => Ok(Response::result(
                    Some(request.request_id),
                    "cancelled",
                    json!({"active": true}),
                )),
                _ => Err("unexpected request".into()),
            }
        }

        async fn start(&self, _request: Request) -> Result<WorkerFrames, String> {
            let frames = self
                .frames
                .lock()
                .unwrap()
                .take()
                .ok_or("prompt already started")?;
            Ok(stream::iter(frames).boxed())
        }
    }

    fn capabilities() -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            permissions: true,
            fs: true,
            terminal: true,
            load_session: false,
            attachment_kinds: vec![],
            modes: vec!["auto".into()],
            collaboration_modes: vec![CollaborationMode::Default, CollaborationMode::Plan],
        }
    }

    #[tokio::test]
    async fn adapter_streams_one_public_run_and_remaps_remote_identity() {
        let remote_run = RunId::new("private-worker-run");
        let worker = Arc::new(FakeWorker::new(vec![
            AgentEvent::RunStarted {
                run: remote_run.clone(),
            },
            AgentEvent::MessageChunk {
                run: remote_run.clone(),
                role: Role::Agent,
                delta: ContentBlock::text("remote output"),
            },
            AgentEvent::RunFinished {
                run: remote_run,
                outcome: RunOutcome {
                    status: RunStatus::Done,
                    stop_reason: Some("complete".into()),
                    error: None,
                    failure_kind: None,
                    usage: None,
                    execution: None,
                },
            },
        ]));
        let mut provider = RemoteWorkerProvider::with_client(
            worker.clone(),
            "project-1".into(),
            PathBuf::from("/srv/project"),
        );
        provider.connect(ProviderConfig::default()).await.unwrap();
        let session = provider
            .new_session(SessionOptions {
                cwd: Some("/srv/project".into()),
                collaboration_mode: Some(CollaborationMode::Plan),
                ..SessionOptions::default()
            })
            .await
            .unwrap();
        assert!(session.environment.as_ref().unwrap().remote);
        assert!(session.capabilities.permissions);
        provider
            .respond(
                &session.id,
                ClientResponse::Permission {
                    request: agent_core::PermissionRequestId::new("permission-1"),
                    option: "allow-once".into(),
                    feedback: None,
                },
            )
            .await
            .unwrap();
        let events = provider
            .prompt(&session.id, PromptInput::text("inspect"))
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        let public_run = match &events[0] {
            AgentEvent::RunStarted { run } => run.clone(),
            other => panic!("unexpected first event: {other:?}"),
        };
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentEvent::RunStarted { .. }))
                .count(),
            1
        );
        assert!(matches!(
            &events[1],
            AgentEvent::MessageChunk { run, .. } if run == &public_run
        ));
        assert!(matches!(
            &events[2],
            AgentEvent::RunFinished { run, .. } if run == &public_run
        ));
        assert!(worker
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|operation| operation == "session.respond"));
    }

    #[tokio::test]
    async fn adapter_rejects_renderer_owned_configuration() {
        let worker = Arc::new(FakeWorker::new(Vec::new()));
        let mut provider = RemoteWorkerProvider::with_client(
            worker,
            "project-1".into(),
            PathBuf::from("/srv/project"),
        );
        let error = provider
            .connect(ProviderConfig {
                auth_token: Some("renderer-secret".into()),
                ..ProviderConfig::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Protocol(_)));
    }

    #[tokio::test]
    async fn adapter_projects_a_worker_cancellation_as_cancelled_not_failed() {
        let worker = Arc::new(FakeWorker::cancelled());
        let mut provider = RemoteWorkerProvider::with_client(
            worker,
            "project-1".into(),
            PathBuf::from("/srv/project"),
        );
        provider.connect(ProviderConfig::default()).await.unwrap();
        let session = provider
            .new_session(SessionOptions {
                cwd: Some("/srv/project".into()),
                ..SessionOptions::default()
            })
            .await
            .unwrap();

        let events = provider
            .prompt(&session.id, PromptInput::text("inspect"))
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        assert!(matches!(
            events.last(),
            Some(AgentEvent::RunFinished { outcome, .. })
                if outcome.status == RunStatus::Cancelled
                    && outcome.failure_kind.is_none()
                    && outcome.error.is_none()
        ));
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::Error { .. })));
    }

    #[tokio::test]
    async fn adapter_updates_read_roots_through_the_remote_worker() {
        let worker = Arc::new(FakeWorker::new(Vec::new()));
        let mut provider = RemoteWorkerProvider::with_client(
            worker.clone(),
            "project-1".into(),
            PathBuf::from("/srv/project"),
        );
        provider.connect(ProviderConfig::default()).await.unwrap();
        let session = provider
            .new_session(SessionOptions {
                cwd: Some("/srv/project".into()),
                ..SessionOptions::default()
            })
            .await
            .unwrap();

        provider
            .add_read_roots(&session.id, vec!["/srv/shared/api".into()])
            .await
            .unwrap();
        provider
            .remove_read_roots(&session.id, vec!["/srv/shared/api".into()])
            .await
            .unwrap();

        let requests = worker.requests.lock().unwrap();
        assert!(requests
            .iter()
            .any(|operation| operation == "session.add_read_roots"));
        assert!(requests
            .iter()
            .any(|operation| operation == "session.remove_read_roots"));
    }
}
