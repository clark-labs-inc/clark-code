use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const MAX_EXECUTABLES: usize = 4_096;
const MAX_ENVIRONMENT_NAMES: usize = 2_048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemCapabilityCensus {
    pub platform: String,
    pub architecture: String,
    pub executable_names: Vec<String>,
    pub environment_variable_names: Vec<String>,
    pub credential_surfaces: Vec<String>,
    pub executables_truncated: bool,
    pub environment_names_truncated: bool,
}

pub fn collect_system_capabilities(home: Option<&Path>) -> SystemCapabilityCensus {
    let (executable_names, executables_truncated) = std::env::var_os("PATH")
        .map(|path| executable_names(&path))
        .unwrap_or_default();
    let (environment_variable_names, environment_names_truncated) = environment_names();
    let home = home
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from));

    SystemCapabilityCensus {
        platform: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        executable_names,
        environment_variable_names,
        credential_surfaces: credential_surfaces(home.as_deref()),
        executables_truncated,
        environment_names_truncated,
    }
}

fn executable_names(path: &OsStr) -> (Vec<String>, bool) {
    let mut names = BTreeSet::new();
    let mut truncated = false;
    'directories: for directory in std::env::split_paths(path) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if names.len() >= MAX_EXECUTABLES {
                truncated = true;
                break 'directories;
            }
            let path = entry.path();
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if !metadata.is_file() || !is_executable(&path, &metadata) {
                continue;
            }
            if let Some(name) = normalized_executable_name(&path) {
                names.insert(name);
            }
        }
    }
    (names.into_iter().collect(), truncated)
}

#[cfg(unix)]
fn is_executable(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(windows)]
fn is_executable(path: &Path, _metadata: &std::fs::Metadata) -> bool {
    path.extension().is_some_and(|extension| {
        matches!(
            extension.to_string_lossy().to_ascii_lowercase().as_str(),
            "exe" | "cmd" | "bat" | "com" | "ps1"
        )
    })
}

fn normalized_executable_name(path: &Path) -> Option<String> {
    #[cfg(windows)]
    let name = path.file_stem()?;
    #[cfg(not(windows))]
    let name = path.file_name()?;

    let name = name.to_str()?;
    if name.is_empty() || name.chars().any(char::is_control) {
        None
    } else {
        Some(name.to_string())
    }
}

fn environment_names() -> (Vec<String>, bool) {
    let mut names = BTreeSet::new();
    let mut truncated = false;
    for (name, _) in std::env::vars_os() {
        if names.len() >= MAX_ENVIRONMENT_NAMES {
            truncated = true;
            break;
        }
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_environment_name(name) {
            names.insert(name.to_string());
        }
    }
    (names.into_iter().collect(), truncated)
}

fn is_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn credential_surfaces(home: Option<&Path>) -> Vec<String> {
    const CANDIDATES: &[(&str, &str)] = &[
        (".aws/config", "aws_config"),
        (".aws/credentials", "aws_shared_credentials"),
        (".config/gh/hosts.yml", "github_cli_hosts"),
        (".config/glab-cli/config.yml", "gitlab_cli_config"),
        (".azure/azureProfile.json", "azure_profile"),
        (".config/gcloud", "gcloud_config"),
        (".kube/config", "kubernetes_config"),
        (".docker/config.json", "docker_config"),
        (".config/containers/auth.json", "container_registry_config"),
        (".config/pulumi/credentials.json", "pulumi_credentials"),
        (
            ".terraform.d/credentials.tfrc.json",
            "terraform_credentials",
        ),
        (".databrickscfg", "databricks_config"),
        (".sentryclirc", "sentry_cli_config"),
        (".config/stripe/config.toml", "stripe_cli_config"),
        (".config/op/config", "onepassword_cli_config"),
        (".cargo/credentials.toml", "cargo_credentials"),
        (".npmrc", "npm_config"),
        (".pypirc", "python_package_index_config"),
        (".ssh/config", "ssh_config"),
    ];

    let mut surfaces = BTreeSet::new();
    if let Some(home) = home {
        for (relative, label) in CANDIDATES {
            if home.join(relative).exists() {
                surfaces.insert((*label).to_string());
            }
        }
    }
    for (base, candidates) in [
        (
            std::env::var_os("APPDATA").map(PathBuf::from),
            &[
                ("GitHub CLI/hosts.yml", "github_cli_hosts"),
                ("glab-cli/config.yml", "gitlab_cli_config"),
                ("gcloud", "gcloud_config"),
                ("pulumi/credentials.json", "pulumi_credentials"),
                ("stripe/config.toml", "stripe_cli_config"),
                ("pip/pip.ini", "python_package_index_config"),
            ][..],
        ),
        (
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            &[
                ("gh/hosts.yml", "github_cli_hosts"),
                ("glab-cli/config.yml", "gitlab_cli_config"),
                ("gcloud", "gcloud_config"),
                ("containers/auth.json", "container_registry_config"),
                ("pulumi/credentials.json", "pulumi_credentials"),
                ("stripe/config.toml", "stripe_cli_config"),
                ("op/config", "onepassword_cli_config"),
            ][..],
        ),
    ] {
        let Some(base) = base else { continue };
        for (relative, label) in candidates {
            if base.join(relative).exists() {
                surfaces.insert((*label).to_string());
            }
        }
    }
    surfaces.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_scan_returns_names_without_running_files() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("scout-fixture");
        std::fs::write(&executable, b"this is not a runnable program").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&executable, permissions).unwrap();
        }
        #[cfg(windows)]
        let executable = {
            let renamed = temp.path().join("scout-fixture.exe");
            std::fs::rename(executable, &renamed).unwrap();
            renamed
        };

        let (names, truncated) = executable_names(temp.path().as_os_str());
        assert_eq!(names, vec!["scout-fixture"]);
        assert!(!truncated);
        assert!(executable.exists());
    }

    #[cfg(unix)]
    #[test]
    fn executable_scan_follows_file_symlinks_without_running_them() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("real-tool");
        std::fs::write(&target, b"not actually executable code").unwrap();
        let mut permissions = std::fs::metadata(&target).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&target, permissions).unwrap();
        symlink(&target, temp.path().join("linked-tool")).unwrap();

        let (names, truncated) = executable_names(temp.path().as_os_str());
        assert!(names.contains(&"linked-tool".to_string()));
        assert!(!truncated);
    }

    #[test]
    fn credential_census_reports_labels_not_paths_or_contents() {
        let temp = tempfile::tempdir().unwrap();
        let credential = temp.path().join(".aws/credentials");
        std::fs::create_dir_all(credential.parent().unwrap()).unwrap();
        std::fs::write(&credential, b"aws_secret_access_key=do-not-report").unwrap();

        assert!(credential_surfaces(Some(temp.path()))
            .iter()
            .any(|surface| surface == "aws_shared_credentials"));
    }
}
