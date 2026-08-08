use super::*;

fn policy() -> WireSandboxPolicy {
    WireSandboxPolicy {
        read_roots: Vec::new(),
        write_roots: vec![PathBuf::from(r"C:\workspace")],
        deny_read: Vec::new(),
        deny_write: vec![PathBuf::from(r"C:\workspace\.git")],
        network: WireNetworkPolicy::Restricted,
        process_temp_root: Some(PathBuf::from(r"C:\temp")),
    }
}

#[test]
fn runner_request_round_trip_preserves_utf16_and_policy() {
    let request = WindowsRunnerRequest {
        protocol_version: RUNNER_PROTOCOL_VERSION,
        request_id: "request-1".to_string(),
        state_dir: PathBuf::from("/agent-desktop/sandbox"),
        policy: policy(),
        process: WireProcess {
            program: WireOsString::from_os(OsStr::new("cmd.exe")),
            args: vec![WireOsString {
                utf16: vec![0xd800, 65],
            }],
            cwd: WireOsString::from_os(OsStr::new(r"C:\workspace")),
            env: Vec::new(),
        },
    };

    let encoded = encode_request(&request).unwrap();
    let decoded: WindowsRunnerRequest = decode_request(&encoded).unwrap();
    assert_eq!(decoded, request);
    assert!(decoded.validate().is_ok());
}

#[test]
fn marker_pins_runner_digest_and_protocol_versions() {
    let dir = tempfile::tempdir().unwrap();
    let runner = dir.path().join("agent-command-runner.exe");
    std::fs::write(&runner, b"signed runner fixture").unwrap();
    let marker = WindowsSetupMarker {
        setup_protocol_version: SETUP_PROTOCOL_VERSION,
        runner_protocol_version: RUNNER_PROTOCOL_VERSION,
        runner_sha256: sha256_file(&runner).unwrap(),
        offline_identity_sid: "S-1-5-21-fixture".to_string(),
        network_enforcement: WindowsNetworkEnforcement::WindowsFilteringPlatform,
        generation: 1,
        provisioned_policy_sha256: vec![policy().fingerprint().unwrap()],
        provisioned_write_capability_sids: policy().write_capability_sids(),
    };
    assert!(marker.validate_for_runner(&runner).is_ok());
    assert!(marker.validate_bootstrap(&runner).is_ok());
    assert!(marker.validate_for_policy(&policy()).is_ok());
    std::fs::write(&runner, b"replaced runner").unwrap();
    assert!(marker.validate_for_runner(&runner).is_err());
}

#[test]
fn marker_accepts_only_safe_capability_subsets_without_exact_policy_match() {
    let mut provisioned = policy();
    provisioned
        .write_roots
        .push(PathBuf::from(r"C:\AgentDesktop\sandbox-tmp"));
    let marker = WindowsSetupMarker {
        setup_protocol_version: SETUP_PROTOCOL_VERSION,
        runner_protocol_version: RUNNER_PROTOCOL_VERSION,
        runner_sha256: "fixture".into(),
        offline_identity_sid: "S-1-5-21-fixture".into(),
        network_enforcement: WindowsNetworkEnforcement::WindowsFilteringPlatform,
        generation: 1,
        provisioned_policy_sha256: vec![provisioned.fingerprint().unwrap()],
        provisioned_write_capability_sids: provisioned.write_capability_sids(),
    };

    let readonly = WireSandboxPolicy {
        read_roots: Vec::new(),
        write_roots: vec![PathBuf::from(r"C:\AgentDesktop\sandbox-tmp")],
        deny_read: Vec::new(),
        deny_write: Vec::new(),
        network: WireNetworkPolicy::Restricted,
        process_temp_root: Some(PathBuf::from(r"C:\AgentDesktop\sandbox-tmp\session-1")),
    };
    assert!(marker.validate_for_policy(&readonly).is_ok());

    let mut unprovisioned = readonly.clone();
    unprovisioned.write_roots = vec![PathBuf::from(r"C:\other")];
    assert!(marker.validate_for_policy(&unprovisioned).is_err());

    let mut new_deny_policy = readonly;
    new_deny_policy
        .deny_write
        .push(PathBuf::from(r"C:\AgentDesktop\sandbox-tmp\protected"));
    assert!(marker.validate_for_policy(&new_deny_policy).is_err());
}

#[test]
fn policy_fingerprint_collapses_redundant_session_temp_roots() {
    let mut first = policy();
    first
        .write_roots
        .push(PathBuf::from(r"C:\workspace\.agent\tmp\one"));
    first.process_temp_root = Some(PathBuf::from(r"C:\workspace\.agent\tmp\one"));
    let mut second = policy();
    second
        .write_roots
        .push(PathBuf::from(r"C:\workspace\.agent\tmp\two"));
    second.process_temp_root = Some(PathBuf::from(r"C:\workspace\.agent\tmp\two"));
    assert_eq!(first.fingerprint().unwrap(), second.fingerprint().unwrap());
}

#[test]
fn write_capabilities_are_root_scoped_and_ignore_redundant_children() {
    let mut first = policy();
    first
        .write_roots
        .push(PathBuf::from(r"C:\workspace\nested"));
    let mut second = policy();
    second.write_roots = vec![PathBuf::from(r"C:\other")];
    assert_eq!(first.write_capability_sids().len(), 2);
    assert_ne!(
        first.write_capability_sids()[1],
        second.write_capability_sids()[1]
    );
    assert!(first.write_capability_sids()[0].starts_with("S-1-5-21-"));
    assert_eq!(
        first.write_capability_sids()[0],
        WireSandboxPolicy::device_capability_sid()
    );
}

#[test]
fn readonly_policy_gets_only_the_device_capability() {
    let readonly = WireSandboxPolicy {
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        deny_read: Vec::new(),
        deny_write: Vec::new(),
        network: WireNetworkPolicy::Restricted,
        process_temp_root: None,
    };
    assert_eq!(
        readonly.write_capability_sids(),
        [WireSandboxPolicy::device_capability_sid()]
    );
}

#[test]
fn validation_rejects_relative_privilege_boundary_paths() {
    let request = WindowsSetupRequest {
        protocol_version: SETUP_PROTOCOL_VERSION,
        request_id: "request-1".to_string(),
        state_dir: PathBuf::from("relative"),
        runner_path: PathBuf::from(r"C:\AgentDesktop\agent-command-runner.exe"),
        policy: policy(),
        root_proofs: vec![WindowsRootProof {
            root: PathBuf::from(r"C:\workspace"),
            proof_path: PathBuf::from(r"C:\workspace\.agent-sandbox-setup-request-1-0.proof"),
            nonce: "0123456789abcdef0123456789abcdef".to_string(),
        }],
    };
    assert!(request.validate().is_err());
}

#[test]
fn windows_requests_reject_unenforceable_read_or_network_policies() {
    let mut request = WindowsRunnerRequest {
        protocol_version: RUNNER_PROTOCOL_VERSION,
        request_id: "request-policy".to_string(),
        state_dir: PathBuf::from(r"C:\AgentDesktop\sandbox"),
        policy: policy(),
        process: WireProcess {
            program: WireOsString::from_os(OsStr::new("cmd.exe")),
            args: Vec::new(),
            cwd: WireOsString::from_os(OsStr::new(r"C:\workspace")),
            env: Vec::new(),
        },
    };
    request.policy.deny_read = vec![PathBuf::from(r"C:\secret")];
    assert!(request.validate().is_err());
    request.policy.deny_read.clear();
    request.policy.network = WireNetworkPolicy::Enabled;
    assert!(request.validate().is_err());
}

#[test]
fn runner_request_rejects_embedded_nuls_and_environment_injection() {
    let mut request = WindowsRunnerRequest {
        protocol_version: RUNNER_PROTOCOL_VERSION,
        request_id: "request-strings".to_string(),
        state_dir: PathBuf::from(r"C:\AgentDesktop\sandbox"),
        policy: policy(),
        process: WireProcess {
            program: WireOsString::from_os(OsStr::new("cmd.exe")),
            args: vec![WireOsString {
                utf16: vec![65, 0, 66],
            }],
            cwd: WireOsString::from_os(OsStr::new(r"C:\workspace")),
            env: Vec::new(),
        },
    };
    assert!(request.validate().is_err());
    request.process.args.clear();
    request.process.env.push((
        WireOsString::from_os(OsStr::new("BAD=NAME")),
        WireOsString::from_os(OsStr::new("value")),
    ));
    assert!(request.validate().is_err());
}

#[test]
fn decoder_rejects_oversized_input_before_base64_allocation() {
    let oversized = "A".repeat(MAX_ENCODED_REQUEST_CHARS + 1);
    assert!(decode_request::<WindowsRunnerRequest>(&oversized).is_err());
}

#[test]
fn encoder_rejects_requests_that_cannot_fit_a_windows_command_line() {
    let mut request = WindowsRunnerRequest {
        protocol_version: RUNNER_PROTOCOL_VERSION,
        request_id: "request-oversized".to_string(),
        state_dir: PathBuf::from(r"C:\AgentDesktop\sandbox"),
        policy: policy(),
        process: WireProcess {
            program: WireOsString::from_os(OsStr::new("powershell.exe")),
            args: Vec::new(),
            cwd: WireOsString::from_os(OsStr::new(r"C:\workspace")),
            env: Vec::new(),
        },
    };
    request.process.args.push(WireOsString {
        utf16: vec![b'x' as u16; MAX_REQUEST_BYTES],
    });
    assert!(encode_request(&request).is_err());
}
