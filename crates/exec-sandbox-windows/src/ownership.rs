use std::path::{Path, PathBuf};

use exec_sandbox_protocol::WindowsSetupRequest;

/// Consume files that the unelevated desktop created with `create_new` inside
/// every requested ACL grant root. This turns an elevated setup request into a
/// proof of pre-existing user authority instead of a generic ACL deputy.
pub fn verify_and_consume(request: &WindowsSetupRequest) -> Result<(), String> {
    for proof in &request.root_proofs {
        let root = canonical(&proof.root, "sandbox write root")?;
        let proof_path = canonical(&proof.proof_path, "sandbox ownership proof")?;
        let proof_parent = proof_path
            .parent()
            .ok_or_else(|| "Windows sandbox ownership proof has no parent".to_string())?;
        if proof_parent != root {
            return Err(format!(
                "Windows sandbox ownership proof escaped its root: {}",
                proof.proof_path.display()
            ));
        }
        let expected_name = format!(".clark-sandbox-setup-{}-", request.request_id);
        let name = proof_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Windows sandbox ownership proof filename is invalid".to_string())?;
        if !name.starts_with(&expected_name) || !name.ends_with(".proof") {
            return Err("Windows sandbox ownership proof filename is invalid".to_string());
        }
        let content = std::fs::read_to_string(&proof_path).map_err(|error| {
            format!(
                "read Windows sandbox ownership proof {}: {error}",
                proof_path.display()
            )
        })?;
        if content != proof.nonce {
            return Err(format!(
                "Windows sandbox ownership proof content mismatch: {}",
                proof_path.display()
            ));
        }
        std::fs::remove_file(&proof_path).map_err(|error| {
            format!(
                "consume Windows sandbox ownership proof {}: {error}",
                proof_path.display()
            )
        })?;
    }
    Ok(())
}

fn canonical(path: &Path, label: &str) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use exec_sandbox_protocol::{
        WindowsRootProof, WireNetworkPolicy, WireSandboxPolicy, SETUP_PROTOCOL_VERSION,
    };

    use super::*;

    fn request(root: &Path, proof_path: PathBuf, nonce: &str) -> WindowsSetupRequest {
        WindowsSetupRequest {
            protocol_version: SETUP_PROTOCOL_VERSION,
            request_id: "ownership-test".into(),
            state_dir: root.join("state"),
            runner_path: root.join("clark-command-runner.exe"),
            policy: WireSandboxPolicy {
                read_roots: Vec::new(),
                write_roots: vec![root.to_path_buf()],
                deny_read: Vec::new(),
                deny_write: Vec::new(),
                network: WireNetworkPolicy::Restricted,
                process_temp_root: None,
            },
            root_proofs: vec![WindowsRootProof {
                root: root.to_path_buf(),
                proof_path,
                nonce: nonce.into(),
            }],
        }
    }

    #[test]
    fn consumes_a_valid_unelevated_root_proof() {
        let root = tempfile::tempdir().unwrap();
        let nonce = "0123456789abcdef0123456789abcdef";
        let proof = root
            .path()
            .join(".clark-sandbox-setup-ownership-test-0.proof");
        std::fs::write(&proof, nonce).unwrap();
        verify_and_consume(&request(root.path(), proof.clone(), nonce)).unwrap();
        assert!(!proof.exists());
    }

    #[test]
    fn rejects_a_proof_outside_the_granted_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let nonce = "0123456789abcdef0123456789abcdef";
        let proof = outside
            .path()
            .join(".clark-sandbox-setup-ownership-test-0.proof");
        std::fs::write(&proof, nonce).unwrap();
        assert!(verify_and_consume(&request(root.path(), proof.clone(), nonce)).is_err());
        assert!(proof.exists());
    }
}
