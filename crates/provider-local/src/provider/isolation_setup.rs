use super::*;

pub(super) fn build_local_executor(
    config: &LocalConfig,
    sandbox: &Sandbox,
    preset: exec_sandbox::SandboxPreset,
) -> Result<(Arc<dyn Executor>, Option<tempfile::TempDir>)> {
    if config.sandbox_mode == crate::config::LocalSandboxMode::Disabled
        || preset == exec_sandbox::SandboxPreset::DangerFullAccess
    {
        return Ok((Arc::new(LocalExecutor), None));
    }

    let mut extra_write_roots = Vec::new();
    if let Some(docs) = sandbox.docs_root() {
        extra_write_roots.push(docs.to_path_buf());
    }
    #[cfg(windows)]
    if let Some(docs_root) = crate::workspace::workspace_root() {
        extra_write_roots.push(docs_root);
    }
    #[cfg(windows)]
    let private_temp = {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| Error::Io("LOCALAPPDATA is unavailable".to_string()))?
            .join("Clark Code")
            .join("sandbox-tmp");
        std::fs::create_dir_all(&base).map_err(|error| Error::Io(error.to_string()))?;
        extra_write_roots.push(base.clone());
        tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(base)
            .map_err(|error| Error::Io(error.to_string()))?
    };
    #[cfg(not(windows))]
    let private_temp = tempfile::Builder::new()
        .prefix("clark-sandbox-")
        .tempdir()
        .map_err(|error| Error::Io(error.to_string()))?;
    let policy = match preset {
        exec_sandbox::SandboxPreset::ReadOnly => {
            exec_sandbox::SandboxPolicy::read_only().with_write_roots(extra_write_roots)
        }
        exec_sandbox::SandboxPreset::WorkspaceWrite => {
            exec_sandbox::SandboxPolicy::workspace_write(
                sandbox.root().to_path_buf(),
                extra_write_roots,
            )
        }
        exec_sandbox::SandboxPreset::DangerFullAccess => unreachable!(),
    }
    .with_process_temp_root(private_temp.path().to_path_buf());
    let install = clark_install_context::InstallContext::current();
    let runtime = exec_sandbox::SandboxRuntime {
        linux_bubblewrap: install.bundled_tool(clark_install_context::BUBBLEWRAP),
        windows_runner: install.bundled_tool(clark_install_context::WINDOWS_SANDBOX_RUNNER),
        windows_setup: install.bundled_tool(clark_install_context::WINDOWS_SANDBOX_SETUP),
        windows_state_dir: None,
    };
    let manager =
        exec_sandbox::SandboxManager::current_with_runtime(policy.clone(), runtime.clone())
            .map_err(Error::Other)?;
    #[cfg(windows)]
    let manager = auto_enroll_windows_workspace(manager, policy, runtime)?;
    if matches!(
        manager.status(),
        exec_sandbox::SandboxStatus::Enforced { .. }
    ) {
        let executor =
            Arc::new(exec_sandbox::SandboxedExecutor::with_manager(manager).map_err(Error::Other)?);
        return Ok((executor, Some(private_temp)));
    }
    if config.sandbox_mode == crate::config::LocalSandboxMode::Required {
        return Err(Error::Unsupported(format!(
            "required local sandbox is not ready: {:?}",
            manager.status()
        )));
    }
    tracing::warn!(status = ?manager.status(), "local sandbox is not ready; using explicit host execution");
    Ok((Arc::new(LocalExecutor), None))
}

#[cfg(windows)]
fn auto_enroll_windows_workspace(
    manager: exec_sandbox::SandboxManager,
    policy: exec_sandbox::SandboxPolicy,
    runtime: exec_sandbox::SandboxRuntime,
) -> Result<exec_sandbox::SandboxManager> {
    if !matches!(
        manager.status(),
        exec_sandbox::SandboxStatus::SetupRequired { .. }
    ) {
        return Ok(manager);
    }
    let Some(action) = manager.setup_action().map_err(Error::Other)? else {
        return Ok(manager);
    };
    if action.requires_elevation {
        for path in action.cleanup_paths {
            let _ = std::fs::remove_file(path);
        }
        return Ok(manager);
    }
    match exec_sandbox_windows::run_setup_action(
        &action.program,
        &action.args,
        false,
        action.cleanup_paths,
    ) {
        Ok(()) => exec_sandbox::SandboxManager::current_with_runtime(policy, runtime)
            .map_err(Error::Other),
        Err(error) => {
            tracing::warn!(
                error,
                "automatic user-mode Windows workspace enrollment failed"
            );
            Ok(manager)
        }
    }
}

/// Stable policy used by the desktop's explicit Windows setup flow. Session
/// directories nest under these roots, so one consented ACL reconciliation is
/// reusable without broadening access beyond Clark's project/docs/temp areas.
pub fn local_sandbox_setup_policy(cwd: &std::path::Path) -> Result<exec_sandbox::SandboxPolicy> {
    #[cfg(not(windows))]
    let write_roots = Vec::new();
    #[cfg(windows)]
    let mut write_roots = Vec::new();
    #[cfg(windows)]
    {
        if let Some(docs_root) = crate::workspace::workspace_root() {
            write_roots.push(docs_root);
        }
        let temp_root = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| Error::Io("LOCALAPPDATA is unavailable".to_string()))?
            .join("Clark Code")
            .join("sandbox-tmp");
        write_roots.push(temp_root);
    }
    Ok(exec_sandbox::SandboxPolicy::workspace_write(
        cwd.to_path_buf(),
        write_roots,
    ))
}
