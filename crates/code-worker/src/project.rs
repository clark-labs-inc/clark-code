//! Typed, project-confined executor primitives for the desktop's read-only and
//! explicitly confirmed project surfaces. This keeps those operations on the
//! same durable worker as the coding provider instead of opening a second
//! transport
//! side channel.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine;
use code_host::{HeadlessPlugin, PluginContext, PluginError, PluginManifest};
use exec_core::{collect_system_capabilities, Executor, LocalExecutor};
use exec_protocol::{
    method, CanonicalizeResult, MetaResult, PathParams, ProcessStartParams, ReadDirResult,
    ReadResult, RenameParams, SystemCapabilityCensusResult, WalkResult, WireDirEntry,
    WireWalkEntry, WriteNewResult, WriteParams,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const CALL: &str = "executor.call";

pub struct ProjectPlugin {
    manifest: PluginManifest,
}

impl ProjectPlugin {
    pub fn new() -> Self {
        Self {
            manifest: PluginManifest {
                id: "project".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: "Project-confined filesystem and process primitives".into(),
                operations: BTreeSet::from([CALL.into()]),
                capabilities: BTreeSet::from(["executor.project_confined".into()]),
            },
        }
    }

    async fn call(&self, context: PluginContext, input: Value) -> Result<Value, PluginError> {
        let root = context.project_root.clone().ok_or_else(|| {
            PluginError::InvalidInput("executor.call requires a registered project_id".into())
        })?;
        let request: ExecutorCall = decode(input)?;
        dispatch(&LocalExecutor, &root, &context, request).await
    }
}

impl Default for ProjectPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HeadlessPlugin for ProjectPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        context: PluginContext,
        operation: &str,
        input: Value,
    ) -> Result<Value, PluginError> {
        match operation {
            CALL => self.call(context, input).await,
            _ => Err(PluginError::UnsupportedOperation {
                plugin: self.manifest.id.clone(),
                operation: operation.into(),
            }),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutorCall {
    method: String,
    #[serde(default)]
    params: Value,
}

async fn dispatch(
    fs: &LocalExecutor,
    root: &Path,
    context: &PluginContext,
    request: ExecutorCall,
) -> Result<Value, PluginError> {
    match request.method.as_str() {
        method::ENV_CAPABILITY_CENSUS => {
            let census = collect_system_capabilities(None);
            encode(SystemCapabilityCensusResult {
                platform: census.platform,
                architecture: census.architecture,
                executable_names: census.executable_names,
                environment_variable_names: census.environment_variable_names,
                credential_surfaces: census.credential_surfaces,
                executables_truncated: census.executables_truncated,
                environment_names_truncated: census.environment_names_truncated,
            })
        }
        method::ENV_HOME => encode(CanonicalizeResult {
            path: root.to_string_lossy().into_owned(),
        }),
        method::FS_READ => {
            let path = path_param(request.params, root)?;
            encode(ReadResult {
                data: base64::engine::general_purpose::STANDARD
                    .encode(fs.read(&path).await.map_err(failed)?),
            })
        }
        method::FS_WRITE | method::FS_WRITE_PRIVATE | method::FS_WRITE_PRIVATE_NEW => {
            let params: WriteParams = decode(request.params)?;
            let path = confined(&params.path, root)?;
            let data = base64::engine::general_purpose::STANDARD
                .decode(params.data)
                .map_err(|_| PluginError::InvalidInput("file data is not valid base64".into()))?;
            match request.method.as_str() {
                method::FS_WRITE => {
                    fs.write(&path, &data).await.map_err(failed)?;
                    Ok(json!({}))
                }
                method::FS_WRITE_PRIVATE => {
                    fs.write_private(&path, &data).await.map_err(failed)?;
                    Ok(json!({}))
                }
                _ => encode(WriteNewResult {
                    created: fs.write_private_new(&path, &data).await.map_err(failed)?,
                }),
            }
        }
        method::FS_SYNC_FILE | method::FS_SYNC_DIRECTORY => {
            let path = path_param(request.params, root)?;
            if request.method == method::FS_SYNC_FILE {
                fs.sync_file(&path).await.map_err(failed)?;
            } else {
                fs.sync_directory(&path).await.map_err(failed)?;
            }
            Ok(json!({}))
        }
        method::FS_CREATE_DIR | method::FS_REMOVE_FILE | method::FS_REMOVE_DIR => {
            let path = path_param(request.params, root)?;
            match request.method.as_str() {
                method::FS_CREATE_DIR => fs.create_dir_all(&path).await,
                method::FS_REMOVE_FILE => fs.remove_file(&path).await,
                _ => fs.remove_dir_all(&path).await,
            }
            .map_err(failed)?;
            Ok(json!({}))
        }
        method::FS_RENAME => {
            let params: RenameParams = decode(request.params)?;
            fs.rename(&confined(&params.from, root)?, &confined(&params.to, root)?)
                .await
                .map_err(failed)?;
            Ok(json!({}))
        }
        method::FS_READ_DIR => {
            let path = path_param(request.params, root)?;
            let entries = fs.read_dir(&path).await.map_err(failed)?;
            encode(ReadDirResult {
                entries: entries
                    .into_iter()
                    .map(|entry| WireDirEntry {
                        name: entry.name,
                        is_dir: entry.is_dir,
                        is_symlink: entry.is_symlink,
                    })
                    .collect(),
            })
        }
        method::FS_METADATA => {
            let path = path_param(request.params, root)?;
            let metadata = fs.metadata(&path).await.map_err(failed)?;
            encode(MetaResult {
                modified_ms: metadata.modified.and_then(to_millis),
                len: metadata.len,
                is_dir: metadata.is_dir,
                is_symlink: metadata.is_symlink,
            })
        }
        method::FS_CANONICALIZE => {
            let path = path_param(request.params, root)?;
            let canonical = fs.canonicalize(&path).await.map_err(failed)?;
            ensure_resolved(&canonical, root)?;
            encode(CanonicalizeResult {
                path: canonical.to_string_lossy().into_owned(),
            })
        }
        method::FS_WALK => {
            let path = path_param(request.params, root)?;
            let entries = fs.walk(&path).await.map_err(failed)?;
            encode(WalkResult {
                entries: entries
                    .into_iter()
                    .map(|entry| WireWalkEntry {
                        path: entry.path.to_string_lossy().into_owned(),
                        modified_ms: entry.modified.and_then(to_millis),
                        len: entry.len,
                    })
                    .collect(),
            })
        }
        method::PROCESS_START => {
            let params: ProcessStartParams = decode(request.params)?;
            let cwd = confined(&params.cwd, root)?;
            let timeout = Duration::from_millis(params.timeout_ms.clamp(1, 24 * 60 * 60 * 1000));
            let output = fs
                .exec(&params.command, &cwd, timeout, &context.cancellation)
                .await
                .map_err(failed)?;
            Ok(json!({
                "stdout": base64::engine::general_purpose::STANDARD.encode(output.stdout),
                "stderr": base64::engine::general_purpose::STANDARD.encode(output.stderr),
                "code": output.code
            }))
        }
        _ => Err(PluginError::InvalidInput(format!(
            "unsupported project executor method: {}",
            request.method
        ))),
    }
}

fn path_param(value: Value, root: &Path) -> Result<PathBuf, PluginError> {
    let params: PathParams = decode(value)?;
    confined(&params.path, root)
}

fn confined(raw: &str, root: &Path) -> Result<PathBuf, PluginError> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return Err(PluginError::InvalidInput("project path is unsafe".into()));
    }
    if !path.starts_with(root) {
        return Err(PluginError::InvalidInput(
            "project path escapes its registered root".into(),
        ));
    }
    if let Some(parent) = path.parent().filter(|parent| parent.exists()) {
        ensure_resolved(
            &std::fs::canonicalize(parent).map_err(|error| failed(error.to_string()))?,
            root,
        )?;
    }
    Ok(path)
}

fn ensure_resolved(path: &Path, root: &Path) -> Result<(), PluginError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|error| failed(error.to_string()))?;
    if path.starts_with(&canonical_root) {
        Ok(())
    } else {
        Err(PluginError::InvalidInput(
            "project path resolves outside its registered root".into(),
        ))
    }
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, PluginError> {
    serde_json::from_value(value).map_err(|error| PluginError::InvalidInput(error.to_string()))
}

fn encode(value: impl serde::Serialize) -> Result<Value, PluginError> {
    serde_json::to_value(value).map_err(|error| PluginError::Failed(error.to_string()))
}

fn failed(error: String) -> PluginError {
    PluginError::Failed(error)
}

fn to_millis(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use code_host::{ProjectRegistration, Request, RequestCommand, Response, PROTOCOL_VERSION};
    use exec_protocol::{method, PathParams};

    use crate::config::WorkerConfig;

    async fn invoke(
        root: &std::path::Path,
        method_name: &str,
        params: serde_json::Value,
    ) -> Response {
        let config = WorkerConfig {
            projects: vec![ProjectRegistration {
                id: "fixture".into(),
                root: root.into(),
            }],
            trajectory_root: root.join(".agent-desktop/trajectory"),
            enabled_plugins: ["project".into()].into_iter().collect(),
            ..WorkerConfig::default()
        };
        let host = crate::build_host(&config).unwrap();
        host.handle(Request {
            schema_version: PROTOCOL_VERSION,
            request_id: "project-test".into(),
            command: RequestCommand::Invoke {
                plugin: "project".into(),
                operation: "executor.call".into(),
                project_id: Some("fixture".into()),
                input: serde_json::json!({ "method": method_name, "params": params }),
            },
        })
        .await
    }

    #[tokio::test]
    async fn reads_inside_registered_project_and_rejects_parent_escape() {
        let project = tempfile::tempdir().unwrap();
        let root = project.path().canonicalize().unwrap();
        std::fs::write(root.join("inside.txt"), b"inside").unwrap();
        let response = invoke(
            &root,
            method::FS_READ,
            serde_json::to_value(PathParams {
                path: root.join("inside.txt").to_string_lossy().into_owned(),
            })
            .unwrap(),
        )
        .await;
        let Response::Result { data, .. } = response else {
            panic!("read failed")
        };
        let encoded = data
            .get("data")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
            b"inside"
        );

        let response = invoke(
            &root,
            method::FS_READ,
            serde_json::to_value(PathParams {
                path: root.join("../outside.txt").to_string_lossy().into_owned(),
            })
            .unwrap(),
        )
        .await;
        assert!(matches!(response, Response::Error { .. }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_parent_escape() {
        use std::os::unix::fs::symlink;

        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"secret").unwrap();
        symlink(outside.path(), project.path().join("link")).unwrap();
        let response = invoke(
            project.path(),
            method::FS_READ,
            serde_json::to_value(PathParams {
                path: project
                    .path()
                    .join("link/secret")
                    .to_string_lossy()
                    .into_owned(),
            })
            .unwrap(),
        )
        .await;
        assert!(matches!(response, Response::Error { .. }));
    }
}
