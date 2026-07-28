use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use crate::error::{RuntimeError, RuntimeResult};
pub(crate) use crate::process_support::discover_executable;
use crate::process_support::{
    isolate_process_group, read_bounded, terminate_process_tree, validate_gcloud_value,
    validate_github_name, validate_opaque_token, validate_page, validate_region,
};

#[derive(Clone)]
pub(crate) struct TargetEnvironment {
    pub(crate) values: BTreeMap<String, OsString>,
}

impl TargetEnvironment {
    pub(crate) fn capture() -> Self {
        const ALLOWED: &[&str] = &[
            "PATH",
            "HOME",
            "USERPROFILE",
            "TEMP",
            "TMP",
            "TMPDIR",
            "XDG_CONFIG_HOME",
            "GH_CONFIG_DIR",
            "CLOUDSDK_CONFIG",
            "CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE",
            "AWS_CONFIG_FILE",
            "AWS_SHARED_CREDENTIALS_FILE",
            "AWS_PROFILE",
            "AWS_DEFAULT_PROFILE",
            "AWS_REGION",
            "AWS_DEFAULT_REGION",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "AWS_WEB_IDENTITY_TOKEN_FILE",
            "AWS_ROLE_ARN",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GITLAB_TOKEN",
            "GLAB_TOKEN",
        ];
        let values = ALLOWED
            .iter()
            .filter_map(|name| std::env::var_os(name).map(|value| ((*name).to_owned(), value)))
            .collect();
        Self { values }
    }

    #[cfg(test)]
    pub(crate) fn from_values(values: impl IntoIterator<Item = (String, OsString)>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub(crate) fn present(&self, name: &str) -> bool {
        self.values.get(name).is_some_and(|value| !value.is_empty())
    }

    pub(crate) fn utf8(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(|value| value.to_str())
    }

    fn command_base(&self) -> BTreeMap<String, OsString> {
        const SAFE: &[&str] = &[
            "PATH",
            "HOME",
            "USERPROFILE",
            "TEMP",
            "TMP",
            "TMPDIR",
            "XDG_CONFIG_HOME",
            "GH_CONFIG_DIR",
            "CLOUDSDK_CONFIG",
            "CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE",
            "AWS_CONFIG_FILE",
            "AWS_SHARED_CREDENTIALS_FILE",
        ];
        SAFE.iter()
            .filter_map(|name| {
                self.values
                    .get(*name)
                    .cloned()
                    .map(|value| ((*name).to_owned(), value))
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AwsAuthMode {
    Environment,
    Profile(String),
    Workload,
}

#[derive(Clone)]
pub(crate) struct ProcessRunner {
    environment: TargetEnvironment,
    gh: Option<PathBuf>,
    aws: Option<PathBuf>,
    gcloud: Option<PathBuf>,
    timeout: Duration,
}

pub(crate) struct ProcessOutput {
    pub(crate) success: bool,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

struct FixedInvocation {
    executable: PathBuf,
    argv: Vec<OsString>,
    environment: BTreeMap<String, OsString>,
}

impl ProcessRunner {
    pub(crate) fn new(
        environment: TargetEnvironment,
        gh: Option<PathBuf>,
        aws: Option<PathBuf>,
        gcloud: Option<PathBuf>,
        timeout: Duration,
    ) -> RuntimeResult<Self> {
        if timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(RuntimeError::InvalidRequest);
        }
        Ok(Self {
            environment,
            gh,
            aws,
            gcloud,
            timeout,
        })
    }

    pub(crate) fn environment(&self) -> &TargetEnvironment {
        &self.environment
    }

    pub(crate) fn has_gh(&self) -> bool {
        self.gh.is_some()
    }

    pub(crate) fn has_aws(&self) -> bool {
        self.aws.is_some()
    }

    pub(crate) fn has_gcloud(&self) -> bool {
        self.gcloud.is_some()
    }

    pub(crate) async fn gh_user(&self) -> RuntimeResult<ProcessOutput> {
        self.run(self.gh_invocation(["api", "--method", "GET", "user"])?)
            .await
    }

    pub(crate) async fn gh_org(&self, organization: &str) -> RuntimeResult<ProcessOutput> {
        validate_github_name(organization)?;
        self.run(self.gh_invocation(["api", "--method", "GET", &format!("orgs/{organization}")])?)
            .await
    }

    pub(crate) async fn gh_repositories(
        &self,
        organization: &str,
        page: u32,
        page_size: u32,
    ) -> RuntimeResult<ProcessOutput> {
        validate_github_name(organization)?;
        validate_page(page, page_size)?;
        self.run(self.gh_invocation([
            "api",
            "--include",
            "--method",
            "GET",
            &format!("orgs/{organization}/repos"),
            "-f",
            &format!("per_page={page_size}"),
            "-f",
            &format!("page={page}"),
        ])?)
        .await
    }

    pub(crate) async fn aws_profiles(&self) -> RuntimeResult<ProcessOutput> {
        self.run(self.aws_invocation(["configure", "list-profiles"], None, None)?)
            .await
    }

    pub(crate) async fn aws_sts(&self, auth: &AwsAuthMode) -> RuntimeResult<ProcessOutput> {
        self.run(self.aws_invocation(
            [
                "--no-cli-pager",
                "sts",
                "get-caller-identity",
                "--output",
                "json",
            ],
            Some(auth),
            None,
        )?)
        .await
    }

    pub(crate) async fn aws_accounts(
        &self,
        auth: &AwsAuthMode,
        page_size: u32,
        next_token: Option<&str>,
    ) -> RuntimeResult<ProcessOutput> {
        validate_page(1, page_size)?;
        let mut argv: Vec<OsString> = vec![
            "--no-cli-pager".into(),
            "organizations".into(),
            "list-accounts".into(),
            "--max-results".into(),
            page_size.to_string().into(),
            "--output".into(),
            "json".into(),
        ];
        if let Some(token) = next_token {
            validate_opaque_token(token)?;
            argv.extend(["--starting-token".into(), token.into()]);
        }
        self.run(self.aws_invocation(argv, Some(auth), None)?).await
    }

    pub(crate) async fn aws_resources(
        &self,
        auth: &AwsAuthMode,
        region: &str,
        page_size: u32,
        next_token: Option<&str>,
    ) -> RuntimeResult<ProcessOutput> {
        validate_region(region)?;
        validate_page(1, page_size)?;
        let mut argv: Vec<OsString> = vec![
            "--no-cli-pager".into(),
            "resource-explorer-2".into(),
            "list-resources".into(),
            "--query-string".into(),
            "*".into(),
            "--max-results".into(),
            page_size.to_string().into(),
            "--region".into(),
            region.into(),
            "--output".into(),
            "json".into(),
        ];
        if let Some(token) = next_token {
            validate_opaque_token(token)?;
            argv.extend(["--next-token".into(), token.into()]);
        }
        self.run(self.aws_invocation(argv, Some(auth), Some(region))?)
            .await
    }

    pub(crate) async fn gcloud(
        &self,
        mut argv: Vec<OsString>,
        account: Option<&str>,
    ) -> RuntimeResult<ProcessOutput> {
        if argv.is_empty() || argv.len() > 32 {
            return Err(RuntimeError::InvalidRequest);
        }
        argv.push("--quiet".into());
        if let Some(account) = account {
            validate_gcloud_value(account)?;
            argv.extend(["--account".into(), account.into()]);
        }
        let executable = self
            .gcloud
            .clone()
            .ok_or(RuntimeError::ProviderUnavailable)?;
        self.run(FixedInvocation {
            executable,
            argv,
            environment: self.environment.command_base(),
        })
        .await
    }

    async fn run(&self, invocation: FixedInvocation) -> RuntimeResult<ProcessOutput> {
        let mut command = Command::new(invocation.executable);
        command
            .args(invocation.argv)
            .env_clear()
            .envs(invocation.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        isolate_process_group(&mut command);
        let mut child = command
            .spawn()
            .map_err(|_| RuntimeError::ProviderUnavailable)?;
        let root_pid = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or(RuntimeError::ProviderUnavailable)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(RuntimeError::ProviderUnavailable)?;
        let operation = async {
            let stdout = read_bounded(stdout);
            let stderr = read_bounded(stderr);
            let status = child.wait();
            let (stdout, stderr, status) = tokio::try_join!(stdout, stderr, status)
                .map_err(|_| RuntimeError::ProviderUnavailable)?;
            Ok(ProcessOutput {
                success: status.success(),
                stdout,
                stderr,
            })
        };
        match timeout(self.timeout, operation).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => {
                terminate_process_tree(&mut child, root_pid).await;
                Err(error)
            }
            Err(_) => {
                terminate_process_tree(&mut child, root_pid).await;
                Err(RuntimeError::BoundExceeded)
            }
        }
    }

    fn gh_invocation<const N: usize>(&self, argv: [&str; N]) -> RuntimeResult<FixedInvocation> {
        let executable = self.gh.clone().ok_or(RuntimeError::ProviderUnavailable)?;
        Ok(FixedInvocation {
            executable,
            argv: argv.into_iter().map(OsString::from).collect(),
            environment: self.environment.command_base(),
        })
    }

    fn aws_invocation<S: Into<OsString>>(
        &self,
        argv: impl IntoIterator<Item = S>,
        auth: Option<&AwsAuthMode>,
        region: Option<&str>,
    ) -> RuntimeResult<FixedInvocation> {
        let executable = self.aws.clone().ok_or(RuntimeError::ProviderUnavailable)?;
        let mut environment = self.environment.command_base();
        if let Some(auth) = auth {
            match auth {
                AwsAuthMode::Environment => {
                    for name in [
                        "AWS_ACCESS_KEY_ID",
                        "AWS_SECRET_ACCESS_KEY",
                        "AWS_SESSION_TOKEN",
                    ] {
                        if let Some(value) = self.environment.values.get(name) {
                            environment.insert(name.to_owned(), value.clone());
                        }
                    }
                }
                AwsAuthMode::Profile(profile) => {
                    environment.insert("AWS_PROFILE".to_owned(), profile.into());
                }
                AwsAuthMode::Workload => {
                    for name in ["AWS_WEB_IDENTITY_TOKEN_FILE", "AWS_ROLE_ARN"] {
                        if let Some(value) = self.environment.values.get(name) {
                            environment.insert(name.to_owned(), value.clone());
                        }
                    }
                }
            }
        }
        if let Some(region) = region {
            environment.insert("AWS_REGION".to_owned(), region.into());
        }
        Ok(FixedInvocation {
            executable,
            argv: argv.into_iter().map(Into::into).collect(),
            environment,
        })
    }
}
