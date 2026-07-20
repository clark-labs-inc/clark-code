mod backend;
mod executor;
mod policy;

pub use backend::{
    BackendKind, SandboxBackend, SandboxManager, SandboxRuntime, SandboxSetupAction, SandboxStatus,
};
pub use executor::SandboxedExecutor;
pub use policy::{NetworkPolicy, SandboxPolicy, SandboxPreset};

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use exec_core::ProcessSpec;

    use super::*;

    fn fixture_policy() -> SandboxPolicy {
        SandboxPolicy::workspace_write(fixture_workspace(), Vec::new())
    }

    fn fixture_workspace() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\workspace")
        } else {
            PathBuf::from("/workspace")
        }
    }

    #[test]
    fn every_backend_preserves_the_inner_command() {
        let original =
            ProcessSpec::argv("/bin/sh", fixture_workspace()).args(["-c", "printf sandbox-ok"]);
        for backend in [
            BackendKind::MacosSeatbelt,
            BackendKind::LinuxBubblewrap,
            BackendKind::WindowsRestrictedToken,
        ] {
            let manager = SandboxManager::simulate(
                fixture_policy(),
                backend,
                PathBuf::from(match backend {
                    BackendKind::MacosSeatbelt => "/usr/bin/sandbox-exec",
                    BackendKind::LinuxBubblewrap => "/usr/bin/bwrap",
                    BackendKind::WindowsRestrictedToken => "clark-command-runner.exe",
                }),
            );
            let prepared = manager.prepare_process(original.clone()).unwrap();
            if backend == BackendKind::WindowsRestrictedToken {
                assert_eq!(prepared.args[0], "--request-b64");
                let request: exec_sandbox_protocol::WindowsRunnerRequest =
                    exec_sandbox_protocol::decode_request(
                        prepared.args[1].to_string_lossy().as_ref(),
                    )
                    .unwrap();
                assert_eq!(request.process.program.to_os_string(), "/bin/sh");
                assert!(request
                    .process
                    .args
                    .iter()
                    .any(|arg| arg.to_os_string() == "printf sandbox-ok"));
                continue;
            }
            let rendered = prepared
                .args
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>();
            assert!(rendered.iter().any(|arg| arg == "/bin/sh"));
            assert!(rendered.iter().any(|arg| arg == "printf sandbox-ok"));
        }
    }

    #[test]
    fn windows_compiler_rejects_read_constraints_it_cannot_enforce() {
        let mut policy = fixture_policy();
        policy.deny_read.push(PathBuf::from("/host/secret"));
        let manager = SandboxManager::simulate(
            policy,
            BackendKind::WindowsRestrictedToken,
            PathBuf::from("clark-command-runner.exe"),
        );
        let process = ProcessSpec::argv("cmd.exe", fixture_workspace());
        assert!(manager.prepare_process(process).is_err());
    }
}
