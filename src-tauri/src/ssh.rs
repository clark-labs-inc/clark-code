//! Read-only SSH host discovery used before a native remote worker is attached.
//! Coding, file execution, and lifecycle live exclusively in `code-remote` and
//! the account-partitioned runtime registry.

use std::process::Stdio;

use tokio::process::Command;

const SSH_CONNECT_TIMEOUT: &str = "10";

fn background_command(program: &str) -> Command {
    let mut command = Command::new(program);
    exec_core::suppress_console_window(&mut command);
    command
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteArch {
    LinuxX86_64,
    LinuxAarch64,
    DarwinArm64,
    DarwinX86_64,
}

impl RemoteArch {
    fn from_uname(out: &str) -> Result<Self, String> {
        let mut parts = out.split_whitespace();
        match (parts.next().unwrap_or(""), parts.next().unwrap_or("")) {
            ("Linux", "x86_64") => Ok(Self::LinuxX86_64),
            ("Linux", "aarch64" | "arm64") => Ok(Self::LinuxAarch64),
            ("Darwin", "arm64") => Ok(Self::DarwinArm64),
            ("Darwin", "x86_64") => Ok(Self::DarwinX86_64),
            _ => Err(format!("unsupported remote platform: {:?}", out.trim())),
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "linux-x86_64",
            Self::LinuxAarch64 => "linux-aarch64",
            Self::DarwinArm64 => "darwin-aarch64",
            Self::DarwinX86_64 => "darwin-x86_64",
        }
    }
}

#[derive(serde::Serialize)]
pub struct Probe {
    pub arch: String,
    pub home: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RemoteDirectory {
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RemoteDirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub directories: Vec<RemoteDirectory>,
}

pub async fn probe(host: &str) -> Result<Probe, String> {
    let (arch, home) = detect_arch_and_home(host).await?;
    Ok(Probe {
        arch: arch.slug().into(),
        home,
    })
}

pub async fn list_directories(
    host: &str,
    path: Option<&str>,
) -> Result<RemoteDirectoryListing, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("SSH host is required".into());
    }
    let target = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("target={}", shq(value)))
        .unwrap_or_else(|| "target=$HOME".into());
    let command = format!(
        "{target}; cd \"$target\" 2>/dev/null || {{ printf '%s\\n' 'Folder is not accessible' >&2; exit 2; }}; \
         current=$(pwd -P) || exit 2; printf '%s\\0' \"$current\"; \
         for entry in .[!.]* ..?* *; do [ -d \"$entry\" ] || continue; \
         name=${{entry#./}}; printf '%s\\0' \"$name\"; done"
    );
    parse_directory_listing(&ssh_capture_bytes(host, &command).await?)
}

async fn detect_arch_and_home(host: &str) -> Result<(RemoteArch, String), String> {
    let output = ssh_capture(
        host.trim(),
        "printf '%s\\n' \"$(uname -sm)\"; printf '%s' \"$HOME\"",
    )
    .await?;
    parse_arch_and_home(&output)
}

fn parse_arch_and_home(output: &str) -> Result<(RemoteArch, String), String> {
    let mut lines = output.split('\n');
    let arch = RemoteArch::from_uname(lines.next().unwrap_or(""))?;
    let home = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    if home.is_empty() {
        return Err("could not resolve remote $HOME".into());
    }
    Ok((arch, home))
}

fn parse_directory_listing(output: &[u8]) -> Result<RemoteDirectoryListing, String> {
    let mut fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let path = fields
        .next()
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .filter(|value| value.starts_with('/'))
        .ok_or("remote folder listing did not return an absolute path")?;
    let mut directories = fields
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .filter(|name| name != "." && name != ".." && !name.contains('/'))
        .map(|name| RemoteDirectory {
            path: if path == "/" {
                format!("/{name}")
            } else {
                format!("{path}/{name}")
            },
            name,
        })
        .collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(RemoteDirectoryListing {
        parent: remote_parent(&path),
        path,
        directories,
    })
}

fn remote_parent(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    let (parent, _) = path.rsplit_once('/')?;
    (!path.is_empty()).then(|| if parent.is_empty() { "/" } else { parent }.to_string())
}

async fn ssh_capture(host: &str, remote_command: &str) -> Result<String, String> {
    let output = ssh_output(host, remote_command).await?;
    Ok(String::from_utf8_lossy(&output).into_owned())
}

async fn ssh_capture_bytes(host: &str, remote_command: &str) -> Result<Vec<u8>, String> {
    ssh_output(host, remote_command).await
}

async fn ssh_output(host: &str, remote_command: &str) -> Result<Vec<u8>, String> {
    if host.is_empty() {
        return Err("SSH host is required".into());
    }
    let output = background_command("ssh")
        .args([
            "-o",
            &format!("ConnectTimeout={SSH_CONNECT_TIMEOUT}"),
            host,
            remote_command,
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| format!("spawning ssh: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("SSH request to {host} failed")
        } else {
            detail
        });
    }
    Ok(output.stdout)
}

fn shq(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests;
