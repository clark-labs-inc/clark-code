use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LocalSandboxStatus {
    pub state: &'static str,
    pub backend: &'static str,
    pub reason: Option<String>,
    pub setup_available: bool,
}

#[tauri::command]
pub fn local_sandbox_status(cwd: String) -> Result<LocalSandboxStatus, String> {
    sandbox_status(Path::new(&cwd))
}

#[tauri::command]
pub async fn local_sandbox_setup(cwd: String) -> Result<LocalSandboxStatus, String> {
    let cwd = PathBuf::from(cwd);
    let policy =
        provider_local::local_sandbox_setup_policy(&cwd).map_err(|error| error.to_string())?;
    let sandbox_manager = manager(policy.clone())?;
    if matches!(
        sandbox_manager.status(),
        exec_sandbox::SandboxStatus::Enforced { .. }
    ) {
        return sandbox_status(&cwd);
    }
    let action = sandbox_manager
        .setup_action()?
        .ok_or_else(|| "this sandbox backend has no setup action".to_string())?;
    let requires_elevation = action.requires_elevation;
    let first = tauri::async_runtime::spawn_blocking(move || {
        exec_sandbox_windows::run_setup_action(
            &action.program,
            &action.args,
            action.requires_elevation,
            action.cleanup_paths,
        )
    })
    .await
    .map_err(|error| format!("Windows sandbox setup task failed: {error}"))?;
    if let Err(error) = first {
        if requires_elevation {
            return Err(error);
        }
        // The user explicitly chose setup, but Windows denied user-mode
        // WRITE_DAC on this root. Rebuild ownership proofs and use the same
        // bundled helper as a narrow elevated fallback.
        let fallback = manager(policy)?
            .setup_action()?
            .ok_or_else(|| "this sandbox backend has no setup action".to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            exec_sandbox_windows::run_setup_action(
                &fallback.program,
                &fallback.args,
                true,
                fallback.cleanup_paths,
            )
        })
        .await
        .map_err(|join| format!("Windows sandbox fallback task failed: {join}"))?
        .map_err(|fallback| format!("{error}; elevated fallback failed: {fallback}"))?;
    }
    let status = sandbox_status(&cwd)?;
    if status.state != "enforced" {
        return Err(status
            .reason
            .unwrap_or_else(|| "Windows sandbox setup did not become ready".to_string()));
    }
    Ok(status)
}

fn sandbox_status(cwd: &Path) -> Result<LocalSandboxStatus, String> {
    let policy =
        provider_local::local_sandbox_setup_policy(cwd).map_err(|error| error.to_string())?;
    let manager = manager(policy)?;
    let backend = match manager.status() {
        exec_sandbox::SandboxStatus::Enforced { backend }
        | exec_sandbox::SandboxStatus::Unavailable { backend, .. }
        | exec_sandbox::SandboxStatus::SetupRequired { backend, .. } => backend_name(*backend),
    };
    Ok(match manager.status() {
        exec_sandbox::SandboxStatus::Enforced { .. } => LocalSandboxStatus {
            state: "enforced",
            backend,
            reason: None,
            setup_available: false,
        },
        exec_sandbox::SandboxStatus::Unavailable { reason, .. } => LocalSandboxStatus {
            state: "unavailable",
            backend,
            reason: Some(reason.clone()),
            setup_available: false,
        },
        exec_sandbox::SandboxStatus::SetupRequired { reason, .. } => LocalSandboxStatus {
            state: "setup_required",
            backend,
            reason: Some(reason.clone()),
            setup_available: manager.setup_available(),
        },
    })
}

fn manager(policy: exec_sandbox::SandboxPolicy) -> Result<exec_sandbox::SandboxManager, String> {
    let install = clark_install_context::InstallContext::current();
    exec_sandbox::SandboxManager::current_with_runtime(
        policy,
        exec_sandbox::SandboxRuntime {
            linux_bubblewrap: install.bundled_tool(clark_install_context::BUBBLEWRAP),
            windows_runner: install.bundled_tool(clark_install_context::WINDOWS_SANDBOX_RUNNER),
            windows_setup: install.bundled_tool(clark_install_context::WINDOWS_SANDBOX_SETUP),
            windows_state_dir: None,
        },
    )
}

pub(crate) fn release_smoke_executor(
    cwd: &Path,
) -> Result<exec_sandbox::SandboxedExecutor, String> {
    let policy =
        provider_local::local_sandbox_setup_policy(cwd).map_err(|error| error.to_string())?;
    exec_sandbox::SandboxedExecutor::with_manager(manager(policy)?)
}

fn backend_name(backend: exec_sandbox::BackendKind) -> &'static str {
    match backend {
        exec_sandbox::BackendKind::MacosSeatbelt => "macos_seatbelt",
        exec_sandbox::BackendKind::LinuxBubblewrap => "linux_bubblewrap",
        exec_sandbox::BackendKind::WindowsRestrictedToken => "windows_restricted_token",
    }
}
