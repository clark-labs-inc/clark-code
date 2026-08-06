use std::path::{Path, PathBuf};
use std::process::Stdio;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::spec::RemoteWorkerSpec;
use crate::transport::SshTransport;

const WORKER_BIN_DIR: &str = ".clark/bin";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteArch {
    LinuxX86_64,
    LinuxAarch64,
    DarwinArm64,
    DarwinX86_64,
}

impl RemoteArch {
    pub fn from_uname(output: &str) -> Result<Self, RemoteArtifactError> {
        let mut fields = output.split_whitespace();
        match (
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
        ) {
            ("Linux", "x86_64") => Ok(Self::LinuxX86_64),
            ("Linux", "aarch64" | "arm64") => Ok(Self::LinuxAarch64),
            ("Darwin", "arm64") => Ok(Self::DarwinArm64),
            ("Darwin", "x86_64") => Ok(Self::DarwinX86_64),
            _ => Err(RemoteArtifactError::UnsupportedPlatform(
                output.trim().into(),
            )),
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "linux-x86_64",
            Self::LinuxAarch64 => "linux-aarch64",
            Self::DarwinArm64 => "darwin-aarch64",
            Self::DarwinX86_64 => "darwin-x86_64",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RemoteArtifact {
    pub arch: RemoteArch,
    pub home: String,
    pub binary_path: String,
    pub binary_sha256: String,
    pub config_path: String,
}

impl RemoteArtifact {
    pub(crate) async fn prepare(
        spec: &RemoteWorkerSpec,
        config_bytes: &[u8],
        transport: &SshTransport,
    ) -> Result<Self, RemoteArtifactError> {
        spec.validate()
            .map_err(|error| RemoteArtifactError::Spec(error.to_string()))?;
        let (arch, home) = detect_arch_home(transport).await?;
        let roots = format!(
            "mkdir -p {} {}",
            shq(&spec.remote_root.to_string_lossy()),
            shq(&spec.trajectory_root.to_string_lossy())
        );
        if !ssh_ok(transport, &roots).await {
            return Err(RemoteArtifactError::RemoteCommand(
                "could not create registered project or trajectory root".into(),
            ));
        }
        if let Some(binary) = &spec.remote_binary {
            let allowed_prefix = format!("{home}/{WORKER_BIN_DIR}/");
            if !binary.starts_with(&allowed_prefix)
                || binary.split('/').any(|component| component == "..")
            {
                return Err(RemoteArtifactError::InvalidRemoteBinary(binary.clone()));
            }
        }
        let expected_binary_path = spec.remote_binary.clone().unwrap_or_else(|| {
            format!(
                "{home}/{WORKER_BIN_DIR}/clark-code-worker-v{}-{}",
                env!("CARGO_PKG_VERSION"),
                arch.slug()
            )
        });
        let local_binary =
            select_local_binary(spec.local_binary.as_ref(), &spec.local_binaries, arch);
        let binary_path = if let Some(local_binary) = local_binary {
            upload_verified_binary(transport, local_binary, &expected_binary_path).await?;
            expected_binary_path
        } else if ssh_ok(
            transport,
            &format!("test -x {}", shq(&expected_binary_path)),
        )
        .await
        {
            expected_binary_path
        } else {
            installed_worker(transport).await.map_err(|_| {
                RemoteArtifactError::MissingBinary(format!(
                    "{expected_binary_path}; install the current Clark Code CLI on the SSH host"
                ))
            })?
        };
        let binary_sha256 = remote_binary_sha256(transport, &binary_path).await?;

        // Configs contain no credentials and are immutable by digest. Reusing
        // them turns a cold process restart into startup rather than transfer.
        let config_path = cached_config_path(&home, config_bytes);
        upload_config(transport, config_bytes, &config_path).await?;
        Ok(Self {
            arch,
            home,
            binary_path,
            binary_sha256,
            config_path,
        })
    }
}

fn cached_config_path(home: &str, bytes: &[u8]) -> String {
    format!(
        "{home}/.clark/config/code-worker/config-{}.json",
        hex_digest(bytes)
    )
}

fn select_local_binary<'a>(
    exact: Option<&'a PathBuf>,
    by_arch: &'a std::collections::BTreeMap<String, PathBuf>,
    arch: RemoteArch,
) -> Option<&'a PathBuf> {
    exact.or_else(|| by_arch.get(arch.slug()))
}

async fn remote_binary_sha256(
    transport: &SshTransport,
    binary_path: &str,
) -> Result<String, RemoteArtifactError> {
    let command = format!(
        "set -e; binary={binary}; [ -f \"$binary\" ] && [ -x \"$binary\" ]; if command -v sha256sum >/dev/null 2>&1; then sha256sum \"$binary\" | awk '{{print $1}}'; else shasum -a 256 \"$binary\" | awk '{{print $1}}'; fi",
        binary = shq(binary_path),
    );
    let output = ssh_capture(transport, &command).await?;
    parse_sha256(&output)
}

fn parse_sha256(value: &str) -> Result<String, RemoteArtifactError> {
    let digest = value.trim().to_ascii_lowercase();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RemoteArtifactError::InvalidBinaryDigest);
    }
    Ok(digest)
}

async fn installed_worker(transport: &SshTransport) -> Result<String, RemoteArtifactError> {
    let candidate = ssh_capture(
        transport,
        "candidate=$(command -v clark-code-headless 2>/dev/null || true); [ -n \"$candidate\" ] && [ -x \"$candidate\" ] && case \"$candidate\" in /*) printf '%s' \"$candidate\";; *) exit 1;; esac",
    )
    .await?;
    let candidate = candidate.trim();
    if candidate.is_empty() || !candidate.starts_with('/') || candidate.contains(['\n', '\r', '\0'])
    {
        return Err(RemoteArtifactError::InvalidRemoteBinary(candidate.into()));
    }
    Ok(candidate.into())
}

async fn detect_arch_home(
    transport: &SshTransport,
) -> Result<(RemoteArch, String), RemoteArtifactError> {
    let output = ssh_capture(
        transport,
        "printf '%s\\n' \"$(uname -sm)\"; printf '%s' \"$HOME\"",
    )
    .await?;
    let (arch_line, home) = output.split_once('\n').unwrap_or((output.as_str(), ""));
    let arch = RemoteArch::from_uname(arch_line)?;
    let home = home.trim();
    if home.is_empty() || !home.starts_with('/') {
        return Err(RemoteArtifactError::InvalidHome(home.into()));
    }
    Ok((arch, home.into()))
}

async fn upload_verified_binary(
    transport: &SshTransport,
    local: &Path,
    remote: &str,
) -> Result<(), RemoteArtifactError> {
    if !local.is_file() {
        return Err(RemoteArtifactError::LocalBinaryMissing(local.to_path_buf()));
    }
    let bytes = tokio::fs::read(local).await?;
    let digest = hex_digest(&bytes);
    let verify_existing = format!(
        "set -e; file={remote}; [ -f \"$file\" ] && [ -x \"$file\" ]; if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum \"$file\" | awk '{{print $1}}'); else got=$(shasum -a 256 \"$file\" | awk '{{print $1}}'); fi; [ \"$got\" = {digest} ]",
        remote = shq(remote),
        digest = shq(&digest),
    );
    if ssh_ok(transport, &verify_existing).await {
        return Ok(());
    }
    let parent = remote
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or(".");
    if !ssh_ok(transport, &format!("mkdir -p {}", shq(parent))).await {
        return Err(RemoteArtifactError::RemoteCommand(
            "could not create worker directory".into(),
        ));
    }
    let temporary = format!("{remote}.part-{}", uuid::Uuid::new_v4().simple());
    if let Err(error) = scp(transport, local, &temporary).await {
        let _ = ssh_ok(transport, &format!("rm -f {}", shq(&temporary))).await;
        return Err(error);
    }
    let verify = format!(
        "set -e; chmod 700 {tmp}; if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum {tmp} | awk '{{print $1}}'); else got=$(shasum -a 256 {tmp} | awk '{{print $1}}'); fi; [ \"$got\" = {digest} ]; mv {tmp} {remote}; chmod 700 {remote}",
        tmp = shq(&temporary),
        remote = shq(remote),
        digest = shq(&digest),
    );
    if !ssh_ok(transport, &verify).await {
        let _ = ssh_ok(transport, &format!("rm -f {}", shq(&temporary))).await;
        return Err(RemoteArtifactError::RemoteCommand(
            "worker checksum verification failed".into(),
        ));
    }
    Ok(())
}

async fn upload_config(
    transport: &SshTransport,
    bytes: &[u8],
    remote: &str,
) -> Result<(), RemoteArtifactError> {
    let digest = hex_digest(bytes);
    let verify_existing = format!(
        "set -e; file={remote}; [ -f \"$file\" ]; chmod 600 \"$file\"; if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum \"$file\" | awk '{{print $1}}'); else got=$(shasum -a 256 \"$file\" | awk '{{print $1}}'); fi; [ \"$got\" = {digest} ]",
        remote = shq(remote),
        digest = shq(&digest),
    );
    if ssh_ok(transport, &verify_existing).await {
        return Ok(());
    }
    let temporary = format!("{remote}.part-{}", uuid::Uuid::new_v4().simple());
    let local = tempfile::NamedTempFile::new()?;
    tokio::fs::write(local.path(), bytes).await?;
    let parent = remote
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or(".");
    if !ssh_ok(transport, &format!("mkdir -p {}", shq(parent))).await {
        return Err(RemoteArtifactError::RemoteCommand(
            "could not create config directory".into(),
        ));
    }
    if let Err(error) = scp(transport, local.path(), &temporary).await {
        let _ = ssh_ok(transport, &format!("rm -f {}", shq(&temporary))).await;
        return Err(error);
    }
    let install = format!(
        "set -e; chmod 600 {tmp}; mv {tmp} {remote}; chmod 600 {remote}",
        tmp = shq(&temporary),
        remote = shq(remote),
    );
    if !ssh_ok(transport, &install).await {
        let _ = ssh_ok(transport, &format!("rm -f {}", shq(&temporary))).await;
        return Err(RemoteArtifactError::RemoteCommand(
            "worker config install failed".into(),
        ));
    }
    Ok(())
}

async fn ssh_capture(
    transport: &SshTransport,
    command: &str,
) -> Result<String, RemoteArtifactError> {
    let output = transport
        .ssh_command()
        .args([command])
        .stdin(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        return Err(RemoteArtifactError::Ssh(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn ssh_ok(transport: &SshTransport, command: &str) -> bool {
    transport
        .ssh_command()
        .args([command])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn scp(
    transport: &SshTransport,
    local: &Path,
    remote: &str,
) -> Result<(), RemoteArtifactError> {
    let status = transport
        .scp_command()
        .arg(local)
        .arg(transport.destination(remote))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(RemoteArtifactError::RemoteCommand("scp failed".into()))
    }
}

pub(crate) fn shq(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Error)]
pub enum RemoteArtifactError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("remote spec invalid: {0}")]
    Spec(String),
    #[error("unsupported remote platform: {0}")]
    UnsupportedPlatform(String),
    #[error("remote home is invalid: {0}")]
    InvalidHome(String),
    #[error("local worker binary is missing: {0}")]
    LocalBinaryMissing(PathBuf),
    #[error("remote worker binary is not installed: {0}")]
    MissingBinary(String),
    #[error("remote worker binary must be under the per-user Clark bin directory: {0}")]
    InvalidRemoteBinary(String),
    #[error("remote worker binary did not produce one exact SHA-256 digest")]
    InvalidBinaryDigest,
    #[error("SSH failed: {0}")]
    Ssh(String),
    #[error("remote command failed: {0}")]
    RemoteCommand(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_platform_mapping_is_explicit() {
        assert_eq!(
            RemoteArch::from_uname("Linux x86_64\n").unwrap().slug(),
            "linux-x86_64"
        );
        assert_eq!(
            RemoteArch::from_uname("Linux aarch64\n").unwrap().slug(),
            "linux-aarch64"
        );
        assert_eq!(
            RemoteArch::from_uname("Darwin arm64\n").unwrap().slug(),
            "darwin-aarch64"
        );
        assert!(RemoteArch::from_uname("Windows x86_64").is_err());
    }

    #[test]
    fn shell_quote_cannot_escape_single_quotes() {
        assert_eq!(shq("/tmp/a'b"), "'/tmp/a'\\''b'");
    }

    #[test]
    fn remote_binary_digest_is_exact_and_normalized() {
        assert_eq!(parse_sha256(&"A".repeat(64)).unwrap(), "a".repeat(64));
        assert!(matches!(
            parse_sha256("not-a-digest"),
            Err(RemoteArtifactError::InvalidBinaryDigest)
        ));
        assert!(matches!(
            parse_sha256(&format!("{} extra", "a".repeat(64))),
            Err(RemoteArtifactError::InvalidBinaryDigest)
        ));
    }

    #[test]
    fn architecture_worker_is_selected_after_remote_detection() {
        let exact = PathBuf::from("/tmp/exact-worker");
        let linux = PathBuf::from("/tmp/linux-worker");
        let workers = [("linux-x86_64".to_string(), linux.clone())]
            .into_iter()
            .collect();
        assert_eq!(
            select_local_binary(None, &workers, RemoteArch::LinuxX86_64),
            Some(&linux)
        );
        assert_eq!(
            select_local_binary(Some(&exact), &workers, RemoteArch::LinuxX86_64),
            Some(&exact)
        );
        assert_eq!(
            select_local_binary(None, &workers, RemoteArch::DarwinArm64),
            None
        );
    }

    #[test]
    fn missing_worker_error_explains_the_remote_install_contract() {
        assert!(RemoteArtifactError::MissingBinary(
            "/home/user/.clark/bin/worker; install the current Clark Code CLI on the SSH host"
                .into()
        )
        .to_string()
        .contains("install the current Clark Code CLI"));
    }

    #[test]
    fn worker_config_cache_is_content_addressed() {
        let first = cached_config_path("/home/ubuntu", br#"{"model":"one"}"#);
        let same = cached_config_path("/home/ubuntu", br#"{"model":"one"}"#);
        let changed = cached_config_path("/home/ubuntu", br#"{"model":"two"}"#);
        assert_eq!(first, same);
        assert_ne!(first, changed);
        assert!(first.starts_with("/home/ubuntu/.clark/config/code-worker/config-"));
        assert!(first.ends_with(".json"));
    }
}
