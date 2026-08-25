use super::*;

#[cfg(windows)]
fn windows_sandbox_data_root() -> Result<std::path::PathBuf> {
    exec_sandbox::windows_product_data_root()
        .ok_or_else(|| Error::Io("LOCALAPPDATA is unavailable".to_string()))
}

pub(super) fn build_local_executor(
    config: &LocalConfig,
    sandbox: &mut Sandbox,
    preset: exec_sandbox::SandboxPreset,
) -> Result<(Arc<dyn Executor>, Option<tempfile::TempDir>)> {
    // Keep the file-tool trust flag exactly in sync with process containment:
    // Full Access / disabled sandbox lifts both; any contained preset restores
    // both, even on a sandbox cloned from a previously trusted session.
    sandbox.host_trusted = explicit_host_execution_allowed(config.sandbox_mode, preset);
    if sandbox.host_trusted {
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
        let base = windows_sandbox_data_root()?.join("sandbox-tmp");
        std::fs::create_dir_all(&base).map_err(|error| Error::Io(error.to_string()))?;
        extra_write_roots.push(base.clone());
        tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(base)
            .map_err(|error| Error::Io(error.to_string()))?
    };
    #[cfg(not(windows))]
    let private_temp = tempfile::Builder::new()
        .prefix("agent-sandbox-")
        .tempdir()
        .map_err(|error| Error::Io(error.to_string()))?;
    let policy = match preset {
        exec_sandbox::SandboxPreset::ReadOnly => {
            exec_sandbox::SandboxPolicy::read_only().with_write_roots(extra_write_roots)
        }
        exec_sandbox::SandboxPreset::WorkspaceWrite => {
            // Project-approved external write targets (shared machine caches,
            // typically reached through an in-project symlink such as Cargo's
            // `target/`) join the workspace here. Plan Mode's ReadOnly preset
            // never receives them.
            extra_write_roots.extend(config.sandbox_write_roots.iter().cloned());
            exec_sandbox::SandboxPolicy::workspace_write(
                sandbox.root().to_path_buf(),
                extra_write_roots,
            )
        }
        exec_sandbox::SandboxPreset::DangerFullAccess => unreachable!(),
    }
    .with_process_temp_root(private_temp.path().to_path_buf());
    let install = desktop_install_context::InstallContext::current();
    let runtime = exec_sandbox::SandboxRuntime {
        linux_bubblewrap: install.bundled_tool(desktop_install_context::BUBBLEWRAP),
        windows_runner: install.bundled_tool(desktop_install_context::WINDOWS_SANDBOX_RUNNER),
        windows_setup: install.bundled_tool(desktop_install_context::WINDOWS_SANDBOX_SETUP),
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
    Err(Error::Unsupported(format!(
        "project sandbox is not ready: {:?}. Set sandbox mode to disabled or choose danger-full-access to run on the host.",
        manager.status()
    )))
}

fn explicit_host_execution_allowed(
    mode: crate::config::LocalSandboxMode,
    preset: exec_sandbox::SandboxPreset,
) -> bool {
    crate::sandbox::preset_runs_on_bare_host(mode, preset)
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

/// Stable policy used by the desktop's explicit sandbox-setup flow. Session
/// directories nest under these roots, so one consented ACL reconciliation is
/// reusable without broadening access beyond Clark Code's project/docs/temp
/// areas plus the project's own `sandbox_write_roots` grants — keeping the
/// enrolled perimeter identical to what WorkspaceWrite sessions will enforce.
pub async fn local_sandbox_setup_policy(
    cwd: &std::path::Path,
) -> Result<exec_sandbox::SandboxPolicy> {
    let project = crate::project_settings::load(&LocalExecutor, cwd).await;
    let (granted, rejected) =
        crate::project_settings::validated_write_roots(&project.sandbox_write_roots);
    for entry in rejected {
        tracing::warn!(entry, "ignoring non-absolute sandbox_write_roots entry");
    }
    #[cfg(not(windows))]
    let mut write_roots = Vec::new();
    #[cfg(windows)]
    let mut write_roots = Vec::new();
    #[cfg(windows)]
    {
        if let Some(docs_root) = crate::workspace::workspace_root() {
            write_roots.push(docs_root);
        }
        let temp_root = windows_sandbox_data_root()?.join("sandbox-tmp");
        write_roots.push(temp_root);
    }
    write_roots.extend(granted);
    Ok(exec_sandbox::SandboxPolicy::workspace_write(
        cwd.to_path_buf(),
        write_roots,
    ))
}

#[cfg(test)]
mod tests {
    use super::explicit_host_execution_allowed;
    use crate::config::LocalSandboxMode;
    use exec_sandbox::SandboxPreset;

    #[test]
    fn host_execution_requires_an_explicit_uncontained_mode() {
        assert!(!explicit_host_execution_allowed(
            LocalSandboxMode::Auto,
            SandboxPreset::WorkspaceWrite
        ));
        assert!(!explicit_host_execution_allowed(
            LocalSandboxMode::Required,
            SandboxPreset::WorkspaceWrite
        ));
        assert!(explicit_host_execution_allowed(
            LocalSandboxMode::Disabled,
            SandboxPreset::WorkspaceWrite
        ));
        assert!(explicit_host_execution_allowed(
            LocalSandboxMode::Auto,
            SandboxPreset::DangerFullAccess
        ));
    }

    #[tokio::test]
    async fn setup_policy_enrolls_the_same_grants_sessions_will_use() {
        let project = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let absolute_root = cache.path().canonicalize().unwrap();
        std::fs::create_dir_all(project.path().join(".agent")).unwrap();
        std::fs::write(
            project.path().join(".agent/settings.json"),
            serde_json::json!({
                "sandbox_write_roots": [
                    absolute_root.to_string_lossy(),
                    "relative/escape"
                ]
            })
            .to_string(),
        )
        .unwrap();

        let policy = super::local_sandbox_setup_policy(project.path())
            .await
            .unwrap();
        assert!(policy.write_roots.contains(&absolute_root));
        // Exactly the checkout plus the one absolute grant survived; the
        // relative entry was refused rather than enrolled.
        assert_eq!(policy.write_roots.len(), 2);
    }
}
