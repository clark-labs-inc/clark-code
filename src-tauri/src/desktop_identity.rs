//! Machine-bound remote-control identity. Raw machine identifiers never leave
//! this process; registrations carry only a product-scoped digest.
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
pub struct DesktopIdentity {
    id: String,
    name: String,
}

fn scoped_identity(machine: &str) -> Result<String, String> {
    if machine.trim().is_empty() {
        return Err("This computer's identity is unavailable".into());
    }
    let mut hash = Sha256::new();
    hash.update(b"clark-code-remote-host-v1\0");
    hash.update(machine.trim().as_bytes());
    Ok(format!("computer-{:x}", hash.finalize()))
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(program).args(args).output()
        .map_err(|_| "Could not read this computer's identity".to_string())?;
    if !output.status.success() {
        return Err("Could not read this computer's identity".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "macos")]
fn machine_identity() -> Result<(String, String), String> {
    let info = output("/usr/sbin/ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])?;
    let machine = info.lines().find(|line| line.contains("\"IOPlatformUUID\""))
        .and_then(|line| line.split('=') .nth(1))
        .map(|value| value.trim().trim_matches('"').to_string())
        .ok_or_else(|| "This computer's identity is unavailable".to_string())?;
    let name = output("/usr/sbin/scutil", &["--get", "ComputerName"])
        .or_else(|_| output("/bin/hostname", &[]))?;
    Ok((machine, name))
}

#[cfg(target_os = "linux")]
fn machine_identity() -> Result<(String, String), String> {
    let machine = std::fs::read_to_string("/etc/machine-id")
        .map_err(|_| "This computer's identity is unavailable".to_string())?;
    let name = output("hostname", &[])?;
    Ok((machine, name))
}

#[cfg(target_os = "windows")]
fn machine_identity() -> Result<(String, String), String> {
    let info = output("reg", &["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"])?;
    let machine = info.lines().find(|line| line.contains("MachineGuid"))
        .and_then(|line| line.split_whitespace().last())
        .ok_or_else(|| "This computer's identity is unavailable".to_string())?.to_string();
    let name = std::env::var("COMPUTERNAME").map_err(|_| "Computer name unavailable".to_string())?;
    Ok((machine, name))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn machine_identity() -> Result<(String, String), String> {
    Err("Remote desktop control is unavailable on this platform".into())
}

#[tauri::command]
pub async fn desktop_identity() -> Result<DesktopIdentity, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let (machine, name) = machine_identity()?;
        Ok(DesktopIdentity { id: scoped_identity(&machine)?, name })
    }).await.map_err(|_| "Could not identify this computer".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn reads_this_macs_identity_without_keychain_or_signing_access() {
        let (machine, name) = machine_identity().unwrap();
        assert!(!name.trim().is_empty());
        assert_eq!(scoped_identity(&machine).unwrap().len(), "computer-".len() + 64);
    }

    #[test]
    fn identity_is_stable_distinct_and_does_not_expose_machine_id() {
        let first = scoped_identity("machine-one").unwrap();
        assert_eq!(first, scoped_identity("machine-one\n").unwrap());
        assert_ne!(first, scoped_identity("machine-two").unwrap());
        assert!(!first.contains("machine-one"));
        assert!(scoped_identity("  ").is_err());
    }
}
