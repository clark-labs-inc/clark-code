use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use exec_core::ProcessSpec;
use exec_sandbox_protocol::{
    encode_request, WindowsRootProof, WindowsRunnerRequest, WindowsSetupRequest, WireNetworkPolicy,
    WireOsString, WireProcess, WireSandboxPolicy, RUNNER_PROTOCOL_VERSION, SETUP_PROTOCOL_VERSION,
};

use super::SandboxSetupAction;
use crate::{NetworkPolicy, SandboxPolicy};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// The non-privileged desktop process never constructs Windows tokens itself.
/// It sends a versioned, argv-shaped request to the separately signed runner;
/// absence of that runner is surfaced as SetupRequired by SandboxManager.
pub(super) fn prepare(
    policy: &SandboxPolicy,
    helper: &Path,
    state_dir: &Path,
    process: ProcessSpec,
) -> Result<ProcessSpec, String> {
    let cwd = process.cwd.clone();
    let env = process.env.clone();
    let request = WindowsRunnerRequest {
        protocol_version: RUNNER_PROTOCOL_VERSION,
        request_id: format!(
            "{}-{}",
            std::process::id(),
            REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ),
        state_dir: state_dir.to_path_buf(),
        policy: wire_policy(policy),
        process: WireProcess {
            program: WireOsString::from_os(process.program.as_os_str()),
            args: process
                .args
                .iter()
                .map(|argument| WireOsString::from_os(argument))
                .collect(),
            cwd: WireOsString::from_os(process.cwd.as_os_str()),
            env: process
                .env
                .iter()
                .map(|(name, value)| (WireOsString::from_os(name), WireOsString::from_os(value)))
                .collect(),
        },
    };
    request.validate()?;
    let encoded = encode_request(&request)?;
    Ok(ProcessSpec {
        program: helper.to_path_buf(),
        args: vec![OsString::from("--request-b64"), OsString::from(encoded)],
        cwd,
        env,
    })
}

pub(super) fn setup_action(
    policy: &SandboxPolicy,
    runner: &Path,
    setup: &Path,
    state_dir: &Path,
) -> Result<SandboxSetupAction, String> {
    let request_id = format!("setup-{}-{}", std::process::id(), uuid::Uuid::new_v4());
    let root_proofs = create_root_proofs(policy, &request_id)?;
    let cleanup_paths = root_proofs
        .iter()
        .map(|proof| proof.proof_path.clone())
        .collect();
    let request = WindowsSetupRequest {
        protocol_version: SETUP_PROTOCOL_VERSION,
        request_id,
        state_dir: state_dir.to_path_buf(),
        runner_path: runner.to_path_buf(),
        policy: wire_policy(policy),
        root_proofs,
    };
    if let Err(error) = request.validate() {
        cleanup_proofs(&request.root_proofs);
        return Err(error);
    }
    let encoded = match encode_request(&request) {
        Ok(encoded) => encoded,
        Err(error) => {
            cleanup_proofs(&request.root_proofs);
            return Err(error);
        }
    };
    Ok(SandboxSetupAction {
        program: setup.to_path_buf(),
        args: vec![OsString::from("--request-b64"), OsString::from(encoded)],
        requires_elevation: exec_sandbox_protocol::read_setup_marker(state_dir)
            .and_then(|marker| marker.validate_bootstrap(runner))
            .is_err(),
        cleanup_paths,
    })
}

fn create_root_proofs(
    policy: &SandboxPolicy,
    request_id: &str,
) -> Result<Vec<WindowsRootProof>, String> {
    let mut proofs: Vec<WindowsRootProof> = Vec::with_capacity(policy.write_roots.len());
    for (index, root) in policy.write_roots.iter().enumerate() {
        if let Err(error) = std::fs::create_dir_all(root) {
            cleanup_proofs(&proofs);
            return Err(format!(
                "create Windows sandbox write root {}: {error}",
                root.display()
            ));
        }
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let proof_path = root.join(format!(".clark-sandbox-setup-{request_id}-{index}.proof"));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&proof_path)
        {
            Ok(file) => file,
            Err(error) => {
                cleanup_proofs(&proofs);
                return Err(format!("prove write access to {}: {error}", root.display()));
            }
        };
        if let Err(error) = file
            .write_all(nonce.as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = std::fs::remove_file(&proof_path);
            cleanup_proofs(&proofs);
            return Err(format!(
                "write ownership proof in {}: {error}",
                root.display()
            ));
        }
        proofs.push(WindowsRootProof {
            root: root.clone(),
            proof_path,
            nonce,
        });
    }
    Ok(proofs)
}

fn cleanup_proofs(proofs: &[WindowsRootProof]) {
    for proof in proofs {
        let _ = std::fs::remove_file(&proof.proof_path);
    }
}

pub(super) fn wire_policy(policy: &SandboxPolicy) -> WireSandboxPolicy {
    WireSandboxPolicy {
        read_roots: policy.read_roots.clone(),
        write_roots: policy.write_roots.clone(),
        deny_read: policy.deny_read.clone(),
        deny_write: policy.deny_write.clone(),
        network: match policy.network {
            NetworkPolicy::Restricted => WireNetworkPolicy::Restricted,
            NetworkPolicy::Enabled => WireNetworkPolicy::Enabled,
        },
        process_temp_root: policy.process_temp_root.clone(),
    }
}

#[cfg(test)]
mod tests {
    use exec_sandbox_protocol::{
        setup_marker_path, sha256_file, WindowsNetworkEnforcement, WindowsSetupMarker,
        RUNNER_PROTOCOL_VERSION,
    };

    use super::*;

    #[test]
    fn first_setup_action_is_explicitly_elevated_and_versioned() {
        let temp = tempfile::tempdir().unwrap();
        let policy = SandboxPolicy::workspace_write(temp.path().to_path_buf(), vec![]);
        let action = setup_action(
            &policy,
            &temp.path().join("clark-command-runner.exe"),
            &temp.path().join("clark-windows-sandbox-setup.exe"),
            &temp.path().join("state"),
        )
        .unwrap();
        assert!(action.requires_elevation);
        assert_eq!(action.cleanup_paths.len(), 1);
        assert_eq!(action.args[0], "--request-b64");
        let request: WindowsSetupRequest =
            exec_sandbox_protocol::decode_request(action.args[1].to_string_lossy().as_ref())
                .unwrap();
        assert_eq!(request.protocol_version, SETUP_PROTOCOL_VERSION);
        assert_eq!(request.policy.network, WireNetworkPolicy::Restricted);
        assert_eq!(request.root_proofs.len(), 1);
        for path in action.cleanup_paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn bootstrapped_setup_action_enrolls_without_elevation() {
        let temp = tempfile::tempdir().unwrap();
        let runner = temp.path().join("clark-command-runner.exe");
        std::fs::write(&runner, b"runner fixture").unwrap();
        let state = temp.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let marker = WindowsSetupMarker {
            setup_protocol_version: SETUP_PROTOCOL_VERSION,
            runner_protocol_version: RUNNER_PROTOCOL_VERSION,
            runner_sha256: sha256_file(&runner).unwrap(),
            offline_identity_sid: "S-1-5-21-1000".into(),
            network_enforcement: WindowsNetworkEnforcement::WindowsFilteringPlatform,
            generation: 1,
            provisioned_policy_sha256: Vec::new(),
            provisioned_write_capability_sids: vec![WireSandboxPolicy::device_capability_sid()],
        };
        std::fs::write(
            setup_marker_path(&state),
            serde_json::to_vec(&marker).unwrap(),
        )
        .unwrap();
        let policy = SandboxPolicy::workspace_write(temp.path().join("project"), vec![]);
        let action = setup_action(
            &policy,
            &runner,
            &temp.path().join("clark-windows-sandbox-setup.exe"),
            &state,
        )
        .unwrap();
        assert!(!action.requires_elevation);
        for path in action.cleanup_paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn stable_setup_roots_attest_ephemeral_session_children() {
        let workspace = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let docs = app_data.path().join("docs");
        let temp = app_data.path().join("sandbox-tmp");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::create_dir_all(&temp).unwrap();

        let setup = SandboxPolicy::workspace_write(
            workspace.path().to_path_buf(),
            vec![docs.clone(), temp.clone()],
        );
        let runtime = SandboxPolicy::workspace_write(
            workspace.path().to_path_buf(),
            vec![docs.join("session-1"), docs, temp.clone()],
        )
        .with_process_temp_root(temp.join("session-2"));

        let setup_wire = wire_policy(&setup);
        let runtime_wire = wire_policy(&runtime);
        assert_eq!(
            setup_wire.fingerprint().unwrap(),
            runtime_wire.fingerprint().unwrap()
        );
    }
}
