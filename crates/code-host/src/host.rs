use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::contract::{ProjectRegistry, RegistryError};
use crate::idempotency::{IdempotencyStore, Reservation};
use crate::plugin::{HeadlessPlugin, PluginContext, PluginError, PluginRegistry, ProgressCapture};
use crate::protocol::{Request, RequestCommand, Response};
use crate::trajectory::{now_millis, TrajectoryRecord, TrajectoryStatus};
use crate::PROTOCOL_VERSION;

/// Provider- and model-neutral headless host. The host is the policy boundary;
/// plugins are extensions that receive only resolved, host-owned context.
#[derive(Clone)]
pub struct HeadlessHost {
    projects: ProjectRegistry,
    trajectory_root: PathBuf,
    plugins: PluginRegistry,
    active: Arc<Mutex<std::collections::BTreeMap<String, CancellationToken>>>,
    trajectory_lock: Arc<Mutex<Option<u64>>>,
    idempotency: IdempotencyStore,
}

impl HeadlessHost {
    pub fn new(projects: ProjectRegistry, trajectory_root: impl Into<PathBuf>) -> Self {
        let trajectory_root = trajectory_root.into();
        Self {
            projects,
            idempotency: IdempotencyStore::new(&trajectory_root),
            trajectory_root,
            plugins: PluginRegistry::default(),
            active: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            trajectory_lock: Arc::new(Mutex::new(None)),
        }
    }

    pub fn trajectory_root(&self) -> &Path {
        &self.trajectory_root
    }

    pub fn register_plugin<P>(&mut self, plugin: P) -> Result<(), PluginError>
    where
        P: HeadlessPlugin + 'static,
    {
        self.plugins.register(plugin)
    }

    pub fn catalog(&self) -> Vec<crate::PluginManifest> {
        self.plugins.catalog()
    }

    pub fn project_root(&self, project_id: &str) -> Result<PathBuf, HostError> {
        Ok(self.projects.resolve(project_id)?.to_path_buf())
    }

    pub async fn handle(&self, request: Request) -> Response {
        self.handle_inner(request, None).await
    }

    /// Handle one request while forwarding ordered progress frames to the
    /// worker's bounded stdout queue. Only the worker composition root should
    /// supply this channel; direct host callers retain the terminal-only API.
    pub async fn handle_stream(
        &self,
        request: Request,
        output: tokio::sync::mpsc::Sender<Response>,
    ) -> Response {
        self.handle_inner(request, Some(output)).await
    }

    async fn handle_inner(
        &self,
        request: Request,
        progress_output: Option<tokio::sync::mpsc::Sender<Response>>,
    ) -> Response {
        let request_id = request.request_id.clone();
        if request.schema_version != PROTOCOL_VERSION {
            return Response::error(
                Some(request_id),
                "unsupported_schema",
                format!("schema_version must be {PROTOCOL_VERSION}"),
            );
        }
        if !portable_request_id(&request.request_id) {
            return Response::error(
                Some(request_id),
                "invalid_request_id",
                "request_id must be a bounded portable identifier",
            );
        }

        match request.command {
            RequestCommand::Ping => Response::result(
                Some(request_id),
                "pong",
                json!({
                    "host": "code-host",
                    "protocol_version": PROTOCOL_VERSION,
                    "quiet_by_default": true,
                }),
            ),
            RequestCommand::Catalog => Response::result(
                Some(request_id),
                "catalog",
                json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "plugins": self.catalog(),
                }),
            ),
            RequestCommand::Cancel { target_request_id } => {
                let cancelled = self
                    .active
                    .lock()
                    .await
                    .get(&target_request_id)
                    .map(|token| {
                        token.cancel();
                        true
                    })
                    .unwrap_or(false);
                Response::result(
                    Some(request_id),
                    "cancelled",
                    json!({"target_request_id": target_request_id, "active": cancelled}),
                )
            }
            RequestCommand::Shutdown => {
                let active = self.active.lock().await;
                let cancelled = active.len();
                for token in active.values() {
                    token.cancel();
                }
                Response::result(
                    Some(request_id),
                    "shutdown",
                    json!({"accepted": true, "cancelled_active": cancelled}),
                )
            }
            RequestCommand::Invoke {
                plugin,
                operation,
                project_id,
                input,
            } => {
                self.invoke(
                    &request_id,
                    &plugin,
                    &operation,
                    project_id.as_deref(),
                    input,
                    progress_output,
                )
                .await
            }
        }
    }

    async fn invoke(
        &self,
        request_id: &str,
        plugin: &str,
        operation: &str,
        project_id: Option<&str>,
        input: serde_json::Value,
        progress_output: Option<tokio::sync::mpsc::Sender<Response>>,
    ) -> Response {
        let project_root = match project_id {
            Some(id) => match self.projects.resolve(id) {
                Ok(root) => Some(root.to_path_buf()),
                Err(error) => {
                    return Response::error(
                        Some(request_id.into()),
                        "unknown_project",
                        error.to_string(),
                    )
                }
            },
            None => None,
        };
        let cancellation = CancellationToken::new();
        let mut active = self.active.lock().await;
        if active.contains_key(request_id) {
            return Response::error(
                Some(request_id.into()),
                "duplicate_request_id",
                "an invocation with this request_id is already active",
            );
        }
        active.insert(request_id.to_string(), cancellation.clone());
        drop(active);

        let request_contract = json!({
            "schema_version": PROTOCOL_VERSION,
            "plugin": plugin,
            "operation": operation,
            "project_id": project_id,
            "input": input,
        });
        let request_hash = match self
            .idempotency
            .reserve(request_id, &request_contract)
            .await
        {
            Ok(Reservation::Fresh { request_hash }) => request_hash,
            Ok(Reservation::Replay { progress, terminal }) => {
                self.active.lock().await.remove(request_id);
                if let Some(output) = progress_output {
                    for frame in progress {
                        if output.send(frame).await.is_err() {
                            return Response::error(
                                Some(request_id.into()),
                                "replay_disconnected",
                                "remote request replay channel closed",
                            );
                        }
                    }
                }
                return terminal;
            }
            Ok(Reservation::Ambiguous) => {
                self.active.lock().await.remove(request_id);
                return Response::error(
                    Some(request_id.into()),
                    "ambiguous_request",
                    "this request may already have executed; the agent will not run it twice",
                );
            }
            Ok(Reservation::Conflict) => {
                self.active.lock().await.remove(request_id);
                return Response::error(
                    Some(request_id.into()),
                    "request_id_conflict",
                    "request_id was already used for different work",
                );
            }
            Ok(Reservation::CapacityExhausted) => {
                self.active.lock().await.remove(request_id);
                return Response::error(
                    Some(request_id.into()),
                    "receipt_capacity_exhausted",
                    "durable request receipt storage is full; the agent will not run untracked work",
                );
            }
            Err(error) => {
                self.active.lock().await.remove(request_id);
                return Response::error(Some(request_id.into()), "receipt_failed", error);
            }
        };

        let started = self
            .record(TrajectoryRecord {
                schema_version: PROTOCOL_VERSION,
                sequence: 0,
                timestamp_ms: now_millis(),
                request_id: request_id.to_string(),
                plugin: plugin.to_string(),
                operation: operation.to_string(),
                status: TrajectoryStatus::Started,
                error: None,
            })
            .await;
        if let Err(error) = started {
            self.active.lock().await.remove(request_id);
            return Response::error(
                Some(request_id.into()),
                "trajectory_failed",
                error.to_string(),
            );
        }

        let progress_capture = ProgressCapture::default();
        let result = self
            .plugins
            .invoke(
                plugin,
                operation,
                PluginContext {
                    request_id: request_id.to_string(),
                    project_id: project_id.map(str::to_string),
                    project_root,
                    trajectory_root: self.trajectory_root.clone(),
                    cancellation: cancellation.clone(),
                    progress: crate::ProgressReporter::new(request_id.to_string(), progress_output)
                        .with_capture(progress_capture.clone()),
                },
                input,
            )
            .await;
        self.active.lock().await.remove(request_id);

        let (status, response) = match result {
            Ok(data) => (
                TrajectoryStatus::Completed,
                Response::result(Some(request_id.into()), "plugin_result", data),
            ),
            Err(PluginError::Cancelled) => (
                TrajectoryStatus::Cancelled,
                Response::error(
                    Some(request_id.into()),
                    "cancelled",
                    "plugin invocation cancelled",
                ),
            ),
            Err(error) => (
                TrajectoryStatus::Failed,
                Response::error(Some(request_id.into()), "plugin_failed", error.to_string()),
            ),
        };
        let error = match &response {
            Response::Error { message, .. } => Some(message.clone()),
            Response::Result { .. } => None,
            Response::Progress { .. } => unreachable!("plugin completion must be terminal"),
        };
        if let Err(record_error) = self
            .record(TrajectoryRecord {
                schema_version: PROTOCOL_VERSION,
                sequence: 0,
                timestamp_ms: now_millis(),
                request_id: request_id.to_string(),
                plugin: plugin.to_string(),
                operation: operation.to_string(),
                status,
                error,
            })
            .await
        {
            return Response::error(
                Some(request_id.into()),
                "trajectory_failed",
                record_error.to_string(),
            );
        }
        let captured = progress_capture.finish().await;
        if let Err(error) = self
            .idempotency
            .complete(request_id, request_hash, captured, response.clone())
            .await
        {
            return Response::error(Some(request_id.into()), "receipt_failed", error);
        }
        response
    }

    async fn record(&self, mut record: TrajectoryRecord) -> Result<(), HostError> {
        let mut sequence = self.trajectory_lock.lock().await;
        tokio::fs::create_dir_all(&self.trajectory_root).await?;
        if sequence.is_none() {
            *sequence = Some(scan_next_sequence(&self.trajectory_root).await?);
        }
        record.sequence = sequence.expect("trajectory sequence was initialized");
        *sequence = Some(record.sequence.saturating_add(1));
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(TrajectoryRecord::path(&self.trajectory_root))
            .await?;
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        file.write_all(&line).await?;
        file.flush().await?;
        Ok(())
    }
}

async fn scan_next_sequence(root: &Path) -> Result<u64, HostError> {
    let path = TrajectoryRecord::path(root);
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut next = 0;
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        let record: TrajectoryRecord = serde_json::from_str(line)?;
        next = next.max(record.sequence.saturating_add(1));
    }
    Ok(next)
}

fn portable_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Plugin(#[from] PluginError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use super::*;

    struct EchoPlugin {
        manifest: crate::PluginManifest,
    }

    struct ProgressPlugin {
        manifest: crate::PluginManifest,
    }

    #[async_trait]
    impl HeadlessPlugin for EchoPlugin {
        fn manifest(&self) -> &crate::PluginManifest {
            &self.manifest
        }

        async fn invoke(
            &self,
            context: crate::PluginContext,
            operation: &str,
            input: serde_json::Value,
        ) -> Result<serde_json::Value, crate::PluginError> {
            Ok(json!({
                "operation": operation,
                "request_id": context.request_id,
                "input": input,
            }))
        }
    }

    #[async_trait]
    impl HeadlessPlugin for ProgressPlugin {
        fn manifest(&self) -> &crate::PluginManifest {
            &self.manifest
        }

        async fn invoke(
            &self,
            context: crate::PluginContext,
            _operation: &str,
            _input: serde_json::Value,
        ) -> Result<serde_json::Value, crate::PluginError> {
            context
                .progress
                .emit("agent_event", json!({"part": 1}))
                .await?;
            context
                .progress
                .emit("agent_event", json!({"part": 2}))
                .await?;
            Ok(json!({"complete": true}))
        }
    }

    #[tokio::test]
    async fn host_dispatches_plugins_and_writes_trajectory() {
        let temp = tempfile::tempdir().unwrap();
        let projects = ProjectRegistry::new([crate::ProjectRegistration {
            id: "fixture".into(),
            root: temp.path().to_path_buf(),
        }])
        .unwrap();
        let mut host = HeadlessHost::new(projects, temp.path().join("trajectory"));
        host.register_plugin(EchoPlugin {
            manifest: crate::PluginManifest {
                id: "echo".into(),
                version: "1.0.0".into(),
                description: "test plugin".into(),
                operations: BTreeSet::from(["echo".into()]),
                capabilities: BTreeSet::new(),
            },
        })
        .unwrap();
        let response = host
            .handle(Request {
                schema_version: PROTOCOL_VERSION,
                request_id: "request-1".into(),
                command: RequestCommand::Invoke {
                    plugin: "echo".into(),
                    operation: "echo".into(),
                    project_id: Some("fixture".into()),
                    input: json!({"value": 7}),
                },
            })
            .await;
        match response {
            Response::Result { kind, data, .. } => {
                assert_eq!(kind, "plugin_result");
                assert_eq!(data["input"]["value"], 7);
            }
            other => panic!("unexpected response: {other:?}"),
        }
        let trajectory = tokio::fs::read_to_string(temp.path().join("trajectory/trajectory.jsonl"))
            .await
            .unwrap();
        assert_eq!(trajectory.lines().count(), 2);
    }

    #[tokio::test]
    async fn full_receipt_store_refuses_invocation_before_plugin_execution() {
        let temp = tempfile::tempdir().unwrap();
        let projects = ProjectRegistry::new([crate::ProjectRegistration {
            id: "fixture".into(),
            root: temp.path().to_path_buf(),
        }])
        .unwrap();
        let trajectory_root = temp.path().join("trajectory");
        let mut host = HeadlessHost::new(projects, &trajectory_root);
        host.idempotency = IdempotencyStore::with_capacity(&trajectory_root, 1);
        host.register_plugin(EchoPlugin {
            manifest: crate::PluginManifest {
                id: "echo".into(),
                version: "1.0.0".into(),
                description: "test plugin".into(),
                operations: BTreeSet::from(["echo".into()]),
                capabilities: BTreeSet::new(),
            },
        })
        .unwrap();

        let response = host
            .handle(Request {
                schema_version: PROTOCOL_VERSION,
                request_id: "capacity-1".into(),
                command: RequestCommand::Invoke {
                    plugin: "echo".into(),
                    operation: "echo".into(),
                    project_id: Some("fixture".into()),
                    input: json!({"value": 7}),
                },
            })
            .await;

        assert!(matches!(
            response,
            Response::Error { ref code, .. } if code == "receipt_capacity_exhausted"
        ));
        assert!(!trajectory_root.join("trajectory.jsonl").exists());
    }

    #[tokio::test]
    async fn streaming_host_emits_ordered_progress_before_one_terminal_response() {
        let temp = tempfile::tempdir().unwrap();
        let projects = ProjectRegistry::new([crate::ProjectRegistration {
            id: "fixture".into(),
            root: temp.path().to_path_buf(),
        }])
        .unwrap();
        let mut host = HeadlessHost::new(projects, temp.path().join("trajectory"));
        host.register_plugin(ProgressPlugin {
            manifest: crate::PluginManifest {
                id: "progress".into(),
                version: "1.0.0".into(),
                description: "test plugin".into(),
                operations: BTreeSet::from(["run".into()]),
                capabilities: BTreeSet::new(),
            },
        })
        .unwrap();
        let (output, mut progress) = tokio::sync::mpsc::channel(4);
        let terminal = host
            .handle_stream(
                Request {
                    schema_version: PROTOCOL_VERSION,
                    request_id: "stream-1".into(),
                    command: RequestCommand::Invoke {
                        plugin: "progress".into(),
                        operation: "run".into(),
                        project_id: Some("fixture".into()),
                        input: Value::Null,
                    },
                },
                output,
            )
            .await;

        for expected in 0..2 {
            match progress.recv().await.expect("progress frame") {
                Response::Progress {
                    request_id,
                    sequence,
                    kind,
                    ..
                } => {
                    assert_eq!(request_id.as_deref(), Some("stream-1"));
                    assert_eq!(sequence, expected);
                    assert_eq!(kind, "agent_event");
                }
                other => panic!("unexpected frame: {other:?}"),
            }
        }
        assert!(matches!(terminal, Response::Result { .. }));
    }

    #[tokio::test]
    async fn cancel_is_visible_to_a_plugin() {
        struct CancelPlugin {
            manifest: crate::PluginManifest,
        }

        #[async_trait]
        impl HeadlessPlugin for CancelPlugin {
            fn manifest(&self) -> &crate::PluginManifest {
                &self.manifest
            }

            async fn invoke(
                &self,
                context: crate::PluginContext,
                _operation: &str,
                _input: serde_json::Value,
            ) -> Result<serde_json::Value, crate::PluginError> {
                context.cancellation.cancelled().await;
                Err(crate::PluginError::Cancelled)
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let projects = ProjectRegistry::new([crate::ProjectRegistration {
            id: "fixture".into(),
            root: temp.path().to_path_buf(),
        }])
        .unwrap();
        let mut host = HeadlessHost::new(projects, temp.path().join("trajectory"));
        host.register_plugin(CancelPlugin {
            manifest: crate::PluginManifest {
                id: "cancel".into(),
                version: "1.0.0".into(),
                description: "test plugin".into(),
                operations: BTreeSet::from(["wait".into()]),
                capabilities: BTreeSet::new(),
            },
        })
        .unwrap();
        let invoke_host = host.clone();
        let invocation = tokio::spawn(async move {
            invoke_host
                .handle(Request {
                    schema_version: PROTOCOL_VERSION,
                    request_id: "long-running".into(),
                    command: RequestCommand::Invoke {
                        plugin: "cancel".into(),
                        operation: "wait".into(),
                        project_id: None,
                        input: serde_json::Value::Null,
                    },
                })
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let cancel = host
            .handle(Request {
                schema_version: PROTOCOL_VERSION,
                request_id: "cancel-request".into(),
                command: RequestCommand::Cancel {
                    target_request_id: "long-running".into(),
                },
            })
            .await;
        assert!(matches!(cancel, Response::Result { .. }));
        assert!(matches!(invocation.await.unwrap(), Response::Error { .. }));
    }
}
