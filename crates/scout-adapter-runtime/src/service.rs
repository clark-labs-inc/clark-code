use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use scout_adapter_protocol::{
    AdapterPageOutcome, AdapterPageReceipt, AdapterPageRequest, CursorHandle,
};

use crate::error::{RuntimeError, RuntimeResult};
use crate::github::GithubAdapter;
use crate::gitlab::GitlabAdapter;
use crate::process::{discover_executable, ProcessRunner, TargetEnvironment};
use crate::service_support::{adapter_build_sha256, candidate, failure_receipt};
use crate::types::{
    AuthCandidate, AuthCandidateHandle, CensusRequest, CensusResponse, FetchPageResponse,
    SafeFailure, ToolCapability, ToolKind, VerifyAuthRequest, VerifyAuthResponse,
    RUNTIME_PROTOCOL_VERSION,
};
use crate::vault::{PrivateVault, StoredAuthRef};
use crate::{aws, gcp, github, gitlab};

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_HTTP_BODY: u64 = 4 * 1024 * 1024;
const CURSOR_TTL_MS: u64 = 23 * 60 * 60 * 1_000;

pub struct RuntimeConfig {
    vault_root: PathBuf,
    process_timeout: Duration,
    http_timeout: Duration,
    max_http_body: u64,
    github_api_base: Url,
    gitlab_api_base: Url,
    target_environment: Option<TargetEnvironment>,
    gh_executable: Option<PathBuf>,
    aws_executable: Option<PathBuf>,
    gcloud_executable: Option<PathBuf>,
}

impl RuntimeConfig {
    pub fn new(vault_root: impl Into<PathBuf>) -> Self {
        Self {
            vault_root: vault_root.into(),
            process_timeout: DEFAULT_PROCESS_TIMEOUT,
            http_timeout: DEFAULT_HTTP_TIMEOUT,
            max_http_body: DEFAULT_MAX_HTTP_BODY,
            github_api_base: Url::parse("https://api.github.com/").expect("constant URL"),
            gitlab_api_base: Url::parse("https://gitlab.com/api/v4/").expect("constant URL"),
            target_environment: None,
            gh_executable: None,
            aws_executable: None,
            gcloud_executable: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        vault_root: impl Into<PathBuf>,
        environment: TargetEnvironment,
        gh_executable: Option<PathBuf>,
        aws_executable: Option<PathBuf>,
        gcloud_executable: Option<PathBuf>,
        github_api_base: Url,
    ) -> Self {
        let mut config = Self::new(vault_root);
        config.target_environment = Some(environment);
        config.gh_executable = gh_executable;
        config.aws_executable = aws_executable;
        config.gcloud_executable = gcloud_executable;
        config.github_api_base = github_api_base;
        config
    }

    #[cfg(test)]
    pub(crate) fn with_gitlab_api_base(mut self, gitlab_api_base: Url) -> Self {
        self.gitlab_api_base = gitlab_api_base;
        self
    }
}

pub struct ScoutAdapterService {
    vault: PrivateVault,
    runner: ProcessRunner,
    github: GithubAdapter,
    gitlab: GitlabAdapter,
}

impl ScoutAdapterService {
    pub fn open(config: RuntimeConfig) -> Result<Self, SafeFailure> {
        Self::try_open(config).map_err(|error| SafeFailure::from(&error))
    }

    fn try_open(config: RuntimeConfig) -> RuntimeResult<Self> {
        let environment = config
            .target_environment
            .unwrap_or_else(TargetEnvironment::capture);
        let gh = pin_executable(
            config
                .gh_executable
                .or_else(|| discover_executable("gh", &environment)),
        );
        let aws = pin_executable(
            config
                .aws_executable
                .or_else(|| discover_executable("aws", &environment)),
        );
        let gcloud = pin_executable(
            config
                .gcloud_executable
                .or_else(|| discover_executable("gcloud", &environment)),
        );
        let runner = ProcessRunner::new(environment, gh, aws, gcloud, config.process_timeout)?;
        let github = GithubAdapter::new(
            config.github_api_base,
            config.http_timeout,
            config.max_http_body,
        )?;
        let gitlab = GitlabAdapter::new(
            config.gitlab_api_base,
            config.http_timeout,
            config.max_http_body,
        )?;
        Ok(Self {
            vault: PrivateVault::open(config.vault_root)?,
            runner,
            github,
            gitlab,
        })
    }

    pub async fn census(&self, request: CensusRequest) -> CensusResponse {
        match self.try_census(request).await {
            Ok((target, candidates, tools, observed_at_ms)) => CensusResponse::Succeeded {
                target: Box::new(target),
                candidates,
                tools,
                coverage_manifest: crate::adapter_coverage_manifest(),
                observed_at_ms,
            },
            Err(error) => CensusResponse::Failed {
                failure: SafeFailure::from(&error),
            },
        }
    }

    async fn try_census(
        &self,
        request: CensusRequest,
    ) -> RuntimeResult<(
        scout_adapter_protocol::TargetIdentity,
        Vec<AuthCandidate>,
        Vec<ToolCapability>,
        u64,
    )> {
        if request.runtime_protocol_version != RUNTIME_PROTOCOL_VERSION {
            return Err(RuntimeError::InvalidRequest);
        }
        let target = self.vault.target()?;
        let mut references = Vec::new();
        for variable in ["GH_TOKEN", "GITHUB_TOKEN"] {
            if self.runner.environment().present(variable) {
                references.push(StoredAuthRef::GithubEnvironment {
                    variable: variable.to_owned(),
                });
            }
        }
        for variable in ["GITLAB_TOKEN", "GLAB_TOKEN"] {
            if self.runner.environment().present(variable) {
                references.push(StoredAuthRef::GitlabEnvironment {
                    variable: variable.to_owned(),
                });
            }
        }
        if self.runner.has_gh() {
            references.push(StoredAuthRef::GithubCli);
        }
        if self.runner.environment().present("AWS_ACCESS_KEY_ID")
            && self.runner.environment().present("AWS_SECRET_ACCESS_KEY")
        {
            references.push(StoredAuthRef::AwsEnvironment);
        }
        if self
            .runner
            .environment()
            .present("AWS_WEB_IDENTITY_TOKEN_FILE")
            && self.runner.environment().present("AWS_ROLE_ARN")
        {
            references.push(StoredAuthRef::AwsWorkload);
        }

        let (profiles, aws_census_failure) = match aws::census_profiles(&self.runner).await {
            Ok(profiles) => (profiles, None),
            Err(error) => (Vec::new(), Some(SafeFailure::from(&error))),
        };
        references.extend(
            profiles
                .into_iter()
                .map(|profile| StoredAuthRef::AwsProfile { profile }),
        );
        let (gcp_accounts, gcp_census_failure) = match gcp::census_accounts(&self.runner).await {
            Ok(accounts) => (accounts, None),
            Err(error) => (Vec::new(), Some(SafeFailure::from(&error))),
        };
        references.extend(
            gcp_accounts
                .into_iter()
                .map(|account| StoredAuthRef::GcpCli { account }),
        );
        let stored = references
            .iter()
            .map(|reference| {
                (
                    AuthCandidateHandle::for_target_ref(&target.target_id, &reference.stable_key()),
                    reference.clone(),
                )
            })
            .collect::<Vec<_>>();
        self.vault.replace_candidates(&stored)?;
        let mut candidates = stored
            .into_iter()
            .map(|(handle, reference)| candidate(handle, reference))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.handle.cmp(&right.handle));
        let native_github = self.runner.environment().present("GH_TOKEN")
            || self.runner.environment().present("GITHUB_TOKEN");
        let native_gitlab = self.runner.environment().present("GITLAB_TOKEN")
            || self.runner.environment().present("GLAB_TOKEN");
        let tools = vec![
            ToolCapability {
                tool: ToolKind::NativeGithubHttps,
                available: native_github,
                census_failure: None,
            },
            ToolCapability {
                tool: ToolKind::NativeGitlabHttps,
                available: native_gitlab,
                census_failure: None,
            },
            ToolCapability {
                tool: ToolKind::GhCli,
                available: self.runner.has_gh(),
                census_failure: None,
            },
            ToolCapability {
                tool: ToolKind::AwsCli,
                available: self.runner.has_aws(),
                census_failure: aws_census_failure,
            },
            ToolCapability {
                tool: ToolKind::GcloudCli,
                available: self.runner.has_gcloud(),
                census_failure: gcp_census_failure,
            },
        ];
        Ok((target, candidates, tools, now_ms()))
    }

    pub async fn verify_auth(&self, request: VerifyAuthRequest) -> VerifyAuthResponse {
        match self.try_verify_auth(request).await {
            Ok((target, auth_context)) => VerifyAuthResponse::Succeeded {
                target: Box::new(target),
                auth_context: Box::new(auth_context),
            },
            Err(error) => VerifyAuthResponse::Failed {
                failure: SafeFailure::from(&error),
            },
        }
    }

    async fn try_verify_auth(
        &self,
        request: VerifyAuthRequest,
    ) -> RuntimeResult<(
        scout_adapter_protocol::TargetIdentity,
        scout_adapter_protocol::AuthContextDescriptor,
    )> {
        if request.runtime_protocol_version != RUNTIME_PROTOCOL_VERSION {
            return Err(RuntimeError::InvalidRequest);
        }
        request.candidate_handle.validate()?;
        request.adapter_id.validate()?;
        let target = self.vault.target()?;
        if request.target_id != target.target_id
            || request.target_identity_sha256 != target.fingerprint_sha256()?
        {
            return Err(RuntimeError::TargetMismatch);
        }
        let reference = self.vault.candidate(&request.candidate_handle)?;
        let expected_adapter = match reference {
            StoredAuthRef::GithubEnvironment { .. } | StoredAuthRef::GithubCli => {
                github::adapter_id()
            }
            StoredAuthRef::GitlabEnvironment { .. } => gitlab::adapter_id(),
            StoredAuthRef::AwsEnvironment
            | StoredAuthRef::AwsProfile { .. }
            | StoredAuthRef::AwsWorkload => aws::adapter_id(),
            StoredAuthRef::GcpCli { .. } => gcp::adapter_id(),
        };
        if request.adapter_id != expected_adapter {
            return Err(RuntimeError::UnsupportedAdapter);
        }
        let observed_at_ms = now_ms();
        let auth_context = if expected_adapter == github::adapter_id() {
            self.github
                .verify(
                    &reference,
                    &self.runner,
                    &target,
                    request.requested_authority_scope.as_deref(),
                    observed_at_ms,
                )
                .await?
        } else if expected_adapter == gitlab::adapter_id() {
            self.gitlab
                .verify(
                    &reference,
                    &self.runner,
                    &target,
                    request.requested_authority_scope.as_deref(),
                    observed_at_ms,
                )
                .await?
        } else if expected_adapter == aws::adapter_id() {
            aws::verify(
                &reference,
                &self.runner,
                &target,
                request.requested_authority_scope.as_deref(),
                observed_at_ms,
            )
            .await?
        } else {
            gcp::verify(
                &reference,
                &self.runner,
                &target,
                request.requested_authority_scope.as_deref(),
                observed_at_ms,
            )
            .await?
        };
        self.vault.store_auth(&auth_context, reference)?;
        Ok((target, auth_context))
    }

    pub async fn fetch_page(&self, request: AdapterPageRequest) -> FetchPageResponse {
        match self.try_fetch_page(request).await {
            Ok(receipt) => FetchPageResponse::Succeeded {
                receipt: Box::new(receipt),
            },
            Err(error) => FetchPageResponse::Failed {
                failure: SafeFailure::from(&error),
            },
        }
    }

    async fn try_fetch_page(
        &self,
        request: AdapterPageRequest,
    ) -> RuntimeResult<AdapterPageReceipt> {
        let started_at_ms = now_ms();
        let target = self.vault.target()?;
        let (auth, reference) = self.vault.auth(&request.auth_context_handle)?;
        if auth
            .expires_at_ms
            .is_some_and(|expiry| expiry <= started_at_ms)
        {
            return Err(RuntimeError::AuthStale);
        }
        request.validate(&target, &auth, started_at_ms)?;
        crate::route_registry::validate_registered_route(&request)?;
        let cursor = if request.page_ordinal == 0 {
            None
        } else {
            Some(self.vault.cursor(&request, &target, &auth, started_at_ms)?)
        };
        let duration = Duration::from_millis(request.limits.max_duration_ms);
        let provider = if request.adapter_id == github::adapter_id() {
            tokio::time::timeout(
                duration,
                self.github
                    .fetch(&request, &reference, &self.runner, cursor),
            )
            .await
            .map_err(|_| RuntimeError::BoundExceeded)?
            .map(|page| (page.records, page.next_cursor, page.redaction))
        } else if request.adapter_id == gitlab::adapter_id() {
            tokio::time::timeout(
                duration,
                self.gitlab
                    .fetch(&request, &reference, &self.runner, cursor),
            )
            .await
            .map_err(|_| RuntimeError::BoundExceeded)?
            .map(|page| (page.records, page.next_cursor, page.redaction))
        } else if request.adapter_id == aws::adapter_id() {
            tokio::time::timeout(
                duration,
                aws::fetch(&request, &reference, &self.runner, cursor),
            )
            .await
            .map_err(|_| RuntimeError::BoundExceeded)?
            .map(|page| (page.records, page.next_cursor, page.redaction))
        } else if request.adapter_id == gcp::adapter_id() {
            tokio::time::timeout(
                duration,
                gcp::fetch(&request, &reference, &self.runner, cursor),
            )
            .await
            .map_err(|_| RuntimeError::BoundExceeded)?
            .map(|page| (page.records, page.next_cursor, page.redaction))
        } else {
            Err(RuntimeError::UnsupportedAdapter)
        };
        let observed_at_ms = now_ms();
        let (records, next_provider_cursor, redaction) = match provider {
            Ok(page) => page,
            Err(error) => return failure_receipt(request, target, auth, error, observed_at_ms),
        };
        crate::route_registry::validate_registered_records(&request, &records)?;
        if serde_json::to_vec(&records)
            .map_err(|_| RuntimeError::ProviderProtocol)?
            .len() as u64
            > request.limits.max_response_bytes
        {
            return failure_receipt(
                request,
                target,
                auth,
                RuntimeError::BoundExceeded,
                observed_at_ms,
            );
        }
        let next_cursor_handle = next_provider_cursor
            .as_ref()
            .map(|_| CursorHandle::random());
        let outcome = AdapterPageOutcome::Succeeded {
            final_page: next_cursor_handle.is_none(),
        };
        let receipt = AdapterPageReceipt::new(
            request,
            target,
            auth.clone(),
            adapter_build_sha256(),
            observed_at_ms,
            outcome,
            records,
            next_cursor_handle,
            redaction,
        )?;
        if let Some(cursor) = next_provider_cursor {
            let auth_expiry = auth.expires_at_ms.unwrap_or(u64::MAX);
            let cursor_expiry = observed_at_ms
                .saturating_add(CURSOR_TTL_MS)
                .min(auth_expiry);
            self.vault
                .store_cursor(&receipt, &cursor, observed_at_ms, cursor_expiry)?;
        }
        Ok(receipt)
    }

    #[cfg(test)]
    pub(crate) fn vault(&self) -> &PrivateVault {
        &self.vault
    }
}

fn pin_executable(path: Option<PathBuf>) -> Option<PathBuf> {
    path.and_then(|path| std::fs::canonicalize(path).ok())
        .filter(|path| path.is_file())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
