use std::path::Path;
use std::str::FromStr;

#[cfg(feature = "helper-service")]
use core_foundation::base::TCFType;
#[cfg(feature = "helper-service")]
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::CFURL;
use security_framework::os::macos::code_signing::{
    Flags, GuestAttributes, SecCode, SecRequirement, SecStaticCode,
};
#[cfg(feature = "helper-service")]
use security_framework_sys::code_signing::{SecCodeRef, SecRequirementRef};
#[cfg(feature = "helper-service")]
use sha2::{Digest, Sha256};

include!(concat!(env!("OUT_DIR"), "/signing.rs"));

#[cfg(feature = "helper-service")]
pub fn verify_service_signature() -> Result<(), String> {
    let requirement = SecRequirement::from_str(SERVICE_SIGNING_REQUIREMENT)
        .map_err(|error| format!("invalid embedded service requirement: {error}"))?;
    let service = SecCode::for_self(Flags::NONE)
        .map_err(|error| format!("could not inspect service signature: {error}"))?;
    service
        .check_validity(
            Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
            &requirement,
        )
        .map_err(|error| format!("service is not a signed Computer Use app: {error}"))
}

pub fn verify_service_at_path(path: &Path) -> Result<(), String> {
    let url = CFURL::from_path(path, false).ok_or_else(|| {
        format!(
            "could not create a service code-signing URL for {}",
            path.display()
        )
    })?;
    let service = SecStaticCode::from_path(&url, Flags::NONE)
        .map_err(|error| format!("could not inspect service at {}: {error}", path.display()))?;
    let requirement = SecRequirement::from_str(SERVICE_SIGNING_REQUIREMENT)
        .map_err(|error| format!("invalid embedded service requirement: {error}"))?;
    service
        .check_validity(
            Flags::STRICT_VALIDATE | Flags::CHECK_ALL_ARCHITECTURES | Flags::NO_NETWORK_ACCESS,
            &requirement,
        )
        .map_err(|error| {
            format!(
                "service at {} is unsigned, modified, or not a signed Computer Use app: {error}",
                path.display()
            )
        })
}

#[cfg(feature = "helper-service")]
pub fn verify_client_peer(client_pid: u32, socket_fd: libc::c_int) -> Result<(), String> {
    if client_pid == 0 {
        return Err("the IPC client PID is invalid".to_string());
    }
    let mut peer_uid = 0;
    let mut peer_gid = 0;
    let peer_result = unsafe { libc::getpeereid(socket_fd, &mut peer_uid, &mut peer_gid) };
    if peer_result != 0 {
        return Err(format!(
            "could not authenticate IPC peer credentials: {}",
            std::io::Error::last_os_error()
        ));
    }
    if peer_uid != unsafe { libc::geteuid() } {
        return Err("the IPC peer belongs to a different user".to_string());
    }
    let peer_pid = peer_pid(socket_fd)?;
    if peer_pid != client_pid {
        return Err("the IPC peer PID does not match the signed client handshake".to_string());
    }

    verify_process(client_pid, CLIENT_SIGNING_REQUIREMENT, "client")
}

pub fn verify_service_pid(service_pid: u32) -> Result<(), String> {
    verify_process(service_pid, SERVICE_SIGNING_REQUIREMENT, "service")
}

fn verify_process(pid: u32, requirement: &str, role: &str) -> Result<(), String> {
    let requirement = SecRequirement::from_str(requirement)
        .map_err(|error| format!("invalid embedded {role} requirement: {error}"))?;
    let mut attributes = GuestAttributes::new();
    attributes.set_pid(pid as libc::pid_t);
    let process = SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE)
        .map_err(|error| format!("could not inspect {role} code identity: {error}"))?;
    process
        .check_validity(
            Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
            &requirement,
        )
        .map_err(|error| format!("{role} failed its product signing requirement: {error}"))
}

#[cfg(feature = "helper-service")]
fn peer_pid(socket_fd: libc::c_int) -> Result<u32, String> {
    let mut pid: libc::c_int = 0;
    let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            socket_fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if result != 0 || length as usize != std::mem::size_of::<libc::c_int>() || pid <= 0 {
        return Err(format!(
            "could not authenticate IPC peer PID: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(pid as u32)
}

#[cfg(feature = "helper-service")]
pub fn resolve_application_identity(
    pid: i32,
    expected_bundle_id: &str,
) -> Result<crate::ApplicationIdentity, String> {
    if pid <= 0 || expected_bundle_id.trim().is_empty() {
        return Err("target signing identity requires a positive PID and bundle ID".to_string());
    }
    let mut attributes = GuestAttributes::new();
    attributes.set_pid(pid);
    let code = SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE)
        .map_err(|error| format!("could not inspect target process signature: {error}"))?;
    let requirement = copy_designated_requirement(&code)?;
    code.check_validity(
        Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
        &requirement,
    )
    .map_err(|error| format!("target process failed its designated requirement: {error}"))?;
    let designated_requirement = requirement_string(&requirement)?;
    let durable_approval_eligible = signer_is_durable(&designated_requirement);
    let team_identifier = team_identifier(&designated_requirement);
    let mut digest = Sha256::new();
    digest.update(expected_bundle_id.as_bytes());
    digest.update([0]);
    digest.update(designated_requirement.as_bytes());
    let identity_key = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(crate::ApplicationIdentity {
        bundle_id: expected_bundle_id.to_string(),
        team_identifier,
        designated_requirement,
        identity_key,
        durable_approval_eligible,
    })
}

#[cfg(feature = "helper-service")]
fn copy_designated_requirement(code: &SecCode) -> Result<SecRequirement, String> {
    let mut requirement: SecRequirementRef = std::ptr::null_mut();
    let result = unsafe {
        SecCodeCopyDesignatedRequirement(code.as_concrete_TypeRef(), 0, &mut requirement)
    };
    if result != 0 || requirement.is_null() {
        return Err(format!(
            "could not resolve target designated requirement: OSStatus {result}"
        ));
    }
    Ok(unsafe { SecRequirement::wrap_under_create_rule(requirement) })
}

#[cfg(feature = "helper-service")]
fn requirement_string(requirement: &SecRequirement) -> Result<String, String> {
    let mut value: CFStringRef = std::ptr::null();
    let result =
        unsafe { SecRequirementCopyString(requirement.as_concrete_TypeRef(), 0, &mut value) };
    if result != 0 || value.is_null() {
        return Err(format!(
            "could not render target designated requirement: OSStatus {result}"
        ));
    }
    Ok(unsafe { CFString::wrap_under_create_rule(value) }.to_string())
}

#[cfg(feature = "helper-service")]
fn signer_is_durable(requirement: &str) -> bool {
    requirement.contains("anchor apple") || requirement.contains("certificate leaf")
}

#[cfg(feature = "helper-service")]
fn team_identifier(requirement: &str) -> Option<String> {
    const MARKER: &str = "certificate leaf[subject.OU] = ";
    let remainder = requirement.split_once(MARKER)?.1.trim_start();
    if let Some(quoted) = remainder.strip_prefix('"') {
        return quoted
            .split_once('"')
            .map(|(team, _)| team.to_string())
            .filter(|team| !team.is_empty());
    }
    remainder
        .split_whitespace()
        .next()
        .map(|team| team.trim_matches(['(', ')']).to_string())
        .filter(|team| !team.is_empty())
}

#[cfg(feature = "helper-service")]
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCodeCopyDesignatedRequirement(
        code: SecCodeRef,
        flags: u32,
        requirement: *mut SecRequirementRef,
    ) -> i32;
    fn SecRequirementCopyString(
        requirement: SecRequirementRef,
        flags: u32,
        text: *mut CFStringRef,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_requirements_pin_exact_products_and_teams() {
        assert!(CLIENT_SIGNING_REQUIREMENT.contains("org.agentdesktop.app"));
        assert!(SERVICE_SIGNING_REQUIREMENT.contains("org.agentdesktop.computer-use"));
        for requirement in [CLIENT_SIGNING_REQUIREMENT, SERVICE_SIGNING_REQUIREMENT] {
            assert!(requirement.contains("AGENTPROD"));
            assert!(requirement.contains("AGENTDEV"));
            assert!(!requirement.contains("certificate leaf[subject.OU] = \"*\""));
            SecRequirement::from_str(requirement).unwrap();
        }
    }

    #[cfg(feature = "helper-service")]
    #[test]
    fn team_parser_and_durable_signer_detection_are_bounded() {
        let requirement = r#"identifier "com.example.App" and anchor apple generic and certificate leaf[subject.OU] = "TEAM123""#;
        assert_eq!(team_identifier(requirement).as_deref(), Some("TEAM123"));
        assert!(signer_is_durable(requirement));
        assert!(!signer_is_durable(
            r#"identifier "com.example.Unsigned" and cdhash H"00""#
        ));
    }
}
