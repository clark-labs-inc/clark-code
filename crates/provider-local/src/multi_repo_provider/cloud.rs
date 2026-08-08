use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_core::domain::PendingUpload;
use agent_core::provider::{PromptInput, ProviderConfig};
use agent_orchestration::{
    HarnessKind, MultiRepoPlan, MultiRepoTask, MultiRepoTaskRole, MultiRepoWriterHarness,
    ProviderFactory, WriterFailure, WriterHarnessAttempt,
};
use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::events::run_provider;
use super::{validate_runtime_root, writer_failure};
use crate::{Executor, IsolatedWriterWorkspace, RepositorySelection, SelectedRepository};

#[derive(Clone)]
pub struct BrokeredCloudWriterConfig {
    pub id: String,
    pub provider_config: ProviderConfig,
    pub timeout: Duration,
    pub scratch_root: PathBuf,
    pub artifact_root: PathBuf,
    pub max_upload_bytes: usize,
}

pub struct BrokeredCloudWriterHarness {
    config: BrokeredCloudWriterConfig,
    selection: Arc<RepositorySelection>,
    plan: Arc<MultiRepoPlan>,
    factory: Arc<dyn ProviderFactory>,
    executor: Arc<dyn Executor>,
}

impl BrokeredCloudWriterHarness {
    pub fn new(
        config: BrokeredCloudWriterConfig,
        selection: Arc<RepositorySelection>,
        plan: Arc<MultiRepoPlan>,
        factory: Arc<dyn ProviderFactory>,
        executor: Arc<dyn Executor>,
    ) -> Result<Self, String> {
        plan.validate()?;
        if config.id.trim().is_empty() || config.timeout.is_zero() || config.max_upload_bytes == 0 {
            return Err("brokered cloud writer configuration is incomplete".into());
        }
        validate_runtime_root(&config.scratch_root, &selection, "scratch")?;
        validate_runtime_root(&config.artifact_root, &selection, "artifact")?;
        let tasks = plan
            .tasks
            .iter()
            .filter(|task| task.role == MultiRepoTaskRole::Writer && task.harness == config.id)
            .collect::<Vec<_>>();
        if tasks.is_empty()
            || tasks
                .iter()
                .any(|task| task.harness_kind != HarnessKind::BrokeredCloud)
        {
            return Err("plan has no brokered cloud writer for this harness".into());
        }
        for task in tasks {
            let repository = &plan.repositories[task.repository_id.as_ref().unwrap()];
            if !repository.cloud_eligible {
                return Err(format!(
                    "repository {} has no brokered cloud consent",
                    repository.repository_id
                ));
            }
        }
        Ok(Self {
            config,
            selection,
            plan,
            factory,
            executor,
        })
    }

    async fn run_inner(
        &self,
        task: MultiRepoTask,
        cancel: CancellationToken,
    ) -> Result<WriterHarnessAttempt, WriterFailure> {
        if task.role != MultiRepoTaskRole::Writer
            || task.harness != self.config.id
            || task.harness_kind != HarnessKind::BrokeredCloud
        {
            return Err(writer_failure(
                "task is not leased to this brokered cloud writer",
            ));
        }
        let repository = &self.plan.repositories[task.repository_id.as_ref().unwrap()];
        if !repository.cloud_eligible {
            return Err(writer_failure(
                "brokered cloud repository consent is absent",
            ));
        }
        self.selection
            .verify_primaries_unchanged(self.executor.as_ref())
            .await
            .map_err(writer_failure)?;
        let workspace = IsolatedWriterWorkspace::create(
            self.executor.as_ref(),
            &self.selection,
            task.clone(),
            &self.config.scratch_root,
        )
        .await
        .map_err(writer_failure)?;
        let bundle = source_bundle(
            self.executor.as_ref(),
            &workspace,
            self.selection.repositories()[task.repository_id.as_ref().unwrap()].clone(),
            self.config.max_upload_bytes,
        )
        .await
        .map_err(writer_failure)?;
        let bundle_json =
            serde_json::to_vec(&bundle).map_err(|error| writer_failure(error.to_string()))?;
        let mut provider_config = self.config.provider_config.clone();
        let mut extra = match provider_config.extra {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            _ => {
                return Err(writer_failure(
                    "managed provider extra config must be an object",
                ))
            }
        };
        extra.insert("tier_id".into(), Value::String(task.model.clone()));
        provider_config.extra = Value::Object(extra);
        provider_config.cwd = None;
        let input = PromptInput {
            blocks: vec![agent_core::domain::ContentBlock::text(cloud_prompt(
                &task, &self.plan,
            ))],
            attachments: vec![PendingUpload {
                filename: "repository-lease.json".into(),
                content_type: "application/json".into(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(bundle_json),
            }],
        };
        let collected = run_provider(
            self.factory.as_ref(),
            provider_config,
            &workspace.root,
            input,
            self.config.timeout,
            cancel,
        )
        .await
        .map_err(|failure| WriterFailure {
            message: failure.message,
            usage: failure.usage,
            reusable_artifact_sha256: None,
        })?;
        let patch = decode_patch(&collected.final_message).map_err(|message| WriterFailure {
            message,
            usage: collected.usage.clone(),
            reusable_artifact_sha256: None,
        })?;
        workspace
            .apply_candidate_patch(self.executor.as_ref(), &patch)
            .await
            .map_err(|message| WriterFailure {
                message,
                usage: collected.usage.clone(),
                reusable_artifact_sha256: None,
            })?;
        let package = workspace
            .package(
                self.executor.as_ref(),
                &self.plan,
                &self.config.artifact_root,
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

#[async_trait]
impl MultiRepoWriterHarness for BrokeredCloudWriterHarness {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::BrokeredCloud
    }

    async fn run(
        &self,
        task: MultiRepoTask,
        _attempt: u32,
        cancel: CancellationToken,
    ) -> Result<WriterHarnessAttempt, WriterFailure> {
        let result = self.run_inner(task, cancel).await;
        let primary = self
            .selection
            .verify_primaries_unchanged(self.executor.as_ref())
            .await;
        match (result, primary) {
            (_, Err(error)) => Err(writer_failure(format!(
                "primary checkout integrity failed: {error}"
            ))),
            (result, Ok(())) => result,
        }
    }
}

#[derive(Serialize)]
struct CloudSourceBundle {
    schema_version: u32,
    repository_id: String,
    base_head_oid: String,
    files: Vec<CloudSourceFile>,
}

#[derive(Serialize)]
struct CloudSourceFile {
    path: String,
    content: Option<String>,
}

async fn source_bundle(
    executor: &dyn Executor,
    workspace: &IsolatedWriterWorkspace,
    selected: SelectedRepository,
    max_bytes: usize,
) -> Result<CloudSourceBundle, String> {
    let mut files = Vec::new();
    let mut bytes = 0usize;
    for path in &workspace.task.allowed_changed_paths {
        let absolute = workspace.root.join(path);
        let content = match executor.read(&absolute).await {
            Ok(contents) => {
                bytes = bytes.saturating_add(contents.len());
                if bytes > max_bytes {
                    return Err(format!(
                        "brokered cloud source lease exceeds {max_bytes} bytes"
                    ));
                }
                Some(
                    String::from_utf8(contents)
                        .map_err(|_| format!("brokered cloud source path is not UTF-8: {path}"))?,
                )
            }
            Err(_) => None,
        };
        files.push(CloudSourceFile {
            path: path.clone(),
            content,
        });
    }
    Ok(CloudSourceBundle {
        schema_version: 1,
        repository_id: selected.baseline.repository_id.to_string(),
        base_head_oid: selected.baseline.head_oid,
        files,
    })
}

fn cloud_prompt(task: &MultiRepoTask, plan: &MultiRepoPlan) -> String {
    let decisions = plan
        .contract_decisions
        .iter()
        .map(|decision| format!("{}: {}", decision.edge_id, decision.compatibility_rule))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are a brokered cloud repository writer. The attached JSON is the complete, explicitly consented source lease.\n\
         Implement only this task: {}\nAllowed paths: {:?}\nCross-repository decisions:\n{}\n\
         Return exactly one JSON object and no trailing prose: {{\"patch_base64\":\"base64 encoded git unified diff\"}}.\n\
         The patch must be relative, must not touch .git, and must change only allowed paths.",
        task.objective, task.allowed_changed_paths, decisions
    )
}

#[derive(Deserialize)]
struct CloudPatch {
    patch_base64: String,
}

fn decode_patch(text: &str) -> Result<Vec<u8>, String> {
    let trimmed = text.trim();
    let report = serde_json::from_str::<CloudPatch>(trimmed)
        .ok()
        .or_else(|| {
            let end = trimmed.rfind('}')?;
            trimmed[..=end]
                .char_indices()
                .rev()
                .filter(|(_, character)| *character == '{')
                .find_map(|(start, _)| serde_json::from_str(&trimmed[start..=end]).ok())
        });
    let report = report.ok_or_else(|| {
        "brokered cloud writer did not return the required patch receipt".to_string()
    })?;
    let patch = base64::engine::general_purpose::STANDARD
        .decode(report.patch_base64)
        .map_err(|error| format!("brokered cloud patch base64 is invalid: {error}"))?;
    if patch.is_empty() {
        return Err("brokered cloud patch is empty".into());
    }
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use agent_core::Provider;
    use agent_orchestration::{IsolationKind, MultiRepoTaskRole, TaskId};

    use super::*;
    use crate::multi_repo_provider::tests::{plan, selected, FakeProvider, FakeState};
    use crate::LocalExecutor;

    fn cloud_plan(
        selection: &RepositorySelection,
        repository: &str,
    ) -> (Arc<MultiRepoPlan>, MultiRepoTask) {
        let mut plan = (*plan(selection)).clone();
        let task = plan
            .tasks
            .iter_mut()
            .find(|task| {
                task.role == MultiRepoTaskRole::Writer
                    && task
                        .repository_id
                        .as_ref()
                        .is_some_and(|id| id.as_str() == repository)
            })
            .unwrap();
        task.harness = "brokered-cloud".into();
        task.harness_kind = HarnessKind::BrokeredCloud;
        task.model = "local-model".into();
        let task = task.clone();
        (Arc::new(plan), task)
    }

    #[tokio::test]
    async fn consented_cloud_patch_is_uploaded_minimally_and_validated_locally() {
        let temp = tempfile::tempdir().unwrap();
        let selection = selected(&temp).await;
        let (plan, task) = cloud_plan(&selection, "sdk");
        let shared = Arc::new(FakeState::default());
        let factory_state = shared.clone();
        let harness = BrokeredCloudWriterHarness::new(
            BrokeredCloudWriterConfig {
                id: "brokered-cloud".into(),
                provider_config: ProviderConfig::default(),
                timeout: Duration::from_secs(2),
                scratch_root: temp.path().join("cloud-scratch"),
                artifact_root: temp.path().join("cloud-artifacts"),
                max_upload_bytes: 10_000,
            },
            selection.clone(),
            plan,
            Arc::new(move || {
                Box::new(FakeProvider {
                    shared: factory_state.clone(),
                    cwd: None,
                }) as Box<dyn Provider>
            }),
            Arc::new(LocalExecutor),
        )
        .unwrap();
        let result = harness
            .run(task, 1, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.package.isolation, IsolationKind::CloudEphemeralClone);
        assert_eq!(result.package.task_id, TaskId::new("sdk-writer").unwrap());
        assert_eq!(shared.cloud_attachments.load(Ordering::SeqCst), 1);
        selection
            .verify_primaries_unchanged(&LocalExecutor)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cloud_harness_refuses_a_repository_without_consent() {
        let temp = tempfile::tempdir().unwrap();
        let selection = selected(&temp).await;
        let (plan, _) = cloud_plan(&selection, "api");
        let error = BrokeredCloudWriterHarness::new(
            BrokeredCloudWriterConfig {
                id: "brokered-cloud".into(),
                provider_config: ProviderConfig::default(),
                timeout: Duration::from_secs(1),
                scratch_root: temp.path().join("cloud-scratch"),
                artifact_root: temp.path().join("cloud-artifacts"),
                max_upload_bytes: 1,
            },
            selection,
            plan,
            Arc::new(|| -> Box<dyn Provider> {
                panic!("provider must not be constructed when consent is absent")
            }),
            Arc::new(LocalExecutor),
        )
        .err()
        .unwrap();
        assert!(error.contains("cloud consent"), "{error}");
    }
}
