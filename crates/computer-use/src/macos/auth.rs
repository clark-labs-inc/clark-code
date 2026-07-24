use std::path::Path;
use std::str::FromStr;

#[cfg(feature = "helper-service")]
use core_foundation::base::TCFType;
#[cfg(feature = "helper-service")]
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::CFURL;
use security_framework::os::macos::code_signing::{Flags, SecRequirement, SecStaticCode};
#[cfg(feature = "helper-service")]
use security_framework::os::macos::code_signing::{GuestAttributes, SecCode};
#[cfg(feature = "helper-service")]
use security_framework_sys::code_signing::{SecCodeRef, SecRequirementRef};
#[cfg(feature = "helper-service")]
use sha2::{Digest, Sha256};

#[cfg(any(feature = "helper-service", test))]
const CLARK_PARENT_REQUIREMENT: &str = r#"
(
  identifier "com.clark.desktop"
  and anchor apple generic
  and certificate leaf[subject.OU] = "TZWY28WKAP"
)
or
(
  identifier "com.clark.desktop.dev"
  and anchor apple generic
  and certificate leaf[subject.OU] = "U94GUJNVAL"
)
"#;

const HELPER_SIGNER_REQUIREMENT: &str = r#"
identifier "clark-computer-use-helper"
and anchor apple generic
and
(
  certificate leaf[subject.OU] = "TZWY28WKAP"
  or certificate leaf[subject.OU] = "U94GUJNVAL"
)
"#;

#[cfg(feature = "helper-service")]
pub fn verify_helper_signature() -> Result<(), String> {
    let requirement = SecRequirement::from_str(HELPER_SIGNER_REQUIREMENT)
        .map_err(|error| format!("invalid embedded helper requirement: {error}"))?;
    let helper = SecCode::for_self(Flags::NONE)
        .map_err(|error| format!("could not inspect helper signature: {error}"))?;
    helper
        .check_validity(
            Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
            &requirement,
        )
        .map_err(|error| format!("helper is not signed by an approved Clark team: {error}"))
}

pub fn verify_helper_at_path(path: &Path) -> Result<(), String> {
    let url = CFURL::from_path(path, false)
        .ok_or_else(|| format!("could not create a code-signing URL for {}", path.display()))?;
    let helper = SecStaticCode::from_path(&url, Flags::NONE)
        .map_err(|error| format!("could not inspect helper at {}: {error}", path.display()))?;
    let requirement = SecRequirement::from_str(HELPER_SIGNER_REQUIREMENT)
        .map_err(|error| format!("invalid embedded helper requirement: {error}"))?;
    helper
        .check_validity(
            Flags::STRICT_VALIDATE
                | Flags::CHECK_ALL_ARCHITECTURES
                | Flags::NO_NETWORK_ACCESS,
            &requirement,
        )
        .map_err(|error| {
            format!(
                "helper at {} is unsigned, modified, or not signed by an approved Clark team: {error}",
                path.display()
            )
        })
}

#[cfg(feature = "helper-service")]
pub fn verify_parent(parent_pid: u32, socket_fd: libc::c_int) -> Result<(), String> {
    if parent_pid == 0 || parent_pid != unsafe { libc::getppid() } as u32 {
        return Err("the IPC peer is not the helper's direct parent".to_string());
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

    let requirement = SecRequirement::from_str(CLARK_PARENT_REQUIREMENT)
        .map_err(|error| format!("invalid embedded parent requirement: {error}"))?;
    let mut attributes = GuestAttributes::new();
    attributes.set_pid(parent_pid as libc::pid_t);
    let parent = SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE)
        .map_err(|error| format!("could not inspect parent code identity: {error}"))?;
    parent
        .check_validity(
            Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
            &requirement,
        )
        .map_err(|error| {
            format!(
                "parent must be a valid Clark Code production or development signature: {error}"
            )
        })
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
    fn parent_requirement_is_pinned_to_exact_products_and_teams() {
        assert!(CLARK_PARENT_REQUIREMENT.contains("com.clark.desktop"));
        assert!(CLARK_PARENT_REQUIREMENT.contains("com.clark.desktop.dev"));
        assert!(CLARK_PARENT_REQUIREMENT.contains("TZWY28WKAP"));
        assert!(CLARK_PARENT_REQUIREMENT.contains("U94GUJNVAL"));
        assert!(!CLARK_PARENT_REQUIREMENT.contains("certificate leaf[subject.OU] = \"*\""));
        assert!(HELPER_SIGNER_REQUIREMENT.contains("identifier \"clark-computer-use-helper\""));
        SecRequirement::from_str(CLARK_PARENT_REQUIREMENT).unwrap();
        SecRequirement::from_str(HELPER_SIGNER_REQUIREMENT).unwrap();
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
