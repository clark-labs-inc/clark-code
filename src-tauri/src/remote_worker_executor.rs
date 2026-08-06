//! `Executor` adapter over the project-confined operation exposed by a durable
//! Clark Code worker. The worker process and SSH transport remain native-owned;
//! callers hold only this cloneable, account-authorized attachment.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use code_host::{Request, RequestCommand, Response, PROTOCOL_VERSION};
use code_remote::RemoteWorkerSlot;
use exec_core::{
    DirEntry, ExecOutput, ExecResult, ExecutionContainment, Executor, FileMeta,
    SystemCapabilityCensus, WalkEntry,
};
use exec_protocol::{
    b64_decode, b64_encode, method, CanonicalizeResult, MetaResult, PathParams, ProcessStartParams,
    ReadDirResult, ReadResult, RenameParams, SystemCapabilityCensusResult, WalkResult,
    WriteNewResult, WriteParams,
};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct RemoteWorkerExecutor {
    worker: Arc<RemoteWorkerSlot>,
    project_id: String,
}

impl RemoteWorkerExecutor {
    pub(crate) fn new(worker: Arc<RemoteWorkerSlot>, project_id: String) -> Self {
        Self { worker, project_id }
    }

    fn request(&self, request_id: String, method: &str, params: serde_json::Value) -> Request {
        Request {
            schema_version: PROTOCOL_VERSION,
            request_id,
            command: RequestCommand::Invoke {
                plugin: "project".into(),
                operation: "executor.call".into(),
                project_id: Some(self.project_id.clone()),
                input: serde_json::json!({ "method": method, "params": params }),
            },
        }
    }

    async fn call(&self, method: &str, params: serde_json::Value) -> ExecResult<serde_json::Value> {
        let id = format!("executor-{}", uuid::Uuid::new_v4().simple());
        let response = self
            .worker
            .request(self.request(id, method, params))
            .await
            .map_err(|error| error.to_string())?;
        response_data(response)
    }

    async fn call_cancellable(
        &self,
        method: &str,
        params: serde_json::Value,
        cancel: &CancellationToken,
    ) -> ExecResult<serde_json::Value> {
        let id = format!("executor-{}", uuid::Uuid::new_v4().simple());
        let mut request = self
            .worker
            .start_request(self.request(id.clone(), method, params))
            .await
            .map_err(|error| error.to_string())?;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let cancellation = Request {
                        schema_version: PROTOCOL_VERSION,
                        request_id: format!("cancel-{}", uuid::Uuid::new_v4().simple()),
                        command: RequestCommand::Cancel { target_request_id: id },
                    };
                    let _ = self.worker.request(cancellation).await;
                    return Err("command cancelled".into());
                }
                frame = request.next() => match frame.map_err(|error| error.to_string())? {
                    code_remote::RemoteWorkerFrame::Progress(_) => {}
                    code_remote::RemoteWorkerFrame::Terminal(response) => return response_data(response),
                }
            }
        }
    }
}

#[async_trait]
impl Executor for RemoteWorkerExecutor {
    async fn system_capability_census(&self) -> ExecResult<SystemCapabilityCensus> {
        let value: SystemCapabilityCensusResult = decode(
            self.call(method::ENV_CAPABILITY_CENSUS, serde_json::json!({}))
                .await?,
        )?;
        Ok(SystemCapabilityCensus {
            platform: value.platform,
            architecture: value.architecture,
            executable_names: value.executable_names,
            environment_variable_names: value.environment_variable_names,
            credential_surfaces: value.credential_surfaces,
            executables_truncated: value.executables_truncated,
            environment_names_truncated: value.environment_names_truncated,
        })
    }

    fn containment(&self) -> ExecutionContainment {
        ExecutionContainment::External
    }

    fn is_local(&self) -> bool {
        false
    }

    async fn read(&self, path: &Path) -> ExecResult<Vec<u8>> {
        let result: ReadResult = decode(self.call(method::FS_READ, path_value(path)).await?)?;
        b64_decode(&result.data)
    }

    async fn write(&self, path: &Path, data: &[u8]) -> ExecResult<()> {
        self.write_method(method::FS_WRITE, path, data).await
    }

    async fn write_private(&self, path: &Path, data: &[u8]) -> ExecResult<()> {
        self.write_method(method::FS_WRITE_PRIVATE, path, data)
            .await
    }

    async fn write_private_new(&self, path: &Path, data: &[u8]) -> ExecResult<bool> {
        let result: WriteNewResult = decode(
            self.call(
                method::FS_WRITE_PRIVATE_NEW,
                encode(&WriteParams {
                    path: wire_path(path),
                    data: b64_encode(data),
                }),
            )
            .await?,
        )?;
        Ok(result.created)
    }

    async fn sync_file(&self, path: &Path) -> ExecResult<()> {
        self.unit(method::FS_SYNC_FILE, path_value(path)).await
    }

    async fn sync_directory(&self, path: &Path) -> ExecResult<()> {
        self.unit(method::FS_SYNC_DIRECTORY, path_value(path)).await
    }

    async fn create_dir_all(&self, path: &Path) -> ExecResult<()> {
        self.unit(method::FS_CREATE_DIR, path_value(path)).await
    }

    async fn remove_file(&self, path: &Path) -> ExecResult<()> {
        self.unit(method::FS_REMOVE_FILE, path_value(path)).await
    }

    async fn remove_dir_all(&self, path: &Path) -> ExecResult<()> {
        self.unit(method::FS_REMOVE_DIR, path_value(path)).await
    }

    async fn rename(&self, from: &Path, to: &Path) -> ExecResult<()> {
        self.unit(
            method::FS_RENAME,
            encode(&RenameParams {
                from: wire_path(from),
                to: wire_path(to),
            }),
        )
        .await
    }

    async fn read_dir(&self, path: &Path) -> ExecResult<Vec<DirEntry>> {
        let result: ReadDirResult =
            decode(self.call(method::FS_READ_DIR, path_value(path)).await?)?;
        Ok(result
            .entries
            .into_iter()
            .map(|entry| DirEntry {
                name: entry.name,
                is_dir: entry.is_dir,
                is_symlink: entry.is_symlink,
            })
            .collect())
    }

    async fn metadata(&self, path: &Path) -> ExecResult<FileMeta> {
        let result: MetaResult = decode(self.call(method::FS_METADATA, path_value(path)).await?)?;
        Ok(FileMeta {
            modified: result.modified_ms.map(millis_to_time),
            len: result.len,
            is_dir: result.is_dir,
            is_symlink: result.is_symlink,
        })
    }

    async fn canonicalize(&self, path: &Path) -> ExecResult<PathBuf> {
        let result: CanonicalizeResult =
            decode(self.call(method::FS_CANONICALIZE, path_value(path)).await?)?;
        Ok(PathBuf::from(result.path))
    }

    async fn home_dir(&self, _cwd: &Path) -> ExecResult<PathBuf> {
        let result: CanonicalizeResult =
            decode(self.call(method::ENV_HOME, serde_json::json!({})).await?)?;
        Ok(PathBuf::from(result.path))
    }

    async fn walk(&self, root: &Path) -> ExecResult<Vec<WalkEntry>> {
        let result: WalkResult = decode(self.call(method::FS_WALK, path_value(root)).await?)?;
        Ok(result
            .entries
            .into_iter()
            .map(|entry| WalkEntry {
                path: PathBuf::from(entry.path),
                modified: entry.modified_ms.map(millis_to_time),
                len: entry.len,
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
        #[derive(serde::Deserialize)]
        struct Output {
            stdout: String,
            stderr: String,
            code: Option<i32>,
        }
        let value = self
            .call_cancellable(
                method::PROCESS_START,
                encode(&ProcessStartParams {
                    process_id: format!("process-{}", uuid::Uuid::new_v4().simple()),
                    command: command.into(),
                    cwd: wire_path(cwd),
                    timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                    pty: false,
                }),
                cancel,
            )
            .await?;
        let output: Output = decode(value)?;
        Ok(ExecOutput {
            stdout: b64_decode(&output.stdout)?,
            stderr: b64_decode(&output.stderr)?,
            code: output.code,
        })
    }
}

impl RemoteWorkerExecutor {
    async fn write_method(&self, method_name: &str, path: &Path, data: &[u8]) -> ExecResult<()> {
        self.unit(
            method_name,
            encode(&WriteParams {
                path: wire_path(path),
                data: b64_encode(data),
            }),
        )
        .await
    }

    async fn unit(&self, method_name: &str, params: serde_json::Value) -> ExecResult<()> {
        self.call(method_name, params).await.map(|_| ())
    }
}

fn response_data(response: Response) -> ExecResult<serde_json::Value> {
    match response {
        Response::Result { kind, data, .. } if kind == "plugin_result" => Ok(data),
        Response::Error { code, message, .. } => Err(format!("remote worker {code}: {message}")),
        other => Err(format!("unexpected remote worker response: {other:?}")),
    }
}

fn encode(value: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

fn decode<T: DeserializeOwned>(value: serde_json::Value) -> ExecResult<T> {
    serde_json::from_value(value)
        .map_err(|error| format!("malformed remote worker response: {error}"))
}

fn path_value(path: &Path) -> serde_json::Value {
    encode(&PathParams {
        path: wire_path(path),
    })
}

fn wire_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn millis_to_time(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}
