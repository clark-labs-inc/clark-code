use std::io::Write as _;
use std::process::Stdio;
use std::time::Duration;

use futures::StreamExt as _;

const MAX_INSTALLER_BYTES: usize = 512 * 1024;

#[cfg(windows)]
const INSTALLER_URL: &str = "https://downloads.clarkchat.com/desktop/cli/install.ps1";
#[cfg(not(windows))]
const INSTALLER_URL: &str = "https://downloads.clarkchat.com/desktop/cli/install.sh";

pub async fn run(release: Option<&str>) -> Result<(), String> {
    if release.is_some_and(|version| !valid_version(version.trim_start_matches('v'))) {
        return Err("--release must be an exact x.y.z version".into());
    }
    let installer = download_installer().await?;
    #[cfg(windows)]
    return run_windows(installer, release);
    #[cfg(not(windows))]
    run_unix(installer, release)
}

async fn download_installer() -> Result<Vec<u8>, String> {
    let client = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(Duration::from_secs(30)),
        user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    })
    .map_err(|error| format!("could not initialize Clark updater: {error}"))?;
    let response = client
        .get(INSTALLER_URL)
        .send()
        .await
        .map_err(|error| format!("could not download the Clark installer: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Clark installer download failed ({})",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_INSTALLER_BYTES as u64)
    {
        return Err("Clark installer exceeded the maximum trusted size".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("could not read Clark installer: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_INSTALLER_BYTES {
            return Err("Clark installer exceeded the maximum trusted size".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("Clark returned an empty installer".into());
    }
    Ok(bytes)
}

#[cfg(not(windows))]
fn run_unix(installer: Vec<u8>, release: Option<&str>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;

    let mut file = tempfile::NamedTempFile::new()
        .map_err(|error| format!("could not create a private Clark update file: {error}"))?;
    file.write_all(&installer)
        .map_err(|error| format!("could not write Clark installer: {error}"))?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not protect Clark installer: {error}"))?;
    let mut command = std::process::Command::new("/bin/sh");
    command.arg(file.path()).env("CLARK_NON_INTERACTIVE", "1");
    if let Some(release) = release {
        command
            .arg("--release")
            .arg(release.trim_start_matches('v'));
    }
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("could not start Clark installer: {error}"))?;
    if !status.success() {
        return Err(format!("Clark installer exited with {status}"));
    }
    Ok(())
}

#[cfg(windows)]
fn run_windows(installer: Vec<u8>, release: Option<&str>) -> Result<(), String> {
    let update_root = crate::runtime::clark_home()?.join("updates");
    std::fs::create_dir_all(&update_root)
        .map_err(|error| format!("could not prepare Clark update directory: {error}"))?;
    let path = update_root.join(format!("install-{}.ps1", uuid::Uuid::new_v4()));
    let mut file = std::fs::File::create(&path)
        .map_err(|error| format!("could not create Clark update script: {error}"))?;
    file.write_all(&installer)
        .map_err(|error| format!("could not write Clark update script: {error}"))?;
    let mut command = std::process::Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(release) = release {
        command.arg("-Release").arg(release.trim_start_matches('v'));
    }
    command
        .spawn()
        .map_err(|error| format!("could not start Clark installer: {error}"))?;
    println!(
        "Clark update started. This terminal can be closed after the installer reports completion."
    );
    Ok(())
}

fn valid_version(version: &str) -> bool {
    let parts = version.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updater_accepts_only_exact_stable_versions() {
        assert!(valid_version("1.2.3"));
        assert!(!valid_version("latest"));
        assert!(!valid_version("1.2"));
        assert!(!valid_version("1.2.3-beta"));
    }
}
