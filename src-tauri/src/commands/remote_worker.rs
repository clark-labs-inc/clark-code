//! Tauri controls for a worker whose model, tools, project, and trajectory
//! all live on an SSH host. The worker is the only remote execution surface:
//! model loop, project tools,
//! trajectory, and desktop inspection all share this durable process.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::product::{ProductRemoteWorkerRequest, ProductRequestContext};
use crate::runtime_registry::{AccountKey, WorkerConnectionKind};
use crate::AppState;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteWorkerConnectInput {
    pub host: String,
    pub remote_root: PathBuf,
    pub model: String,
    pub reasoning_effort: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWorkerConnection {
    pub id: String,
    pub cwd: String,
    pub arch: String,
    pub ssh_transport: String,
    pub connection_kind: RemoteConnectionKind,
    pub connect_duration_ms: u64,
    pub account_worker_count: usize,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteConnectionKind {
    Started,
    Reused,
    Replaced,
}

impl From<WorkerConnectionKind> for RemoteConnectionKind {
    fn from(value: WorkerConnectionKind) -> Self {
        match value {
            WorkerConnectionKind::Started => Self::Started,
            WorkerConnectionKind::Reused => Self::Reused,
            WorkerConnectionKind::Replaced => Self::Replaced,
        }
    }
}

/// Deploy and start a complete remote worker over SSH. The returned id is a
/// native-only handle; no credential or private SSH material crosses into the
/// webview.
#[tauri::command]
pub async fn remote_worker_connect(
    input: RemoteWorkerConnectInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RemoteWorkerConnection, String> {
    connect_remote_worker(input, &app, state.inner()).await
}

async fn connect_remote_worker(
    input: RemoteWorkerConnectInput,
    app: &AppHandle,
    state: &AppState,
) -> Result<RemoteWorkerConnection, String> {
    // One account lifecycle owns validation, credential lookup, worker
    // publication, and the returned receipt. Sign-out cannot remove the
    // partition midway and leave a newly published orphan worker behind.
    let _account_lifecycle = state.account_lifecycle.read().await;
    let launch = state
        .product
        .prepare_remote_worker(
            ProductRemoteWorkerRequest {
                host: input.host,
                remote_root: input.remote_root,
                model: input.model,
                reasoning_effort: input.reasoning_effort,
            },
            ProductRequestContext { app, state },
        )
        .await?
        .ok_or("this product does not provide remote workers")?;
    let account = AccountKey::new(launch.owner_scope)?;
    let spec = launch.spec;
    let credentials = launch.credentials;
    let started_at = std::time::Instant::now();
    let (handle, runtime, connection_kind) = state
        .runtime_registry
        .connect(account.clone(), spec, credentials)
        .await
        .inspect_err(|_| {
            tracing::warn!(
                event = "remote_worker_connect_failed",
                stage = "registry",
                "remote worker capability failed"
            );
        })?;
    let connect_duration_ms = started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let account_worker_count = state
        .runtime_registry
        .worker_count_for_account(&account)
        .await;
    let connection = RemoteWorkerConnection {
        id: handle.as_str().to_string(),
        cwd: runtime.project_root().to_string_lossy().into_owned(),
        arch: runtime.info().arch.clone(),
        ssh_transport: runtime.info().ssh_transport.clone(),
        connection_kind: connection_kind.into(),
        connect_duration_ms,
        account_worker_count,
    };
    let connection_kind = match connection.connection_kind {
        RemoteConnectionKind::Started => "started",
        RemoteConnectionKind::Reused => "reused",
        RemoteConnectionKind::Replaced => "replaced",
    };
    tracing::info!(
        event = "remote_worker_connected",
        connection_kind,
        connect_duration_ms = connection.connect_duration_ms,
        account_worker_count = connection.account_worker_count,
        ssh_transport = %connection.ssh_transport,
        worker_arch = %connection.arch,
        "remote worker capability ready"
    );
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::{RemoteConnectionKind, RemoteWorkerConnection};

    #[test]
    fn connection_receipt_is_typed_and_contains_no_account_identity() {
        let value = serde_json::to_value(RemoteWorkerConnection {
            id: "worker-0123456789abcdef0123456789abcdef".into(),
            cwd: "/srv/project".into(),
            arch: "linux-x86_64".into(),
            ssh_transport: "control_master".into(),
            connection_kind: RemoteConnectionKind::Reused,
            connect_duration_ms: 7,
            account_worker_count: 1,
        })
        .unwrap();

        assert_eq!(value["connectionKind"], "reused");
        assert_eq!(value["sshTransport"], "control_master");
        assert_eq!(value["connectDurationMs"], 7);
        assert_eq!(value["accountWorkerCount"], 1);
        assert!(value.get("account").is_none());
        assert!(value.get("token").is_none());
    }
}
