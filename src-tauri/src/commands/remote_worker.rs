//! Tauri controls for a worker whose model, tools, project, and trajectory
//! all live on an SSH host. The worker is the only remote execution surface:
//! model loop, project tools,
//! trajectory, and desktop inspection all share this durable process.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use clark_install_context::{InstallContext, CODE_REMOTE_LINUX_X86_64};
use code_remote::RemoteWorkerSpec;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

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

fn validate_model(model: &str, reasoning_effort: &str) -> Result<(), String> {
    if !matches!(
        model,
        "clark-code:free"
            | "clark-code:glm52"
            | "clark-code:kimi_k3"
            | "clark-code:deepseek_v4_flash_latest"
    ) {
        return Err("remote worker model is not in the Clark Code catalog".into());
    }
    if !matches!(
        reasoning_effort,
        "max" | "xhigh" | "high" | "medium" | "low" | "minimal"
    ) {
        return Err("remote worker reasoning effort is invalid".into());
    }
    Ok(())
}

fn build_spec(input: RemoteWorkerConnectInput) -> Result<RemoteWorkerSpec, String> {
    validate_model(&input.model, &input.reasoning_effort)?;
    let root = input.remote_root;
    let identity = format!("{}\0{}", input.host, root.display());
    let project_id = format!("project-{:x}", Sha256::digest(identity.as_bytes()));
    let trajectory_root = root.join(".clark").join("trajectory");
    let worker_config = serde_json::json!({
        "schema_version": 1,
        "worker_name": "clark-code-worker",
        "projects": [{ "id": project_id, "root": root }],
        "trajectory_root": trajectory_root,
        "provider": {
            "base_url": "https://api.clarkslabs.com/v1",
            "model": input.model,
            "api_key_env": "CLARK_CODE_API_KEY",
            "reasoning_effort": input.reasoning_effort,
            "allowed_tools": BTreeSet::from(["bash", "write_file", "edit_file"]),
            "allowed_command_prefixes": Vec::<String>::new()
        },
        "enabled_plugins": ["coding", "project"],
        "execution_residency": "remote_worker"
    });
    let binary = InstallContext::current()
        .bundled_tool(CODE_REMOTE_LINUX_X86_64)
        .ok_or("Clark Desktop is missing its signed remote coding worker")?;
    Ok(RemoteWorkerSpec {
        host: input.host,
        project_id,
        remote_root: root,
        trajectory_root,
        worker_config,
        local_binary: None,
        local_binaries: BTreeMap::from([("linux-x86_64".into(), binary)]),
        remote_binary: None,
        credential_envs: vec!["CLARK_CODE_API_KEY".into()],
    })
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

async fn current_account(state: &AppState) -> Result<(String, AccountKey), String> {
    let owner = state
        .runtime_registry
        .cloud_account()
        .await
        .map(|account| account.account.as_str().to_string())
        .ok_or("Clark must be signed in before starting a remote worker")?;
    let account = AccountKey::new(owner.clone())?;
    Ok((owner, account))
}

async fn code_credentials(
    state: &AppState,
    owner: &str,
) -> Result<HashMap<String, String>, String> {
    let secret = state
        .credentials
        .code_key(owner)
        .await?
        .ok_or("Clark Code credential is not provisioned for this account")?;
    Ok(HashMap::from([(
        "CLARK_CODE_API_KEY".into(),
        secret.as_str().to_string(),
    )]))
}

/// Deploy and start a complete remote worker over SSH. The returned id is a
/// native-only handle; no credential or private SSH material crosses into the
/// webview.
#[tauri::command]
pub async fn remote_worker_connect(
    input: RemoteWorkerConnectInput,
    state: State<'_, AppState>,
) -> Result<RemoteWorkerConnection, String> {
    connect_remote_worker(input, state.inner()).await
}

async fn connect_remote_worker(
    input: RemoteWorkerConnectInput,
    state: &AppState,
) -> Result<RemoteWorkerConnection, String> {
    // One account lifecycle owns validation, credential lookup, worker
    // publication, and the returned receipt. Sign-out cannot remove the
    // partition midway and leave a newly published orphan worker behind.
    let _account_lifecycle = state.account_lifecycle.read().await;
    let (owner, account) = current_account(state).await.inspect_err(|_| {
        tracing::warn!(
            event = "remote_worker_connect_failed",
            stage = "account",
            "remote worker capability failed"
        );
    })?;
    let credentials = code_credentials(state, &owner).await.inspect_err(|_| {
        tracing::warn!(
            event = "remote_worker_connect_failed",
            stage = "credential",
            "remote worker capability failed"
        );
    })?;
    let spec = build_spec(input).inspect_err(|_| {
        tracing::warn!(
            event = "remote_worker_connect_failed",
            stage = "spec",
            "remote worker capability failed"
        );
    })?;
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
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        connect_remote_worker, RemoteConnectionKind, RemoteWorkerConnectInput,
        RemoteWorkerConnection,
    };
    use crate::runtime_registry::{AccountKey, CloudAccountState};
    use crate::AppState;

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

    #[tokio::test]
    #[ignore = "requires explicit CLARK_REMOTE_REGISTRY_LIVE=1 and SSH access"]
    async fn live_registry_reuses_one_account_worker_and_tears_it_down() {
        if std::env::var("CLARK_REMOTE_REGISTRY_LIVE").as_deref() != Ok("1") {
            eprintln!("set CLARK_REMOTE_REGISTRY_LIVE=1 to run the registry receipt");
            return;
        }
        let credential_root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        state
            .credentials
            .configure(credential_root.path().join("credentials"))
            .unwrap();
        let owner = "remote-registry-benchmark";
        state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: "http://localhost".into(),
                account: AccountKey::new(owner).unwrap(),
                token: zeroize::Zeroizing::new("benchmark-not-a-session-token".into()),
            }))
            .await;
        state
            .credentials
            .set_code_key(owner, "benchmark-not-a-provider-key".into())
            .await
            .unwrap();
        let host = std::env::var("CLARK_REMOTE_CPU_HOST").unwrap_or_else(|_| "nucleus".into());
        let input = RemoteWorkerConnectInput {
            host: host.clone(),
            remote_root: PathBuf::from("/tmp/clark-code-registry-smoke"),
            model: "clark-code:free".into(),
            reasoning_effort: "max".into(),
        };

        let first = connect_remote_worker(input.clone(), &state)
            .await
            .expect("first native registry connect");
        let second = connect_remote_worker(input, &state)
            .await
            .expect("warm native registry attach");
        assert!(matches!(
            first.connection_kind,
            RemoteConnectionKind::Started
        ));
        assert!(matches!(
            second.connection_kind,
            RemoteConnectionKind::Reused
        ));
        assert_eq!(first.id, second.id);
        assert_eq!(first.account_worker_count, 1);
        assert_eq!(second.account_worker_count, 1);
        assert_eq!(second.ssh_transport, "control_master");
        assert!(
            second.connect_duration_ms < 1_500,
            "warm registry attach took {} ms",
            second.connect_duration_ms
        );

        let account = AccountKey::new(owner).unwrap();
        state.runtime_registry.disconnect_account(&account).await;
        assert_eq!(state.runtime_registry.worker_count().await, 0);

        if let Ok(receipt_path) = std::env::var("CLARK_REMOTE_REGISTRY_RECEIPT") {
            let receipt = serde_json::json!({
                "schema_version": 1,
                "benchmark": "clark_desktop_remote_registry_reconnect",
                "status": "passed",
                "generated_at_ms": SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after Unix epoch")
                    .as_millis(),
                "host": host,
                "first": first,
                "second": second,
                "same_opaque_handle": true,
                "final_account_worker_count": 0,
                "credential_recorded": false,
                "model_called": false,
            });
            let path = Path::new(&receipt_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        }
    }
}
