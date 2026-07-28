use computer_use::{
    default_approval_store, ActionReceipt, ApprovalSnapshot, PermissionRequest, PermissionStatus,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ComputerUsePermissionOwner {
    pub display_name: &'static str,
    pub bundle_id: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ComputerUsePlatformStatus {
    pub supported: bool,
    pub platform: &'static str,
    pub service_ready: bool,
    pub readiness: &'static str,
    pub permission_owner: Option<ComputerUsePermissionOwner>,
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

/// Inspect native computer-use readiness without changing platform privacy
/// state. A missing or rejected service remains visible instead of being
/// collapsed into a generic unsupported-platform state.
#[tauri::command]
pub async fn computer_use_platform_status() -> ComputerUsePlatformStatus {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
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
                service_ready: true,
                readiness: if permissions.screen_recording_restart_required {
                    "restart_required"
                } else if permissions.accessibility && permissions.screen_recording {
                    "ready"
                } else {
                    "needs_permission"
                },
                permission_owner: Some(permission_owner()),
                permissions: Some(permissions),
                detail: None,
            },
            Err(error) => ComputerUsePlatformStatus {
                supported: true,
                platform: std::env::consts::OS,
                service_ready: false,
                readiness: "service_unavailable",
                permission_owner: Some(permission_owner()),
                permissions: None,
                detail: Some(error),
            },
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    ComputerUsePlatformStatus {
        supported: false,
        platform: std::env::consts::OS,
        service_ready: false,
        readiness: "unsupported",
        permission_owner: None,
        permissions: None,
        detail: Some("native computer use is unavailable on this platform".to_string()),
    }
}

fn permission_owner() -> ComputerUsePermissionOwner {
    #[cfg(target_os = "macos")]
    if cfg!(debug_assertions) {
        return ComputerUsePermissionOwner {
            display_name: "Clark Computer Use Dev",
            bundle_id: "com.clark.computer-use.dev",
        };
    } else {
        return ComputerUsePermissionOwner {
            display_name: "Clark Computer Use",
            bundle_id: "com.clark.computer-use",
        };
    }
    #[cfg(target_os = "windows")]
    {
        ComputerUsePermissionOwner {
            display_name: "Clark Computer Use Service",
            bundle_id: "clark-computer-use-helper.exe",
        }
    }
    #[cfg(target_os = "linux")]
    {
        ComputerUsePermissionOwner {
            display_name: "Clark Computer Use Service",
            bundle_id: "com.clark.ComputerUse",
        }
    }
}

/// Trigger the explicit platform privacy setup flow. The frontend exposes this
/// only behind a user click; merely opening Settings calls the preflight above.
#[tauri::command]
pub async fn computer_use_request_permissions() -> Result<PermissionStatus, String> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
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

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err("native computer use is unavailable on this platform".to_string())
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
