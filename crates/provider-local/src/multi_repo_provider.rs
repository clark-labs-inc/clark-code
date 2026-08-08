use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_core::provider::{Provider, ProviderConfig};
use agent_orchestration::{
    HarnessKind, IntegrationHarnessAttempt, MultiRepoIntegrationHarness, MultiRepoPlan,
    MultiRepoReaderHarness, MultiRepoReviewHarness, MultiRepoTask, MultiRepoTaskRole,
    MultiRepoWriterHarness, ProviderFactory, ReaderFailure, ReaderHarnessAttempt,
    ReviewHarnessAttempt, ReviewReceipt, UsageCharge, WriterFailure, WriterHarnessAttempt,
};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::{
    Executor, FreshIntegrationWorkspace, IsolatedReaderWorkspace, IsolatedWriterWorkspace,
    LocalAgentProvider,
};
use crate::{LocalExecutor, RepositorySelection};

#[path = "multi_repo_provider/events.rs"]
mod events;
use events::run_provider;
#[path = "multi_repo_provider/cloud.rs"]
mod cloud;
pub use cloud::{BrokeredCloudWriterConfig, BrokeredCloudWriterHarness};
#[path = "multi_repo_provider/integration.rs"]
mod integration;
#[path = "multi_repo_provider/prompt.rs"]
mod prompt;
use integration::run_integration_checks;
use prompt::{
    isolated_provider_config, parse_reader, parse_review, reader_prompt, review_prompt,
    writer_prompt,
};

#[derive(Clone)]
pub struct LocalMultiRepoRuntime {
    provider_config: ProviderConfig,
    timeout: Duration,
    scratch_root: PathBuf,
    artifact_root: PathBuf,
    selection: Arc<RepositorySelection>,
    plan: Arc<MultiRepoPlan>,
    factory: Arc<dyn ProviderFactory>,
    executor: Arc<dyn Executor>,
    integration_gate: Option<Arc<dyn IntegrationReadinessGate>>,
}

pub struct LocalMultiRepoRuntimeConfig {
    pub provider_config: ProviderConfig,
    pub timeout: Duration,
    pub scratch_root: PathBuf,
    pub artifact_root: PathBuf,
    pub selection: Arc<RepositorySelection>,
    pub plan: Arc<MultiRepoPlan>,
    pub integration_gate: Option<Arc<dyn IntegrationReadinessGate>>,
}

#[async_trait]
pub trait IntegrationReadinessGate: Send + Sync {
    async fn wait_ready(&self, cancel: CancellationToken) -> Result<(), String>;
}

impl LocalMultiRepoRuntime {
    pub fn new(
        config: LocalMultiRepoRuntimeConfig,
        factory: Arc<dyn ProviderFactory>,
        executor: Arc<dyn Executor>,
    ) -> Result<Self, String> {
        config.plan.validate()?;
        if config.timeout.is_zero() {
            return Err("multi-repository provider timeout must be greater than zero".into());
        }
        validate_runtime_root(&config.scratch_root, &config.selection, "scratch")?;
        validate_runtime_root(&config.artifact_root, &config.selection, "artifact")?;
        Ok(Self {
            provider_config: config.provider_config,
            timeout: config.timeout,
            scratch_root: config.scratch_root,
            artifact_root: config.artifact_root,
            selection: config.selection,
            plan: config.plan,
            factory,
            executor,
            integration_gate: config.integration_gate,
        })
    }

    pub fn local(
        provider_config: ProviderConfig,
        timeout: Duration,
        scratch_root: PathBuf,
        artifact_root: PathBuf,
        selection: Arc<RepositorySelection>,
        plan: Arc<MultiRepoPlan>,
    ) -> Result<Self, String> {
        Self::new(
            LocalMultiRepoRuntimeConfig {
                provider_config,
                timeout,
                scratch_root,
                artifact_root,
                selection,
                plan,
                integration_gate: None,
            },
            Arc::new(|| Box::new(LocalAgentProvider::new()) as Box<dyn Provider>),
            Arc::new(LocalExecutor),
        )
    }

    pub fn writer_harness(&self, id: impl Into<String>) -> Result<LocalWriterHarness, String> {
        let id = id.into();
        self.require_task_harness(&id, MultiRepoTaskRole::Writer)?;
        Ok(LocalWriterHarness {
            id,
            runtime: self.clone(),
        })
    }

    pub fn reader_harness(&self, id: impl Into<String>) -> Result<LocalReaderHarness, String> {
        let id = id.into();
        self.require_task_harness(&id, MultiRepoTaskRole::Reader)?;
        Ok(LocalReaderHarness {
            id,
            runtime: self.clone(),
        })
    }

    pub fn reviewer_harness(&self, id: impl Into<String>) -> Result<LocalReviewHarness, String> {
        let id = id.into();
        self.require_task_harness(&id, MultiRepoTaskRole::Reviewer)?;
        Ok(LocalReviewHarness {
            id,
            runtime: self.clone(),
        })
    }

    pub fn integration_harness(
        &self,
        id: impl Into<String>,
    ) -> Result<LocalIntegrationHarness, String> {
        let id = id.into();
        self.require_task_harness(&id, MultiRepoTaskRole::Integrator)?;
        Ok(LocalIntegrationHarness {
            id,
            runtime: self.clone(),
        })
    }

    fn require_task_harness(&self, id: &str, role: MultiRepoTaskRole) -> Result<(), String> {
        if id.trim().is_empty() {
            return Err("multi-repository harness id must not be empty".into());
        }
        let matches = self
            .plan
            .tasks
            .iter()
            .filter(|task| task.role == role && task.harness == id)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(format!("plan has no {role:?} task for harness {id}"));
        }
        if matches
            .iter()
            .any(|task| task.harness_kind != HarnessKind::Local)
        {
            return Err("local runtime cannot serve a non-local task".into());
        }
        Ok(())
    }

    async fn verify_primaries(&self) -> Result<(), String> {
        self.selection
            .verify_primaries_unchanged(self.executor.as_ref())
            .await
    }
}

pub struct LocalReaderHarness {
    id: String,
    runtime: LocalMultiRepoRuntime,
}

#[async_trait]
impl MultiRepoReaderHarness for LocalReaderHarness {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::Local
    }

    async fn run(
        &self,
        task: MultiRepoTask,
        _attempt: u32,
        cancel: CancellationToken,
    ) -> Result<ReaderHarnessAttempt, ReaderFailure> {
        let result = self.run_inner(task, cancel).await;
        let primary_check = self.runtime.verify_primaries().await;
        match (result, primary_check) {
            (_, Err(error)) => Err(reader_failure(format!(
                "primary checkout integrity failed: {error}"
            ))),
            (result, Ok(())) => result,
        }
    }
}

impl LocalReaderHarness {
    async fn run_inner(
        &self,
        task: MultiRepoTask,
        cancel: CancellationToken,
    ) -> Result<ReaderHarnessAttempt, ReaderFailure> {
        validate_task(&task, &self.id, MultiRepoTaskRole::Reader).map_err(reader_failure)?;
        self.runtime
            .verify_primaries()
            .await
            .map_err(reader_failure)?;
        let workspace = IsolatedReaderWorkspace::create(
            self.runtime.executor.as_ref(),
            &self.runtime.selection,
            task.clone(),
            &self.runtime.scratch_root,
        )
        .await
        .map_err(reader_failure)?;
        let config = isolated_provider_config(
            self.runtime.provider_config.clone(),
            &workspace.root,
            &task.model,
            false,
        )
        .map_err(reader_failure)?;
        let collected = run_provider(
            self.runtime.factory.as_ref(),
            config,
            &workspace.root,
            agent_core::provider::PromptInput::text(reader_prompt(&task)),
            self.runtime.timeout,
            cancel,
        )
        .await
        .map_err(|failure| ReaderFailure {
            message: failure.message,
            usage: failure.usage,
        })?;
        let report =
            parse_reader(&collected.final_message, &task).map_err(|message| ReaderFailure {
                message,
                usage: collected.usage.clone(),
            })?;
        Ok(ReaderHarnessAttempt {
            report,
            usage: collected.usage,
        })
    }
}

pub struct LocalWriterHarness {
    id: String,
    runtime: LocalMultiRepoRuntime,
}

#[async_trait]
impl MultiRepoWriterHarness for LocalWriterHarness {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::Local
    }

    async fn run(
        &self,
        task: MultiRepoTask,
        _attempt: u32,
        cancel: CancellationToken,
    ) -> Result<WriterHarnessAttempt, WriterFailure> {
        let result = self.run_inner(task, cancel).await;
        let primary_check = self.runtime.verify_primaries().await;
        match (result, primary_check) {
            (_, Err(error)) => Err(writer_failure(format!(
                "primary checkout integrity failed: {error}"
            ))),
            (result, Ok(())) => result,
        }
    }
}

impl LocalWriterHarness {
    async fn run_inner(
        &self,
        task: MultiRepoTask,
        cancel: CancellationToken,
    ) -> Result<WriterHarnessAttempt, WriterFailure> {
        validate_task(&task, &self.id, MultiRepoTaskRole::Writer).map_err(writer_failure)?;
        if cancel.is_cancelled() {
            return Err(writer_failure("writer was cancelled before start"));
        }
        self.runtime
            .verify_primaries()
            .await
            .map_err(writer_failure)?;
        let workspace = IsolatedWriterWorkspace::create(
            self.runtime.executor.as_ref(),
            &self.runtime.selection,
            task.clone(),
            &self.runtime.scratch_root,
        )
        .await
        .map_err(writer_failure)?;
        let config = isolated_provider_config(
            self.runtime.provider_config.clone(),
            &workspace.root,
            &task.model,
            true,
        )
        .map_err(writer_failure)?;
        let prompt = writer_prompt(&task, &self.runtime.plan);
        let collected = run_provider(
            self.runtime.factory.as_ref(),
            config,
            &workspace.root,
            agent_core::provider::PromptInput::text(prompt),
            self.runtime.timeout,
            cancel,
        )
        .await
        .map_err(|failure| WriterFailure {
            message: failure.message,
            usage: failure.usage,
            reusable_artifact_sha256: None,
        })?;
        let package = workspace
            .package(
                self.runtime.executor.as_ref(),
                &self.runtime.plan,
                &self.runtime.artifact_root,
                Vec::new(),
            )
            .await
            .map_err(|message| WriterFailure {
                message,
                usage: collected.usage.clone(),
                reusable_artifact_sha256: None,
            })?;
        Ok(WriterHarnessAttempt {
            package,
            usage: collected.usage,
        })
    }
}

pub struct LocalReviewHarness {
    id: String,
    runtime: LocalMultiRepoRuntime,
}

#[async_trait]
impl MultiRepoReviewHarness for LocalReviewHarness {
    fn id(&self) -> &str {
        &self.id
    }

    async fn review(
        &self,
        task: MultiRepoTask,
        packages: Vec<agent_orchestration::ChangePackageDescriptor>,
        _attempt: u32,
        cancel: CancellationToken,
    ) -> Result<ReviewHarnessAttempt, String> {
        validate_task(&task, &self.id, MultiRepoTaskRole::Reviewer)?;
        self.runtime.verify_primaries().await?;
        let workspace = FreshIntegrationWorkspace::replay(
            self.runtime.executor.as_ref(),
            &self.runtime.selection,
            &self.runtime.plan,
            &packages,
            &self.runtime.scratch_root,
        )
        .await?;
        let config = isolated_provider_config(
            self.runtime.provider_config.clone(),
            &workspace.root,
            &task.model,
            false,
        )?;
        let collected = run_provider(
            self.runtime.factory.as_ref(),
            config,
            &workspace.root,
            agent_core::provider::PromptInput::text(review_prompt(&task, &packages)),
            self.runtime.timeout,
            cancel,
        )
        .await
        .map_err(|failure| failure.message)?;
        self.runtime.verify_primaries().await?;
        let report = parse_review(&collected.final_message)?;
        Ok(ReviewHarnessAttempt {
            receipt: ReviewReceipt {
                reviewer_task_id: task.id,
                package_sha256: packages
                    .iter()
                    .map(|package| package.patch_sha256.clone())
                    .collect(),
                decision: report.decision,
                rework_task_ids: report.rework_task_ids,
                findings: report.findings,
            },
            usage: collected.usage,
        })
    }
}

pub struct LocalIntegrationHarness {
    id: String,
    runtime: LocalMultiRepoRuntime,
}

#[async_trait]
impl MultiRepoIntegrationHarness for LocalIntegrationHarness {
    fn id(&self) -> &str {
        &self.id
    }

    async fn integrate(
        &self,
        task: MultiRepoTask,
        packages: Vec<agent_orchestration::ChangePackageDescriptor>,
        cancel: CancellationToken,
    ) -> Result<IntegrationHarnessAttempt, String> {
        validate_task(&task, &self.id, MultiRepoTaskRole::Integrator)?;
        if cancel.is_cancelled() {
            return Err("integration was cancelled before fresh replay".into());
        }
        if let Some(gate) = &self.runtime.integration_gate {
            gate.wait_ready(cancel.child_token()).await?;
        }
        self.runtime.verify_primaries().await?;
        let workspace = FreshIntegrationWorkspace::replay(
            self.runtime.executor.as_ref(),
            &self.runtime.selection,
            &self.runtime.plan,
            &packages,
            &self.runtime.scratch_root,
        )
        .await?;
        let check_receipts = run_integration_checks(
            self.runtime.executor.as_ref(),
            &self.runtime.plan,
            &workspace,
            &cancel,
        )
        .await;
        let mut receipt = workspace.receipt().clone();
        receipt
            .checks_run
            .extend(check_receipts.iter().map(|check| check.id.clone()));
        receipt.passed &= check_receipts.iter().all(|check| check.passed);
        receipt.check_receipts = check_receipts;
        self.runtime.verify_primaries().await?;
        Ok(IntegrationHarnessAttempt {
            receipt,
            usage: UsageCharge::default(),
        })
    }
}

fn validate_runtime_root(
    root: &Path,
    selection: &RepositorySelection,
    label: &str,
) -> Result<(), String> {
    if !root.is_absolute() {
        return Err(format!("multi-repository {label} root must be absolute"));
    }
    for selected in selection.repositories().values() {
        let primary = Path::new(&selected.baseline.checkout_root);
        if root.starts_with(primary) || primary.starts_with(root) {
            return Err(format!(
                "multi-repository {label} root overlaps a primary checkout"
            ));
        }
    }
    Ok(())
}

fn validate_task(
    task: &MultiRepoTask,
    harness: &str,
    role: MultiRepoTaskRole,
) -> Result<(), String> {
    if task.role != role || task.harness != harness || task.harness_kind != HarnessKind::Local {
        return Err(format!(
            "task {} is not leased to local harness {harness}",
            task.id
        ));
    }
    Ok(())
}

fn writer_failure(message: impl Into<String>) -> WriterFailure {
    WriterFailure {
        message: message.into(),
        usage: UsageCharge::default(),
        reusable_artifact_sha256: None,
    }
}

fn reader_failure(message: impl Into<String>) -> ReaderFailure {
    ReaderFailure {
        message: message.into(),
        usage: UsageCharge::default(),
    }
}

#[cfg(test)]
#[path = "multi_repo_provider_tests.rs"]
mod tests;
