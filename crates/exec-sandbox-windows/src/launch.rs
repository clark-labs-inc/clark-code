use exec_sandbox_protocol::{WindowsRunnerRequest, WindowsSetupMarker};

/// Native process creation boundary. Implementations must produce a primary
/// WRITE_RESTRICTED token for the attested offline SID and keep all descendants
/// inside a kill-on-close job before returning from `spawn_restricted`.
pub trait LaunchHost {
    fn verify_identity(&mut self, sid: &str) -> Result<(), String>;
    fn verify_network_denied(&mut self, sid: &str) -> Result<(), String>;
    fn spawn_restricted(
        &mut self,
        request: &WindowsRunnerRequest,
        sid: &str,
    ) -> Result<i32, String>;
}

pub fn launch(
    request: &WindowsRunnerRequest,
    marker: &WindowsSetupMarker,
    host: &mut dyn LaunchHost,
) -> Result<i32, String> {
    request.validate()?;
    marker.validate_for_policy(&request.policy)?;
    host.verify_identity(&marker.offline_identity_sid)?;
    host.verify_network_denied(&marker.offline_identity_sid)?;
    host.spawn_restricted(request, &marker.offline_identity_sid)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::PathBuf;

    use exec_sandbox_protocol::{
        WindowsNetworkEnforcement, WireNetworkPolicy, WireOsString, WireProcess, WireSandboxPolicy,
        RUNNER_PROTOCOL_VERSION, SETUP_PROTOCOL_VERSION,
    };

    use super::*;

    struct FakeHost {
        calls: Vec<&'static str>,
        fail_network: bool,
    }

    impl LaunchHost for FakeHost {
        fn verify_identity(&mut self, _sid: &str) -> Result<(), String> {
            self.calls.push("identity");
            Ok(())
        }

        fn verify_network_denied(&mut self, _sid: &str) -> Result<(), String> {
            self.calls.push("network");
            if self.fail_network {
                Err("firewall missing".into())
            } else {
                Ok(())
            }
        }

        fn spawn_restricted(
            &mut self,
            _request: &WindowsRunnerRequest,
            _sid: &str,
        ) -> Result<i32, String> {
            self.calls.push("spawn");
            Ok(17)
        }
    }

    fn fixture() -> (WindowsRunnerRequest, WindowsSetupMarker) {
        let policy = WireSandboxPolicy {
            read_roots: Vec::new(),
            write_roots: vec![PathBuf::from(r"C:\workspace")],
            deny_read: Vec::new(),
            deny_write: Vec::new(),
            network: WireNetworkPolicy::Restricted,
            process_temp_root: None,
        };
        let request = WindowsRunnerRequest {
            protocol_version: RUNNER_PROTOCOL_VERSION,
            request_id: "run-test".into(),
            state_dir: PathBuf::from(r"C:\state"),
            policy: policy.clone(),
            process: WireProcess {
                program: WireOsString::from_os(OsStr::new("cmd.exe")),
                args: Vec::new(),
                cwd: WireOsString::from_os(OsStr::new(r"C:\workspace")),
                env: Vec::new(),
            },
        };
        let marker = WindowsSetupMarker {
            setup_protocol_version: SETUP_PROTOCOL_VERSION,
            runner_protocol_version: RUNNER_PROTOCOL_VERSION,
            runner_sha256: "fixture".into(),
            offline_identity_sid: "S-1-5-21-1000".into(),
            network_enforcement: WindowsNetworkEnforcement::WindowsFilteringPlatform,
            generation: 1,
            provisioned_policy_sha256: vec![policy.fingerprint().unwrap()],
            provisioned_write_capability_sids: policy.write_capability_sids(),
        };
        (request, marker)
    }

    #[test]
    fn launch_reverifies_identity_and_network_before_spawn() {
        let (request, marker) = fixture();
        let mut host = FakeHost {
            calls: Vec::new(),
            fail_network: false,
        };
        assert_eq!(launch(&request, &marker, &mut host).unwrap(), 17);
        assert_eq!(host.calls, ["identity", "network", "spawn"]);
    }

    #[test]
    fn missing_network_enforcement_prevents_process_creation() {
        let (request, marker) = fixture();
        let mut host = FakeHost {
            calls: Vec::new(),
            fail_network: true,
        };
        assert!(launch(&request, &marker, &mut host).is_err());
        assert_eq!(host.calls, ["identity", "network"]);
    }
}
