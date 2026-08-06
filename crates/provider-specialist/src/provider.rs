use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_core::provider::{EventStream, SessionEnvironment};
use agent_core::{
    classify_provider_access_failure, AgentEvent, ClientResponse, CollaborationMode, ContentBlock,
    Error, MessagePhase, PromptInput, Provider, ProviderCapabilities, ProviderConfig, ProviderId,
    Role, RunFailureKind, RunId, RunOutcome, RunStatus, RunUsage, Session, SessionId,
    SessionOptions,
};
use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::Digest;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::{ConnectedConfig, SpecialistConnectConfig};
use crate::protocol::{WorkerCommand, WorkerRequest};
use crate::transport::{self, WorkerFailure};
use code_host::{
    Request as RemoteRequest, RequestCommand as RemoteCommand, Response as RemoteResponse,
};
use code_remote::{RemoteWorker, RemoteWorkerSpec};

const PROVIDER_ID: &str = "specialist";

pub struct SpecialistProvider {
    config: Option<ConnectedConfig>,
    session_id: Option<SessionId>,
    worker_config_path: Option<std::path::PathBuf>,
    remote_worker: Option<Arc<RemoteWorker>>,
    worker_version: Option<String>,
    catalog_sha256: Option<String>,
    active: Arc<Mutex<HashMap<RunId, CancellationToken>>>,
    run_counter: AtomicU64,
    cloud_api_base_url: String,
}

impl SpecialistProvider {
    pub fn new() -> Self {
        Self {
            config: None,
            session_id: None,
            worker_config_path: None,
            remote_worker: None,
            worker_version: None,
            catalog_sha256: None,
            active: Arc::new(Mutex::new(HashMap::new())),
            run_counter: AtomicU64::new(0),
            cloud_api_base_url: "https://api.clarkslabs.com/v1".into(),
        }
    }

    /// Constructs a provider whose worker uses a loopback cloud fixture.
    /// This API is absent from production builds.
    #[cfg(feature = "test-utils")]
    pub fn new_with_test_cloud_api_base_url(
        base_url: impl Into<String>,
    ) -> agent_core::Result<Self> {
        let base_url = base_url.into();
        let endpoint = base_url.trim_end_matches('/');
        if !(endpoint.starts_with("http://127.0.0.1:") || endpoint.starts_with("http://localhost:"))
            || !endpoint.ends_with("/v1")
        {
            return Err(Error::Protocol(
                "specialist test cloud endpoint must be a loopback HTTP /v1 endpoint".into(),
            ));
        }
        let mut provider = Self::new();
        provider.cloud_api_base_url = endpoint.into();
        Ok(provider)
    }

    async fn create_session(
        &mut self,
        id: SessionId,
        options: SessionOptions,
    ) -> agent_core::Result<Session> {
        let config = self.config.as_ref().ok_or(Error::NotConnected)?;
        if let Some(cwd) = options.cwd.as_deref() {
            let requested = if config.remote.is_some() {
                std::path::PathBuf::from(cwd)
            } else {
                std::path::Path::new(cwd).canonicalize().map_err(|error| {
                    Error::Io(format!(
                        "specialist session cwd could not be resolved: {error}"
                    ))
                })?
            };
            if requested != config.cwd {
                return Err(Error::Protocol(
                    "specialist session cwd changed after native registration".into(),
                ));
            }
        }
        let safe_session_id = portable_session_id(id.as_str());
        let session_root = config.runtime_root.join("sessions").join(&safe_session_id);
        tokio::fs::create_dir_all(&session_root).await?;
        let extra = SpecialistConnectConfig {
            specialist: config.specialist,
            workflow: config.workflow.clone(),
            organization_id: config.organization_id.clone(),
            workspace_id: config.workspace_id.clone(),
            scout_context: config.scout_context.clone(),
            runtime_root: config.runtime_root.clone(),
            worker_sha256: config.worker_sha256.clone(),
            model_route: config.model_route,
            max_iterations: config.max_iterations,
            advisor_training_enabled: config.advisor_training_enabled,
            remote: config.remote.clone(),
            remote_worker_binaries: config.remote_worker_binaries.clone(),
        };
        let remote_trajectory = config.remote.as_ref().map(|target| {
            target
                .remote_root
                .join(".clark/specialist-trajectory")
                .join(&safe_session_id)
        });
        let execution_residency = if config.remote.is_some() {
            "remote_worker"
        } else {
            "local_only"
        };
        let trajectory_root = remote_trajectory
            .as_deref()
            .unwrap_or(session_root.as_path());
        let worker_config = extra.worker_config(
            &safe_session_id,
            &config.cwd,
            &config.project_id,
            &self.cloud_api_base_url,
            execution_residency,
            trajectory_root,
        );
        // Every provider attachment receives an immutable config file. This
        // avoids an in-place replacement race when the same durable session is
        // reopened while an older bounded worker is still exiting.
        let worker_config_path =
            session_root.join(format!("worker-{}.json", Uuid::new_v4().simple()));
        let request_id = format!("ping-{}", Uuid::new_v4().simple());
        let request = WorkerRequest {
            schema_version: 1,
            request_id,
            command: WorkerCommand::Ping,
        };
        let ping_cancel = CancellationToken::new();
        let (pong, remote_worker) = if let Some(remote) = &config.remote {
            let model_key = config.model_key.clone().ok_or_else(|| {
                Error::Protocol("specialist remote worker requires a Clark model key".into())
            })?;
            let spec = RemoteWorkerSpec {
                host: remote.host.clone(),
                project_id: config.project_id.clone(),
                remote_root: remote.remote_root.clone(),
                trajectory_root: trajectory_root.to_path_buf(),
                worker_config,
                local_binary: None,
                local_binaries: config.remote_worker_binaries.clone(),
                remote_binary: None,
                credential_envs: vec![config.child_key_env().into()],
            };
            let credentials = HashMap::from([(config.child_key_env().to_string(), model_key)]);
            let worker = Arc::new(
                RemoteWorker::connect_with_credentials(spec, credentials)
                    .await
                    .map_err(|error| Error::Transport(error.to_string()))?,
            );
            let info = worker.info();
            let pong = serde_json::json!({
                "worker": info.worker,
                "protocol_version": 1,
                "worker_version": info.worker_version,
                "advisor_version": info.advisor_version,
                "quiet_by_default": true,
            });
            (pong, Some(worker))
        } else {
            write_private_json(&worker_config_path, &worker_config).await?;
            let pong =
                transport::execute(config, &worker_config_path, &request, "pong", &ping_cancel)
                    .await
                    .map_err(worker_error)?;
            (pong, None)
        };
        if pong.get("worker").and_then(Value::as_str) != Some("clark-code-headless")
            || pong.get("protocol_version").and_then(Value::as_u64) != Some(1)
            || pong
                .get("worker_version")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || pong.get("advisor_version").and_then(Value::as_str) != Some("cloud-advisor.v1")
            || pong.get("quiet_by_default").and_then(Value::as_bool) != Some(true)
        {
            return Err(Error::Protocol(
                "specialist worker handshake did not prove a quiet headless runtime".into(),
            ));
        }
        let worker_version = pong["worker_version"]
            .as_str()
            .expect("validated worker version")
            .to_string();
        let catalog_request = WorkerRequest {
            schema_version: 1,
            request_id: format!("catalog-{}", Uuid::new_v4().simple()),
            command: WorkerCommand::SpecialistCatalog,
        };
        let catalog = if let Some(worker) = &remote_worker {
            remote_catalog(worker, &catalog_request.request_id, &ping_cancel)
                .await
                .map_err(worker_error)?
        } else {
            transport::execute(
                config,
                &worker_config_path,
                &catalog_request,
                "specialist_catalog",
                &ping_cancel,
            )
            .await
            .map_err(worker_error)?
        };
        let catalog_sha256 = validate_catalog(
            &catalog,
            &worker_version,
            config.specialist.as_str(),
            &config.workflow,
        )?;
        self.session_id = Some(id.clone());
        self.worker_config_path = remote_worker.is_none().then_some(worker_config_path);
        self.remote_worker = remote_worker;
        self.worker_version = Some(worker_version);
        self.catalog_sha256 = Some(catalog_sha256);
        Ok(Session {
            id,
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: options.mode,
            collaboration_mode: options.collaboration_mode.unwrap_or_default(),
            environment: Some(SessionEnvironment {
                checkout_root: Some(config.cwd.to_string_lossy().into_owned()),
                repository_root: None,
                workspace_roots: vec![config.cwd.to_string_lossy().into_owned()],
                docs_root: Some(session_root.to_string_lossy().into_owned()),
                remote: config.remote.is_some(),
            }),
        })
    }
}

impl Default for SpecialistProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for SpecialistProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            permissions: false,
            fs: false,
            terminal: false,
            load_session: true,
            attachment_kinds: Vec::new(),
            modes: Vec::new(),
            collaboration_modes: vec![CollaborationMode::Default],
        }
    }

    async fn connect(&mut self, config: ProviderConfig) -> agent_core::Result<()> {
        let connected = ConnectedConfig::parse(config)?;
        tokio::fs::create_dir_all(&connected.runtime_root).await?;
        self.config = Some(connected);
        Ok(())
    }

    async fn new_session(&mut self, options: SessionOptions) -> agent_core::Result<Session> {
        self.create_session(
            SessionId::new(format!("specialist-{}", Uuid::new_v4().simple())),
            options,
        )
        .await
    }

    async fn load_session(&mut self, id: SessionId) -> agent_core::Result<Session> {
        self.create_session(id, SessionOptions::default()).await
    }

    async fn prompt(
        &mut self,
        session: &SessionId,
        input: PromptInput,
    ) -> agent_core::Result<EventStream> {
        if self.session_id.as_ref() != Some(session) {
            return Err(Error::SessionNotFound(session.to_string()));
        }
        if !input.attachments.is_empty() {
            return Err(Error::Unsupported(
                "Scientist and RSI turns do not accept raw attachments; register evidence or a source snapshot first".into(),
            ));
        }
        let message = text_prompt(&input.blocks)?;
        let config = self.config.clone().ok_or(Error::NotConnected)?;
        let worker_config_path = self.worker_config_path.clone();
        let remote_worker = self.remote_worker.clone();
        if worker_config_path.is_none() && remote_worker.is_none() {
            return Err(Error::NotConnected);
        }
        let sequence = self.run_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let run = RunId::new(format!("specialist-run-{sequence}"));
        let request_id = format!("turn-{}-{}", sequence, Uuid::new_v4().simple());
        let request = WorkerRequest {
            schema_version: 1,
            request_id,
            command: WorkerCommand::SpecialistTurn {
                session_id: portable_session_id(session.as_str()),
                specialist: config.specialist.as_str().into(),
                workflow: config.workflow.clone(),
                project_id: config.project_id.clone(),
                scout_context: config.scout_context.clone(),
                message,
                now_ms: now_millis()?,
            },
        };
        let cancel = CancellationToken::new();
        self.active.lock().await.insert(run.clone(), cancel.clone());
        let active = self.active.clone();
        let run_for_task = run.clone();
        let organization_id = config.organization_id.clone();
        let worker_sha256 = remote_worker
            .as_ref()
            .map(|worker| worker.info().binary_sha256.clone())
            .unwrap_or_else(|| config.worker_sha256.clone());
        let worker_version = self.worker_version.clone();
        let catalog_sha256 = self.catalog_sha256.clone();
        let (tx, rx) = async_channel::unbounded();
        tokio::spawn(async move {
            let _ = tx
                .send(AgentEvent::RunStarted {
                    run: run_for_task.clone(),
                })
                .await;
            let result = if let Some(worker) = remote_worker {
                remote_turn(&worker, &request, &cancel).await
            } else {
                transport::execute(
                    &config,
                    worker_config_path.as_ref().expect("local path was checked"),
                    &request,
                    "specialist_turn",
                    &cancel,
                )
                .await
            };
            match result {
                Ok(data) => {
                    emit_success(
                        &tx,
                        &run_for_task,
                        data,
                        organization_id,
                        worker_sha256,
                        worker_version,
                        catalog_sha256,
                    )
                    .await
                }
                Err(WorkerFailure::Cancelled) => {
                    emit_finished(&tx, &run_for_task, RunStatus::Cancelled, None, None).await;
                }
                Err(WorkerFailure::TimedOut) => {
                    emit_failure(
                        &tx,
                        &run_for_task,
                        "specialist_timeout",
                        "The specialist worker exceeded its bounded turn deadline.",
                        RunFailureKind::RuntimeInterrupted,
                    )
                    .await;
                }
                Err(WorkerFailure::Failed(error)) => {
                    emit_failure(
                        &tx,
                        &run_for_task,
                        "specialist_worker_failed",
                        &error,
                        classify_failure(&error),
                    )
                    .await;
                }
            }
            active.lock().await.remove(&run_for_task);
        });
        Ok(rx.boxed())
    }

    async fn cancel(&mut self, session: &SessionId, run: &RunId) -> agent_core::Result<()> {
        if self.session_id.as_ref() != Some(session) {
            return Err(Error::SessionNotFound(session.to_string()));
        }
        let token = self
            .active
            .lock()
            .await
            .get(run)
            .cloned()
            .ok_or_else(|| Error::Other(format!("no active specialist run {run}")))?;
        token.cancel();
        Ok(())
    }

    async fn close_session(&mut self, session: &SessionId) -> agent_core::Result<()> {
        if self.session_id.as_ref() != Some(session) {
            return Ok(());
        }
        for token in self.active.lock().await.values() {
            token.cancel();
        }
        self.session_id = None;
        self.worker_config_path = None;
        if let Some(worker) = self.remote_worker.take() {
            worker
                .disconnect()
                .await
                .map_err(|error| Error::Transport(error.to_string()))?;
        }
        self.worker_version = None;
        self.catalog_sha256 = None;
        Ok(())
    }

    async fn respond(
        &mut self,
        _session: &SessionId,
        _response: ClientResponse,
    ) -> agent_core::Result<()> {
        Err(Error::Unsupported(
            "specialist workers expose no interactive provider permission requests".into(),
        ))
    }
}

async fn remote_catalog(
    worker: &RemoteWorker,
    request_id: &str,
    cancel: &CancellationToken,
) -> Result<Value, WorkerFailure> {
    let data = remote_request(
        worker,
        RemoteRequest {
            schema_version: code_host::PROTOCOL_VERSION,
            request_id: request_id.to_string(),
            command: RemoteCommand::Catalog,
        },
        "catalog",
        cancel,
    )
    .await?;
    data.get("specialist_catalog")
        .cloned()
        .ok_or_else(|| WorkerFailure::Failed("remote catalog omitted specialist_catalog".into()))
}

async fn remote_turn(
    worker: &RemoteWorker,
    request: &WorkerRequest,
    cancel: &CancellationToken,
) -> Result<Value, WorkerFailure> {
    let WorkerCommand::SpecialistTurn {
        session_id,
        specialist,
        workflow,
        project_id,
        scout_context,
        message,
        now_ms,
    } = &request.command
    else {
        return Err(WorkerFailure::Failed(
            "remote specialist turn received the wrong command".into(),
        ));
    };
    remote_request(
        worker,
        RemoteRequest {
            schema_version: code_host::PROTOCOL_VERSION,
            request_id: request.request_id.clone(),
            command: RemoteCommand::Invoke {
                plugin: "scientist".into(),
                operation: "turn".into(),
                project_id: Some(project_id.clone()),
                input: serde_json::json!({
                    "session_id": session_id,
                    "specialist": specialist,
                    "workflow": workflow,
                    "project_id": project_id,
                    "scout_context": scout_context,
                    "message": message,
                    "now_ms": now_ms,
                }),
            },
        },
        "invoke_result",
        cancel,
    )
    .await
}

async fn remote_request(
    worker: &RemoteWorker,
    request: RemoteRequest,
    expected_kind: &str,
    cancel: &CancellationToken,
) -> Result<Value, WorkerFailure> {
    let request_id = request.request_id.clone();
    let response = tokio::select! {
        result = worker.request(request) => {
            result.map_err(|error| WorkerFailure::Failed(error.to_string()))?
        }
        _ = cancel.cancelled() => {
            let _ = worker.request(RemoteRequest {
                schema_version: code_host::PROTOCOL_VERSION,
                request_id: format!("cancel-{}", Uuid::new_v4().simple()),
                command: RemoteCommand::Cancel { target_request_id: request_id },
            }).await;
            return Err(WorkerFailure::Cancelled);
        }
    };
    match response {
        RemoteResponse::Result {
            schema_version: code_host::PROTOCOL_VERSION,
            request_id: Some(response_id),
            kind,
            data,
        } if response_id == request_id && kind == expected_kind => Ok(data),
        RemoteResponse::Error {
            schema_version: code_host::PROTOCOL_VERSION,
            request_id: Some(response_id),
            code,
            message,
        } if response_id == request_id => {
            Err(WorkerFailure::Failed(format!("worker {code}: {message}")))
        }
        _ => Err(WorkerFailure::Failed(
            "remote specialist response did not match its request contract".into(),
        )),
    }
}

async fn emit_success(
    tx: &async_channel::Sender<AgentEvent>,
    run: &RunId,
    data: Value,
    organization_id: Option<String>,
    worker_sha256: String,
    worker_version: Option<String>,
    catalog_sha256: Option<String>,
) {
    if let Err(error) = validate_cloud_sync_receipt(&data) {
        emit_failure(
            tx,
            run,
            "specialist_cloud_sync_receipt_missing",
            &format!(
                "Clark refused to finish the specialist turn because cloud synchronization was not verified: {error}"
            ),
            RunFailureKind::VerificationIncomplete,
        )
        .await;
        return;
    }
    let message = data
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("The specialist completed the bounded turn.")
        .to_string();
    let _ = tx
        .send(AgentEvent::MessageChunk {
            run: run.clone(),
            role: Role::Agent,
            delta: ContentBlock::text(message),
        })
        .await;
    let _ = tx
        .send(AgentEvent::MessagePhase {
            run: run.clone(),
            phase: MessagePhase::FinalAnswer,
        })
        .await;
    if let Some(value) = data.get("presentation") {
        if let Ok(presentation) =
            serde_json::from_value::<agent_core::SpecialistPresentation>(value.clone())
        {
            let _ = tx
                .send(AgentEvent::SpecialistPresentation {
                    run: run.clone(),
                    presentation,
                })
                .await;
        }
    }
    if let Some(usage) = advisor_run_usage(&data) {
        let _ = tx
            .send(AgentEvent::RunUsageUpdated {
                run: run.clone(),
                usage,
            })
            .await;
    }
    let _ = tx
        .send(AgentEvent::Trace {
            run: Some(run.clone()),
            source: "clark_specialist_projection".into(),
            payload: product_projection(
                &data,
                organization_id,
                worker_sha256,
                worker_version,
                catalog_sha256,
            ),
        })
        .await;
    emit_finished(tx, run, RunStatus::Done, None, None).await;
}

fn advisor_run_usage(data: &Value) -> Option<RunUsage> {
    let usage = data.pointer("/cloudAdvisor/usage")?;
    let input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cost_usd = usage.get("cost").and_then(Value::as_f64);
    if input_tokens == 0 && output_tokens == 0 && cost_usd.is_none() {
        return None;
    }
    Some(RunUsage {
        input_tokens,
        output_tokens,
        context_tokens: input_tokens,
        cost_usd,
        context_limit: None,
    })
}

fn product_projection(
    data: &Value,
    organization_id: Option<String>,
    worker_sha256: String,
    worker_version: Option<String>,
    catalog_sha256: Option<String>,
) -> Value {
    let mut payload = Map::new();
    payload.insert("schemaVersion".into(), Value::from(1));
    for key in [
        "specialist",
        "workflow",
        "sessionId",
        "programId",
        "researchProjection",
        "rsiProjection",
        "cloudSync",
    ] {
        if let Some(value) = data.get(key) {
            payload.insert(key.into(), value.clone());
        }
    }
    if let Some(organization_id) = organization_id {
        payload.insert("organizationId".into(), Value::String(organization_id));
    }
    if let (Some(worker_version), Some(catalog_sha256)) = (worker_version, catalog_sha256) {
        payload.insert(
            "runtime".into(),
            serde_json::json!({
                "workerVersion": worker_version,
                "workerSha256": worker_sha256,
                "catalogSha256": catalog_sha256,
            }),
        );
    }
    Value::Object(payload)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpecialistCloudSyncReceipt {
    scope_id: String,
    file_count: usize,
    verified_segment_count: usize,
    total_bytes: u64,
}

fn validate_cloud_sync_receipt(data: &Value) -> Result<(), String> {
    let value = data
        .get("cloudSync")
        .ok_or("worker success omitted cloudSync")?;
    let receipt: SpecialistCloudSyncReceipt = serde_json::from_value(value.clone())
        .map_err(|error| format!("worker cloudSync receipt is invalid: {error}"))?;
    if receipt.scope_id.is_empty()
        || receipt.scope_id.len() > 128
        || !receipt.scope_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err("worker cloudSync scope identity is invalid".into());
    }
    if receipt.file_count > 0 && receipt.verified_segment_count == 0 {
        return Err("worker cloudSync verified no segments for a non-empty artifact set".into());
    }
    let _ = receipt.total_bytes;
    Ok(())
}

fn validate_catalog(
    catalog: &Value,
    expected_worker_version: &str,
    specialist: &str,
    workflow: &str,
) -> agent_core::Result<String> {
    let root = catalog
        .as_object()
        .ok_or_else(|| Error::Protocol("specialist catalog is not an object".into()))?;
    if root.get("schema_version").and_then(Value::as_u64) != Some(1)
        || root.get("catalog_version").and_then(Value::as_str) != Some("1.0.0")
        || root.get("worker_version").and_then(Value::as_str) != Some(expected_worker_version)
        || root
            .get("trust")
            .and_then(|trust| trust.get("requires_signed_release_binary"))
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(Error::Protocol(
            "specialist catalog version or trust contract is invalid".into(),
        ));
    }
    let expected_digest = root
        .get("catalog_sha256")
        .and_then(Value::as_str)
        .filter(|value| value.len() == 64)
        .ok_or_else(|| Error::Protocol("specialist catalog has no digest".into()))?;
    let mut unsigned = catalog.clone();
    unsigned
        .as_object_mut()
        .expect("catalog object checked")
        .remove("catalog_sha256");
    let canonical = serde_json::to_vec(&unsigned)?;
    let actual_digest = format!("{:x}", sha2::Sha256::digest(canonical));
    if actual_digest != expected_digest {
        return Err(Error::Protocol(
            "specialist catalog digest did not match its manifests".into(),
        ));
    }
    let manifest = root
        .get("specialists")
        .and_then(Value::as_array)
        .and_then(|specialists| {
            specialists.iter().find(|manifest| {
                manifest.get("specialist_id").and_then(Value::as_str) == Some(specialist)
            })
        })
        .ok_or_else(|| Error::Protocol("specialist is absent from the worker catalog".into()))?;
    let workflow_exists = manifest
        .get("workflows")
        .and_then(Value::as_array)
        .is_some_and(|workflows| {
            workflows
                .iter()
                .any(|entry| entry.get("id").and_then(Value::as_str) == Some(workflow))
        });
    if !workflow_exists {
        return Err(Error::Protocol(
            "specialist workflow is absent from the worker catalog".into(),
        ));
    }
    Ok(expected_digest.into())
}

async fn emit_failure(
    tx: &async_channel::Sender<AgentEvent>,
    run: &RunId,
    code: &str,
    message: &str,
    failure_kind: RunFailureKind,
) {
    let _ = tx
        .send(AgentEvent::Error {
            code: code.into(),
            message: message.into(),
            run: Some(run.clone()),
        })
        .await;
    emit_finished(
        tx,
        run,
        RunStatus::Failed,
        Some(message.into()),
        Some(failure_kind),
    )
    .await;
}

async fn emit_finished(
    tx: &async_channel::Sender<AgentEvent>,
    run: &RunId,
    status: RunStatus,
    error: Option<String>,
    failure_kind: Option<RunFailureKind>,
) {
    let _ = tx
        .send(AgentEvent::RunFinished {
            run: run.clone(),
            outcome: RunOutcome {
                status,
                stop_reason: None,
                error,
                failure_kind,
                usage: None,
                execution: None,
            },
        })
        .await;
}

fn text_prompt(blocks: &[ContentBlock]) -> agent_core::Result<String> {
    let mut text = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text: value } => text.push(value.as_str()),
            _ => {
                return Err(Error::Unsupported(
                    "Scientist and RSI turns currently require text-only objectives".into(),
                ))
            }
        }
    }
    let message = text.join("\n").trim().to_string();
    if message.is_empty() {
        return Err(Error::Protocol(
            "specialist objective must not be empty".into(),
        ));
    }
    Ok(message)
}

fn portable_session_id(value: &str) -> String {
    let value = value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':') {
                byte as char
            } else {
                '-'
            }
        })
        .collect::<String>();
    if value.len() <= 128 {
        value
    } else {
        format!("session-{}", &value[..120])
    }
}

fn now_millis() -> agent_core::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|error| Error::Other(format!("system clock is before UNIX epoch: {error}")))
}

async fn write_private_json(path: &std::path::Path, value: &Value) -> agent_core::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::Io("specialist config path has no parent".into()))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| Error::Io("specialist config filename is invalid".into()))?;
    let temporary = parent.join(format!(".{name}.tmp-{}", Uuid::new_v4().simple()));
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let write_result = async {
        let mut file = options.open(&temporary).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temporary, path).await?;
        Ok::<(), std::io::Error>(())
    }
    .await;
    if let Err(error) = write_result {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(Error::Io(format!(
            "could not atomically write specialist config: {error}"
        )));
    }
    Ok(())
}

fn worker_error(error: WorkerFailure) -> Error {
    match error {
        WorkerFailure::Cancelled => Error::Other("specialist worker handshake cancelled".into()),
        WorkerFailure::TimedOut => Error::Transport("specialist worker handshake timed out".into()),
        WorkerFailure::Failed(error) => Error::Transport(error),
    }
}

fn classify_failure(error: &str) -> RunFailureKind {
    if let Some(kind) = classify_provider_access_failure(None, error) {
        return kind;
    }
    let lower = error.to_ascii_lowercase();
    if lower.contains("schema") || lower.contains("invalid json") {
        RunFailureKind::ProviderError
    } else {
        RunFailureKind::RuntimeInterrupted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(worker_version: &str) -> Value {
        let mut catalog = serde_json::json!({
            "schema_version": 1,
            "catalog_version": "1.0.0",
            "worker_version": worker_version,
            "trust": {
                "source": "signed_app_bundle",
                "requires_signed_release_binary": true
            },
            "specialists": [{
                "specialist_id": "scientist",
                "workflows": [{"id": "scientist:discover"}]
            }]
        });
        let digest = format!(
            "{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&catalog).unwrap())
        );
        catalog["catalog_sha256"] = Value::String(digest);
        catalog
    }

    #[test]
    fn failure_classification_does_not_confuse_authentication_with_billing() {
        assert_eq!(
            classify_failure("403 Forbidden: credit service credentials were rejected"),
            RunFailureKind::PlatformKeyRejected,
        );
        assert_eq!(
            classify_failure("402 Payment Required: insufficient credits"),
            RunFailureKind::InsufficientCredits,
        );
        assert_eq!(
            classify_failure("credit service temporarily unavailable"),
            RunFailureKind::RuntimeInterrupted,
        );
    }

    #[test]
    fn projection_excludes_private_result_and_message() {
        let data = serde_json::json!({
            "specialist": "scientist",
            "workflow": "scientist:discover",
            "programId": "p1",
            "message": "private-ish model prose",
            "result": {"full": "model response"},
            "researchProjection": {"payload": {"program_id": "p1"}},
            "cloudSync": {
                "scope_id": "specialist-session-1",
                "file_count": 2,
                "verified_segment_count": 3,
                "total_bytes": 99
            },
        });
        let projection = product_projection(
            &data,
            Some("org-1".into()),
            "b".repeat(64),
            Some("1.0.0".into()),
            Some("a".repeat(64)),
        );
        assert!(projection.get("result").is_none());
        assert!(projection.get("message").is_none());
        assert_eq!(projection["organizationId"], "org-1");
        assert_eq!(projection["programId"], "p1");
        assert_eq!(projection["cloudSync"]["verified_segment_count"], 3);
    }

    #[test]
    fn advisor_billed_usage_is_projected_into_the_run_receipt() {
        let usage = advisor_run_usage(&serde_json::json!({
            "cloudAdvisor": {
                "usage": {
                    "prompt_tokens": 1200,
                    "completion_tokens": 80,
                    "cost": 0.013904
                }
            }
        }))
        .expect("advisor usage");
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 80);
        assert_eq!(usage.context_tokens, 1200);
        assert_eq!(usage.cost_usd, Some(0.013904));
    }

    #[test]
    fn successful_specialist_turn_requires_a_verified_cloud_receipt() {
        let valid = serde_json::json!({
            "cloudSync": {
                "scope_id": "specialist-session-1",
                "file_count": 1,
                "verified_segment_count": 1,
                "total_bytes": 5
            }
        });
        assert!(validate_cloud_sync_receipt(&valid).is_ok());
        assert!(validate_cloud_sync_receipt(&serde_json::json!({})).is_err());
        let invalid = serde_json::json!({
            "cloudSync": {
                "scope_id": "../escape",
                "file_count": 1,
                "verified_segment_count": 1,
                "total_bytes": 5
            }
        });
        assert!(validate_cloud_sync_receipt(&invalid).is_err());
    }

    #[test]
    fn non_text_prompt_is_rejected() {
        let error = text_prompt(&[ContentBlock::thinking("hidden")]).unwrap_err();
        assert!(error.to_string().contains("text-only"));
    }

    #[test]
    fn catalog_attestation_accepts_only_the_exact_worker_and_workflow() {
        let catalog = catalog("0.1.0");
        let digest =
            validate_catalog(&catalog, "0.1.0", "scientist", "scientist:discover").unwrap();
        assert_eq!(catalog["catalog_sha256"].as_str(), Some(digest.as_str()));
        assert!(validate_catalog(&catalog, "0.2.0", "scientist", "scientist:discover").is_err());
        assert!(validate_catalog(&catalog, "0.1.0", "scientist", "scientist:replicate").is_err());
    }

    #[test]
    fn catalog_manifest_tampering_fails_digest_validation() {
        let mut catalog = catalog("0.1.0");
        catalog["specialists"][0]["workflows"][0]["id"] =
            Value::String("scientist:replicate".into());
        let error =
            validate_catalog(&catalog, "0.1.0", "scientist", "scientist:replicate").unwrap_err();
        assert!(error.to_string().contains("digest"));
    }
}
