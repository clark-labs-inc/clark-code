use computer_use::{
    default_approval_store, ActionReceipt, ApprovalSnapshot, PermissionRequest, PermissionStatus,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ComputerUsePlatformStatus {
    pub supported: bool,
    pub platform: &'static str,
    pub helper_ready: bool,
    pub permissions: Option<PermissionStatus>,
    pub detail: Option<String>,
}

async fn blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| format!("computer-use task failed: {error}"))?
}

/// Inspect native computer-use readiness without changing either macOS privacy
/// permission. A missing or rejected helper remains visible instead of being
/// collapsed into a generic unsupported-platform state.
#[tauri::command]
pub async fn computer_use_platform_status() -> ComputerUsePlatformStatus {
    #[cfg(target_os = "macos")]
    {
        match blocking(|| {
            let backend = computer_use::native_backend().map_err(|error| error.to_string())?;
            backend.permissions().map_err(|error| error.to_string())
        })
        .await
        {
            Ok(permissions) => ComputerUsePlatformStatus {
                supported: true,
                platform: std::env::consts::OS,
                helper_ready: true,
                permissions: Some(permissions),
                detail: None,
            },
            Err(error) => ComputerUsePlatformStatus {
                supported: true,
                platform: std::env::consts::OS,
                helper_ready: false,
                permissions: None,
                detail: Some(error),
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    ComputerUsePlatformStatus {
        supported: false,
        platform: std::env::consts::OS,
        helper_ready: false,
        permissions: None,
        detail: Some("native computer use is currently available only on macOS".to_string()),
    }
}

/// Trigger the explicit macOS privacy setup flow. The frontend exposes this
/// only behind a user click; merely opening Settings calls the preflight above.
#[tauri::command]
pub async fn computer_use_request_permissions() -> Result<PermissionStatus, String> {
    #[cfg(target_os = "macos")]
    {
        blocking(|| {
            let backend = computer_use::native_backend().map_err(|error| error.to_string())?;
            backend
                .request_permissions(PermissionRequest {
                    accessibility: true,
                    screen_recording: true,
                })
                .map_err(|error| error.to_string())
        })
        .await
    }

    #[cfg(not(target_os = "macos"))]
    Err("native computer use is currently available only on macOS".to_string())
}

#[tauri::command]
pub async fn computer_use_approval_snapshot() -> Result<ApprovalSnapshot, String> {
    blocking(|| {
        default_approval_store()
            .and_then(|store| store.snapshot())
            .map_err(|error| error.to_string())
    })
    .await
}

/// Revoke one signer-bound application grant and return the post-revocation
/// snapshot. The store's exclusive lock means this does not acknowledge until
/// every earlier action lease has quiesced.
#[tauri::command]
pub async fn computer_use_revoke_approval(
    identity_key: String,
) -> Result<ApprovalSnapshot, String> {
    blocking(move || {
        let store = default_approval_store().map_err(|error| error.to_string())?;
        store
            .revoke(&identity_key)
            .map_err(|error| error.to_string())?;
        store.snapshot().map_err(|error| error.to_string())
    })
    .await
}

/// Revoke every durable application grant, with the same immediate-revocation
/// ordering guarantee as the single-grant command.
#[tauri::command]
pub async fn computer_use_revoke_all_approvals() -> Result<ApprovalSnapshot, String> {
    blocking(|| {
        let store = default_approval_store().map_err(|error| error.to_string())?;
        store.revoke_all().map_err(|error| error.to_string())?;
        store.snapshot().map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn computer_use_recent_receipts() -> Result<Vec<ActionReceipt>, String> {
    blocking(|| {
        default_approval_store()
            .and_then(|store| store.recent_receipts())
            .map_err(|error| error.to_string())
    })
    .await
}
