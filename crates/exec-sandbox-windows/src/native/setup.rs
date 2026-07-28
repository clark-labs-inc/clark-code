use std::path::{Path, PathBuf};

use exec_sandbox_protocol::{
    read_setup_marker, setup_marker_path, WindowsSetupMarker, WindowsSetupRequest,
};

use crate::provision::{EnrollmentHost, ProvisionedIdentity, ProvisioningHost};
use crate::state::write_json_atomic;

pub struct WindowsProvisioningHost {
    state_dir: PathBuf,
}

impl WindowsProvisioningHost {
    pub fn new(state_dir: PathBuf) -> Self {
        Self { state_dir }
    }
}

impl ProvisioningHost for WindowsProvisioningHost {
    fn validate_bootstrap(&mut self, request: &WindowsSetupRequest) -> Result<(), String> {
        validate_private_runner(&request.runner_path)?;
        validate_user_state_dir(&request.state_dir)?;
        Ok(())
    }

    fn ensure_offline_identity(&mut self) -> Result<ProvisionedIdentity, String> {
        super::identity::ensure_offline_identity(&self.state_dir)
    }

    fn ensure_network_denied(&mut self, identity: &ProvisionedIdentity) -> Result<(), String> {
        super::firewall::ensure_network_denied(&identity.sid)
    }

    fn reconcile_global_objects(&mut self) -> Result<(), String> {
        super::acl::bootstrap()
    }

    fn existing_marker(&mut self) -> Option<WindowsSetupMarker> {
        read_setup_marker(&self.state_dir).ok()
    }

    fn commit_marker(&mut self, marker: &WindowsSetupMarker) -> Result<(), String> {
        write_json_atomic(&setup_marker_path(&self.state_dir), marker)
    }
}

impl EnrollmentHost for WindowsProvisioningHost {
    fn validate_enrollment(&mut self, request: &WindowsSetupRequest) -> Result<(), String> {
        validate_private_runner(&request.runner_path)?;
        validate_user_state_dir(&request.state_dir)?;
        crate::ownership::verify_and_consume(request)?;
        for denied in &request.policy.deny_write {
            if !request
                .policy
                .write_roots
                .iter()
                .any(|root| denied.starts_with(root))
            {
                return Err(format!(
                    "deny-write root is outside every granted root: {}",
                    denied.display()
                ));
            }
        }
        Ok(())
    }

    fn existing_marker(&mut self) -> Option<WindowsSetupMarker> {
        read_setup_marker(&self.state_dir).ok()
    }

    fn verify_identity(&mut self, identity: &ProvisionedIdentity) -> Result<(), String> {
        let (_, _, sid) = super::identity::load_offline_password(&self.state_dir)?;
        if sid.eq_ignore_ascii_case(&identity.sid) {
            Ok(())
        } else {
            Err("Windows sandbox credential SID does not match bootstrap attestation".into())
        }
    }

    fn verify_network_denied(&mut self, identity: &ProvisionedIdentity) -> Result<(), String> {
        super::firewall::verify_network_denied(&identity.sid)
    }

    fn reconcile_workspace_acl(
        &mut self,
        request: &WindowsSetupRequest,
        identity: &ProvisionedIdentity,
    ) -> Result<(), String> {
        super::acl::enroll(request, &identity.sid)
    }

    fn commit_marker(&mut self, marker: &WindowsSetupMarker) -> Result<(), String> {
        write_json_atomic(&setup_marker_path(&self.state_dir), marker)
    }
}

fn validate_private_runner(runner: &Path) -> Result<(), String> {
    if runner.file_name().and_then(|name| name.to_str()) != Some("clark-command-runner.exe") {
        return Err("Windows sandbox runner has an unexpected filename".into());
    }
    let setup = std::env::current_exe()
        .map_err(|error| format!("resolve Windows sandbox setup executable: {error}"))?;
    let runner_parent = canonical_parent(runner)?;
    let setup_parent = canonical_parent(&setup)?;
    if runner_parent != setup_parent {
        return Err("Windows sandbox runner is not beside the setup helper".into());
    }
    Ok(())
}

fn validate_user_state_dir(state_dir: &Path) -> Result<(), String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA is unavailable".to_string())?;
    validate_user_state_dir_under(&local_app_data, state_dir)
}

fn validate_user_state_dir_under(local_app_data: &Path, state_dir: &Path) -> Result<(), String> {
    let allowed = ["Code", "Code Dev"]
        .map(|product| local_app_data.join("Clark").join(product).join("sandbox"));
    if !allowed
        .iter()
        .any(|expected| normalize(state_dir) == normalize(expected))
    {
        return Err(format!(
            "Windows sandbox state directory must be {} or {}",
            allowed[0].display(),
            allowed[1].display()
        ));
    }
    Ok(())
}

fn canonical_parent(path: &Path) -> Result<PathBuf, String> {
    path.parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?
        .canonicalize()
        .map_err(|error| format!("canonicalize install directory: {error}"))
}

fn normalize(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::validate_user_state_dir_under;
    use std::path::Path;

    #[test]
    fn accepts_only_production_and_development_state_roots() {
        let local = Path::new(r"C:\Users\tester\AppData\Local");
        assert!(validate_user_state_dir_under(
            local,
            Path::new(r"C:\Users\tester\AppData\Local\Clark\Code\sandbox")
        )
        .is_ok());
        assert!(validate_user_state_dir_under(
            local,
            Path::new(r"C:\Users\tester\AppData\Local\Clark\Code Dev\sandbox")
        )
        .is_ok());
        assert!(validate_user_state_dir_under(
            local,
            Path::new(r"C:\Users\tester\AppData\Local\sandbox")
        )
        .is_err());
        assert!(validate_user_state_dir_under(
            local,
            Path::new(r"C:\Users\other\AppData\Local\Clark\Code Dev\sandbox")
        )
        .is_err());
        assert!(validate_user_state_dir_under(
            local,
            Path::new(r"C:\Users\tester\AppData\Local\Clark Code\sandbox")
        )
        .is_err());
    }
}
