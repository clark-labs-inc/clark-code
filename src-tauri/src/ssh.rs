//! Read-only SSH host discovery used before a native remote worker is attached.
//! Coding, file execution, and lifecycle live exclusively in `code-remote` and
//! the account-partitioned runtime registry.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
};

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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SshConfigHost {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
}

/// Return the named hosts that OpenSSH can resolve from the user's config.
/// Wildcard-only patterns are intentionally omitted because they are useful to
/// OpenSSH's resolver but are not concrete destinations a person can choose.
pub fn config_hosts() -> Result<Vec<SshConfigHost>, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or("could not resolve the home folder")?;
    let ssh_dir = home.join(".ssh");
    let config = ssh_dir.join("config");
    if !config.exists() {
        return Ok(Vec::new());
    }

    config_hosts_from_path(&config, &ssh_dir, &home)
}

fn config_hosts_from_path(
    config: &Path,
    ssh_dir: &Path,
    home: &Path,
) -> Result<Vec<SshConfigHost>, String> {
    let mut aliases = Vec::new();
    let mut alias_indexes = HashMap::new();
    let mut visited_files = HashSet::new();
    collect_config_hosts(
        config,
        ssh_dir,
        home,
        &mut visited_files,
        &mut alias_indexes,
        &mut aliases,
    )?;
    aliases.sort_by(|left, right| {
        left.alias
            .to_lowercase()
            .cmp(&right.alias.to_lowercase())
            .then_with(|| left.alias.cmp(&right.alias))
    });
    Ok(aliases)
}

fn collect_config_hosts(
    path: &Path,
    ssh_dir: &Path,
    home: &Path,
    visited_files: &mut HashSet<PathBuf>,
    alias_indexes: &mut HashMap<String, usize>,
    aliases: &mut Vec<SshConfigHost>,
) -> Result<(), String> {
    // Bound recursive Include expansion even if a config contains a large or
    // cyclic glob. Canonicalization below also prevents ordinary cycles.
    if visited_files.len() >= 256 {
        return Ok(());
    }
    let canonical = match path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("reading SSH config {}: {error}", path.display())),
    };
    if !visited_files.insert(canonical.clone()) {
        return Ok(());
    }
    let bytes = std::fs::read(&canonical)
        .map_err(|error| format!("reading SSH config {}: {error}", canonical.display()))?;
    let source = String::from_utf8_lossy(&bytes);

    let mut active_aliases = Vec::new();
    for line in source.lines() {
        let words = ssh_config_words(line);
        let Some((keyword, values)) = words.split_first() else {
            continue;
        };
        if keyword.eq_ignore_ascii_case("host") {
            active_aliases.clear();
            for alias in values.iter().filter(|value| literal_host_alias(value)) {
                let key = alias.to_lowercase();
                active_aliases.push(key.clone());
                if !alias_indexes.contains_key(&key) {
                    alias_indexes.insert(key, aliases.len());
                    aliases.push(SshConfigHost {
                        alias: alias.clone(),
                        hostname: None,
                        user: None,
                    });
                }
            }
        } else if keyword.eq_ignore_ascii_case("hostname") {
            set_config_host_value(
                aliases,
                alias_indexes,
                &active_aliases,
                values.first(),
                |host, value| {
                    if host.hostname.is_none() {
                        host.hostname = Some(value);
                    }
                },
            );
        } else if keyword.eq_ignore_ascii_case("user") {
            set_config_host_value(
                aliases,
                alias_indexes,
                &active_aliases,
                values.first(),
                |host, value| {
                    if host.user.is_none() {
                        host.user = Some(value);
                    }
                },
            );
        } else if keyword.eq_ignore_ascii_case("include") {
            for include in values {
                for included_path in expand_include(include, ssh_dir, home) {
                    collect_config_hosts(
                        &included_path,
                        ssh_dir,
                        home,
                        visited_files,
                        alias_indexes,
                        aliases,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn set_config_host_value(
    aliases: &mut [SshConfigHost],
    alias_indexes: &HashMap<String, usize>,
    active_aliases: &[String],
    value: Option<&String>,
    set: impl Fn(&mut SshConfigHost, String),
) {
    let Some(value) = value else {
        return;
    };
    for alias in active_aliases {
        let Some(index) = alias_indexes.get(alias) else {
            continue;
        };
        set(&mut aliases[*index], value.clone());
    }
}

fn ssh_config_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character == '#' && current.is_empty() {
            break;
        } else if character.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else if character == '=' && words.is_empty() && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        } else if character == '=' && words.len() == 1 && current.is_empty() {
            // OpenSSH permits whitespace around the optional keyword separator.
        } else {
            current.push(character);
        }
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn literal_host_alias(alias: &&String) -> bool {
    !alias.is_empty()
        && !alias.starts_with('!')
        && !alias
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | ']'))
}

fn expand_include(pattern: &str, ssh_dir: &Path, home: &Path) -> Vec<PathBuf> {
    let expanded = if pattern == "~" {
        home.to_path_buf()
    } else if let Some(relative) = pattern.strip_prefix("~/") {
        home.join(relative)
    } else {
        let path = PathBuf::from(pattern);
        if path.is_absolute() {
            path
        } else {
            ssh_dir.join(path)
        }
    };
    let Some(pattern) = expanded.to_str() else {
        return Vec::new();
    };
    let Ok(paths) = glob::glob(pattern) else {
        return Vec::new();
    };
    let mut paths = paths.filter_map(Result::ok).collect::<Vec<_>>();
    paths.sort();
    paths
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
