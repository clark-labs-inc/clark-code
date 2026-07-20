use exec_sandbox_protocol::{
    sha256_file, WindowsNetworkEnforcement, WindowsSetupMarker, WindowsSetupRequest,
    WireNetworkPolicy, WireSandboxPolicy, RUNNER_PROTOCOL_VERSION, SETUP_PROTOCOL_VERSION,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvisionedIdentity {
    pub sid: String,
}

/// Narrow host operations needed by the one-time elevated bootstrap. The
/// coordinator owns ordering and attestation; Windows API details stay behind
/// this boundary and can be replaced or simulated independently.
pub trait ProvisioningHost {
    fn validate_bootstrap(&mut self, request: &WindowsSetupRequest) -> Result<(), String>;
    fn ensure_offline_identity(&mut self) -> Result<ProvisionedIdentity, String>;
    fn ensure_network_denied(&mut self, identity: &ProvisionedIdentity) -> Result<(), String>;
    fn reconcile_global_objects(&mut self) -> Result<(), String>;
    fn existing_marker(&mut self) -> Option<WindowsSetupMarker>;
    fn commit_marker(&mut self, marker: &WindowsSetupMarker) -> Result<(), String>;
}

/// User-mode operations for adding one policy after bootstrap. Implementations
/// must obtain WRITE_DAC through the caller's existing ownership rather than
/// assuming administrator authority.
pub trait EnrollmentHost {
    fn validate_enrollment(&mut self, request: &WindowsSetupRequest) -> Result<(), String>;
    fn existing_marker(&mut self) -> Option<WindowsSetupMarker>;
    fn verify_identity(&mut self, identity: &ProvisionedIdentity) -> Result<(), String>;
    fn verify_network_denied(&mut self, identity: &ProvisionedIdentity) -> Result<(), String>;
    fn reconcile_workspace_acl(
        &mut self,
        request: &WindowsSetupRequest,
        identity: &ProvisionedIdentity,
    ) -> Result<(), String>;
    fn commit_marker(&mut self, marker: &WindowsSetupMarker) -> Result<(), String>;
}

pub fn provision(
    request: &WindowsSetupRequest,
    host: &mut dyn ProvisioningHost,
) -> Result<WindowsSetupMarker, String> {
    request.validate()?;
    if request.policy.network != WireNetworkPolicy::Restricted {
        return Err("Windows sandbox setup only provisions network-restricted policies".into());
    }

    host.validate_bootstrap(request)?;
    let identity = host.ensure_offline_identity()?;
    validate_sid(&identity.sid)?;

    // Network denial precedes every filesystem grant. A failed or interrupted
    // setup may leave an unusable identity, but never a write-capable online
    // sandbox identity.
    host.ensure_network_denied(&identity)?;
    host.reconcile_global_objects()?;

    let existing = host.existing_marker();
    let generation = existing
        .as_ref()
        .map(|marker| marker.generation)
        .unwrap_or(0);
    let mut provisioned_policy_sha256 = existing
        .as_ref()
        .filter(|marker| marker.offline_identity_sid == identity.sid)
        .map(|marker| marker.provisioned_policy_sha256.clone())
        .unwrap_or_default();
    provisioned_policy_sha256.sort();
    provisioned_policy_sha256.dedup();
    let mut provisioned_write_capability_sids = existing
        .filter(|marker| marker.offline_identity_sid == identity.sid)
        .map(|marker| marker.provisioned_write_capability_sids)
        .unwrap_or_default();
    provisioned_write_capability_sids.push(WireSandboxPolicy::device_capability_sid());
    provisioned_write_capability_sids.sort_by_key(|sid| sid.to_ascii_lowercase());
    provisioned_write_capability_sids.dedup_by(|left, right| left.eq_ignore_ascii_case(right));

    let marker = WindowsSetupMarker {
        setup_protocol_version: SETUP_PROTOCOL_VERSION,
        runner_protocol_version: RUNNER_PROTOCOL_VERSION,
        runner_sha256: sha256_file(&request.runner_path)?,
        offline_identity_sid: identity.sid,
        network_enforcement: WindowsNetworkEnforcement::WindowsFilteringPlatform,
        generation: generation.saturating_add(1).max(1),
        provisioned_policy_sha256,
        provisioned_write_capability_sids,
    };
    host.commit_marker(&marker)?;
    Ok(marker)
}

pub fn enroll(
    request: &WindowsSetupRequest,
    host: &mut dyn EnrollmentHost,
) -> Result<WindowsSetupMarker, String> {
    request.validate()?;
    if request.policy.network != WireNetworkPolicy::Restricted {
        return Err("Windows sandbox enrollment requires restricted networking".into());
    }
    host.validate_enrollment(request)?;
    let mut marker = host
        .existing_marker()
        .ok_or_else(|| "Windows sandbox bootstrap is missing".to_string())?;
    marker.validate_bootstrap(&request.runner_path)?;
    let identity = ProvisionedIdentity {
        sid: marker.offline_identity_sid.clone(),
    };
    validate_sid(&identity.sid)?;
    host.verify_identity(&identity)?;
    host.verify_network_denied(&identity)?;
    host.reconcile_workspace_acl(request, &identity)?;

    let policy_fingerprint = request.policy.fingerprint()?;
    if !marker
        .provisioned_policy_sha256
        .iter()
        .any(|value| value.eq_ignore_ascii_case(&policy_fingerprint))
    {
        marker.provisioned_policy_sha256.push(policy_fingerprint);
    }
    marker.provisioned_policy_sha256.sort();
    marker.provisioned_policy_sha256.dedup();
    marker
        .provisioned_write_capability_sids
        .extend(request.policy.write_capability_sids());
    marker
        .provisioned_write_capability_sids
        .sort_by_key(|sid| sid.to_ascii_lowercase());
    marker
        .provisioned_write_capability_sids
        .dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    marker.generation = marker.generation.saturating_add(1).max(1);
    host.commit_marker(&marker)?;
    Ok(marker)
}

fn validate_sid(sid: &str) -> Result<(), String> {
    let parts = sid.split('-').collect::<Vec<_>>();
    if parts.len() < 4
        || parts[0] != "S"
        || parts[1] != "1"
        || parts[2..].iter().any(|part| part.parse::<u64>().is_err())
    {
        return Err("Windows sandbox identity returned an invalid SID".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use exec_sandbox_protocol::{
        WindowsRootProof, WindowsSetupRequest, WireNetworkPolicy, WireSandboxPolicy,
        SETUP_PROTOCOL_VERSION,
    };

    use super::*;

    #[derive(Default)]
    struct FakeHost {
        calls: Vec<&'static str>,
        fail_at: Option<&'static str>,
        marker: Option<WindowsSetupMarker>,
    }

    impl FakeHost {
        fn step(&mut self, name: &'static str) -> Result<(), String> {
            self.calls.push(name);
            if self.fail_at == Some(name) {
                Err(format!("failed at {name}"))
            } else {
                Ok(())
            }
        }
    }

    impl ProvisioningHost for FakeHost {
        fn validate_bootstrap(&mut self, _request: &WindowsSetupRequest) -> Result<(), String> {
            self.step("bootstrap_validate")
        }

        fn ensure_offline_identity(&mut self) -> Result<ProvisionedIdentity, String> {
            self.step("identity")?;
            Ok(ProvisionedIdentity {
                sid: "S-1-5-21-1000".to_string(),
            })
        }

        fn ensure_network_denied(&mut self, _identity: &ProvisionedIdentity) -> Result<(), String> {
            self.step("network")
        }

        fn reconcile_global_objects(&mut self) -> Result<(), String> {
            self.step("global")
        }

        fn existing_marker(&mut self) -> Option<WindowsSetupMarker> {
            self.marker.clone()
        }

        fn commit_marker(&mut self, marker: &WindowsSetupMarker) -> Result<(), String> {
            self.step("marker")?;
            self.marker = Some(marker.clone());
            Ok(())
        }
    }

    impl EnrollmentHost for FakeHost {
        fn validate_enrollment(&mut self, _request: &WindowsSetupRequest) -> Result<(), String> {
            self.step("enrollment_validate")
        }

        fn existing_marker(&mut self) -> Option<WindowsSetupMarker> {
            self.marker.clone()
        }

        fn verify_identity(&mut self, _identity: &ProvisionedIdentity) -> Result<(), String> {
            self.step("verify_identity")
        }

        fn verify_network_denied(&mut self, _identity: &ProvisionedIdentity) -> Result<(), String> {
            self.step("verify_network")
        }

        fn reconcile_workspace_acl(
            &mut self,
            _request: &WindowsSetupRequest,
            _identity: &ProvisionedIdentity,
        ) -> Result<(), String> {
            self.step("workspace_acl")
        }

        fn commit_marker(&mut self, marker: &WindowsSetupMarker) -> Result<(), String> {
            self.step("marker")?;
            self.marker = Some(marker.clone());
            Ok(())
        }
    }

    fn request(runner: &Path) -> WindowsSetupRequest {
        let root = PathBuf::from(r"C:\workspace");
        WindowsSetupRequest {
            protocol_version: SETUP_PROTOCOL_VERSION,
            request_id: "setup-test".into(),
            state_dir: runner.parent().unwrap().to_path_buf(),
            runner_path: runner.to_path_buf(),
            policy: WireSandboxPolicy {
                read_roots: Vec::new(),
                write_roots: vec![root.clone()],
                deny_read: Vec::new(),
                deny_write: vec![PathBuf::from(r"C:\workspace\.git")],
                network: WireNetworkPolicy::Restricted,
                process_temp_root: Some(PathBuf::from(r"C:\workspace\.tmp")),
            },
            root_proofs: vec![WindowsRootProof {
                root,
                proof_path: PathBuf::from(r"C:\workspace\.clark-sandbox-setup-setup-test-0.proof"),
                nonce: "0123456789abcdef0123456789abcdef".into(),
            }],
        }
    }

    #[test]
    fn bootstrap_is_fail_closed_and_attestation_is_last() {
        let temp = tempfile::tempdir().unwrap();
        let runner = temp.path().join("clark-command-runner.exe");
        std::fs::write(&runner, b"runner fixture").unwrap();
        let mut host = FakeHost::default();
        let marker = provision(&request(&runner), &mut host).unwrap();
        assert_eq!(
            host.calls,
            [
                "bootstrap_validate",
                "identity",
                "network",
                "global",
                "marker"
            ]
        );
        assert_eq!(marker.generation, 1);
        assert_eq!(marker.offline_identity_sid, "S-1-5-21-1000");
        assert!(marker.provisioned_policy_sha256.is_empty());
        assert_eq!(marker.provisioned_write_capability_sids.len(), 1);
    }

    #[test]
    fn acl_is_never_granted_when_network_enforcement_fails() {
        let temp = tempfile::tempdir().unwrap();
        let runner = temp.path().join("clark-command-runner.exe");
        std::fs::write(&runner, b"runner fixture").unwrap();
        let mut host = FakeHost {
            fail_at: Some("network"),
            ..FakeHost::default()
        };
        assert!(provision(&request(&runner), &mut host).is_err());
        assert_eq!(host.calls, ["bootstrap_validate", "identity", "network"]);
        assert!(host.marker.is_none());
    }

    #[test]
    fn user_mode_enrollment_accumulates_policy_attestations() {
        let temp = tempfile::tempdir().unwrap();
        let runner = temp.path().join("clark-command-runner.exe");
        std::fs::write(&runner, b"runner fixture").unwrap();
        let mut host = FakeHost::default();
        provision(&request(&runner), &mut host).unwrap();
        host.calls.clear();
        let marker = enroll(&request(&runner), &mut host).unwrap();
        assert_eq!(
            host.calls,
            [
                "enrollment_validate",
                "verify_identity",
                "verify_network",
                "workspace_acl",
                "marker"
            ]
        );
        assert_eq!(marker.provisioned_policy_sha256.len(), 1);
        host.calls.clear();
        let mut second = request(&runner);
        second.policy.write_roots = vec![PathBuf::from(r"C:\other-workspace")];
        second.root_proofs[0].root = PathBuf::from(r"C:\other-workspace");
        second.root_proofs[0].proof_path =
            PathBuf::from(r"C:\other-workspace\.clark-sandbox-setup-setup-test-0.proof");
        let marker = enroll(&second, &mut host).unwrap();
        assert_eq!(marker.generation, 3);
        assert_eq!(marker.provisioned_policy_sha256.len(), 2);
        assert_eq!(marker.provisioned_write_capability_sids.len(), 3);
    }
}
