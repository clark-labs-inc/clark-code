use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agent_orchestration::{
    ChangePackageDescriptor, IntegrationReceipt, IsolationKind, MultiRepoPlan, MultiRepoTask,
    MultiRepoTaskRole, RepositoryBaseline, RepositoryId,
};
use uuid::Uuid;

use crate::exec::Executor;
use crate::repository::inspect_repository;

#[path = "multi_repo_workspace/git.rs"]
mod git;
use git::{
    changed_tree_sha256, checkout_kind, clone_at_baseline, git_bytes, git_shell, git_text,
    parse_nul_paths, sha256, validate_relative_path, working_paths, working_state_sha256,
};

#[path = "multi_repo_workspace/application.rs"]
mod application;
pub use application::PrimaryApplicationReceipt;

#[derive(Clone, Debug)]
pub struct RepositorySelectionRequest {
    pub repository_id: RepositoryId,
    pub root: PathBuf,
    pub allowed_changed_paths: BTreeSet<String>,
    pub cloud_eligible: bool,
}

#[derive(Clone, Debug)]
pub struct SelectedRepository {
    pub baseline: RepositoryBaseline,
    primary_state_sha256: String,
    primary_changed_paths: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct RepositorySelection {
    repositories: BTreeMap<RepositoryId, SelectedRepository>,
}

impl RepositorySelection {
    #[cfg(test)]
    pub(crate) fn from_test_baselines(
        baselines: BTreeMap<RepositoryId, RepositoryBaseline>,
    ) -> Self {
        Self {
            repositories: baselines
                .into_iter()
                .map(|(id, baseline)| {
                    (
                        id,
                        SelectedRepository {
                            primary_state_sha256: baseline.dirty_tree_sha256.clone(),
                            baseline,
                            primary_changed_paths: BTreeSet::new(),
                        },
                    )
                })
                .collect(),
        }
    }

    pub async fn resolve(
        executor: &dyn Executor,
        requests: Vec<RepositorySelectionRequest>,
    ) -> Result<Self, String> {
        if requests.is_empty() {
            return Err("repository selection requires at least one explicit root".into());
        }
        let mut repositories = BTreeMap::new();
        let mut fingerprints = BTreeSet::new();
        let mut roots = Vec::<PathBuf>::new();
        for request in requests {
            for path in &request.allowed_changed_paths {
                validate_relative_path(path)?;
            }
            let identity = inspect_repository(executor, &request.root)
                .await?
                .ok_or_else(|| {
                    format!(
                        "selected root is not a Git repository: {}",
                        request.root.display()
                    )
                })?;
            let root = PathBuf::from(&identity.root);
            if !root.is_absolute() {
                return Err(format!(
                    "Git returned a non-absolute checkout root: {}",
                    root.display()
                ));
            }
            if roots
                .iter()
                .any(|existing| root.starts_with(existing) || existing.starts_with(&root))
            {
                return Err("selected repository checkout roots overlap".into());
            }
            if !fingerprints.insert(identity.fingerprint.clone()) {
                return Err(
                    "multiple selected checkouts resolve to the same repository identity".into(),
                );
            }
            let head_oid = identity
                .head_oid
                .ok_or_else(|| format!("selected repository has no HEAD: {}", root.display()))?;
            let checkout_kind =
                checkout_kind(executor, &root, identity.current_branch.as_deref()).await?;
            let primary_state_sha256 = working_state_sha256(executor, &root).await?;
            let primary_changed_paths = working_paths(executor, &root).await?;
            let baseline = RepositoryBaseline {
                repository_id: request.repository_id.clone(),
                repository_fingerprint: identity.fingerprint,
                checkout_root: identity.root,
                checkout_kind,
                head_oid,
                current_branch: identity.current_branch,
                dirty_tree_sha256: primary_state_sha256.clone(),
                allowed_changed_paths: request.allowed_changed_paths,
                cloud_eligible: request.cloud_eligible,
            };
            roots.push(root);
            if repositories
                .insert(
                    request.repository_id,
                    SelectedRepository {
                        baseline,
                        primary_state_sha256,
                        primary_changed_paths,
                    },
                )
                .is_some()
            {
                return Err("duplicate repository id in selection".into());
            }
        }
        Ok(Self { repositories })
    }

    pub fn repositories(&self) -> &BTreeMap<RepositoryId, SelectedRepository> {
        &self.repositories
    }

    pub fn baselines(&self) -> BTreeMap<RepositoryId, RepositoryBaseline> {
        self.repositories
            .iter()
            .map(|(id, selected)| (id.clone(), selected.baseline.clone()))
            .collect()
    }

    pub async fn verify_primaries_unchanged(&self, executor: &dyn Executor) -> Result<(), String> {
        for selected in self.repositories.values() {
            let root = Path::new(&selected.baseline.checkout_root);
            let head = git_text(executor, root, &["rev-parse", "--verify", "HEAD"]).await?;
            if head.trim() != selected.baseline.head_oid {
                return Err(format!(
                    "primary checkout HEAD moved for {}",
                    selected.baseline.repository_id
                ));
            }
            let state = working_state_sha256(executor, root).await?;
            if state != selected.primary_state_sha256 {
                return Err(format!(
                    "primary checkout changed for {}",
                    selected.baseline.repository_id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct IsolatedWriterWorkspace {
    pub task: MultiRepoTask,
    pub root: PathBuf,
    baseline: RepositoryBaseline,
}

#[derive(Clone, Debug)]
pub struct IsolatedReaderWorkspace {
    pub task: MultiRepoTask,
    pub root: PathBuf,
}

impl IsolatedReaderWorkspace {
    pub async fn create(
        executor: &dyn Executor,
        selection: &RepositorySelection,
        task: MultiRepoTask,
        scratch_root: &Path,
    ) -> Result<Self, String> {
        if task.role != MultiRepoTaskRole::Reader || !task.allowed_changed_paths.is_empty() {
            return Err("isolated reader workspaces require a read-only reader task".into());
        }
        let repository_id = task
            .repository_id
            .as_ref()
            .ok_or_else(|| "reader task has no repository id".to_string())?;
        let selected = selection
            .repositories
            .get(repository_id)
            .ok_or_else(|| "reader task repository was not selected".to_string())?;
        if !scratch_root.is_absolute() {
            return Err("orchestration scratch root must be absolute".into());
        }
        let source = Path::new(&selected.baseline.checkout_root);
        if scratch_root.starts_with(source) || source.starts_with(scratch_root) {
            return Err(
                "orchestration scratch root must be disjoint from selected checkouts".into(),
            );
        }
        executor.create_dir_all(scratch_root).await?;
        let root = scratch_root
            .join("readers")
            .join(format!("{}-{}", task.id, Uuid::new_v4()));
        clone_at_baseline(executor, &selected.baseline, &root, scratch_root).await?;
        Ok(Self { task, root })
    }
}

impl IsolatedWriterWorkspace {
    pub async fn create(
        executor: &dyn Executor,
        selection: &RepositorySelection,
        task: MultiRepoTask,
        scratch_root: &Path,
    ) -> Result<Self, String> {
        if task.role != MultiRepoTaskRole::Writer {
            return Err("isolated writer workspaces require a writer task".into());
        }
        let repository_id = task
            .repository_id
            .as_ref()
            .ok_or_else(|| "writer task has no repository id".to_string())?;
        let selected = selection
            .repositories
            .get(repository_id)
            .ok_or_else(|| "writer task repository was not selected".to_string())?;
        if !task
            .allowed_changed_paths
            .is_subset(&selected.baseline.allowed_changed_paths)
        {
            return Err("writer task exceeds the selected repository scope".into());
        }
        if !scratch_root.is_absolute() {
            return Err("orchestration scratch root must be absolute".into());
        }
        let source = Path::new(&selected.baseline.checkout_root);
        if scratch_root.starts_with(source) || source.starts_with(scratch_root) {
            return Err(
                "orchestration scratch root must be disjoint from selected checkouts".into(),
            );
        }
        executor.create_dir_all(scratch_root).await?;
        let root = scratch_root
            .join("writers")
            .join(format!("{}-{}", task.id, Uuid::new_v4()));
        if executor.metadata(&root).await.is_ok() {
            return Err("isolated writer destination already exists".into());
        }
        clone_at_baseline(executor, &selected.baseline, &root, scratch_root).await?;
        let actual = git_text(executor, &root, &["rev-parse", "--verify", "HEAD"]).await?;
        if actual.trim() != selected.baseline.head_oid {
            return Err("isolated writer clone did not checkout the pinned baseline".into());
        }
        Ok(Self {
            task,
            root,
            baseline: selected.baseline.clone(),
        })
    }

    pub async fn package(
        &self,
        executor: &dyn Executor,
        plan: &MultiRepoPlan,
        artifact_root: &Path,
        checks_run: Vec<String>,
    ) -> Result<ChangePackageDescriptor, String> {
        let head = git_text(executor, &self.root, &["rev-parse", "--verify", "HEAD"]).await?;
        if head.trim() != self.baseline.head_oid {
            return Err("writer moved the isolated checkout baseline".into());
        }
        git_shell(executor, &self.root, "add --intent-to-add --all -- .").await?;
        let changed_raw = git_bytes(
            executor,
            &self.root,
            &["diff", "--name-only", "-z", "HEAD", "--"],
        )
        .await?;
        let changed_paths = parse_nul_paths(&changed_raw)?;
        if changed_paths.is_empty() {
            return Err("writer produced no repository changes".into());
        }
        if !changed_paths.is_subset(&self.task.allowed_changed_paths) {
            return Err(format!(
                "writer changed paths outside its lease: {:?}",
                changed_paths
                    .difference(&self.task.allowed_changed_paths)
                    .collect::<Vec<_>>()
            ));
        }
        let patch = git_bytes(
            executor,
            &self.root,
            &["diff", "--binary", "--no-ext-diff", "HEAD", "--"],
        )
        .await?;
        if patch.is_empty() {
            return Err("writer change package patch is empty".into());
        }
        let patch_sha256 = sha256(&patch);
        let result_tree_sha256 = changed_tree_sha256(
            executor,
            &self.root,
            &self.baseline.head_oid,
            &patch_sha256,
            &changed_paths,
        )
        .await?;
        executor.create_dir_all(artifact_root).await?;
        let artifact_path = artifact_root.join(format!("{patch_sha256}.patch"));
        executor.write(&artifact_path, &patch).await?;
        let isolation = if self.task.harness_kind == agent_orchestration::HarnessKind::ClarkCloud {
            IsolationKind::CloudEphemeralClone
        } else {
            IsolationKind::LocalEphemeralClone
        };
        let descriptor = ChangePackageDescriptor {
            task_id: self.task.id.clone(),
            repository_id: self.baseline.repository_id.clone(),
            base_head_oid: self.baseline.head_oid.clone(),
            changed_paths,
            patch_sha256,
            result_tree_sha256,
            artifact_path: artifact_path.to_string_lossy().into_owned(),
            isolation,
            checks_run,
        };
        plan.validate_change_package(&descriptor)?;
        Ok(descriptor)
    }

    pub async fn apply_candidate_patch(
        &self,
        executor: &dyn Executor,
        patch: &[u8],
    ) -> Result<(), String> {
        if patch.is_empty() {
            return Err("candidate patch is empty".into());
        }
        let parent = self
            .root
            .parent()
            .ok_or_else(|| "isolated workspace has no scratch parent".to_string())?;
        let path = parent.join(format!("candidate-{}.patch", Uuid::new_v4()));
        executor.write(&path, patch).await?;
        let path_arg = crate::git_metadata::shell_path_word(&path);
        let check = git_shell(
            executor,
            &self.root,
            &format!("apply --check --whitespace=nowarn -- {path_arg}"),
        )
        .await;
        if let Err(error) = check {
            let _ = executor.remove_file(&path).await;
            return Err(error);
        }
        let apply = git_shell(
            executor,
            &self.root,
            &format!("apply --whitespace=nowarn -- {path_arg}"),
        )
        .await;
        let _ = executor.remove_file(&path).await;
        apply.map(|_| ())
    }
}

#[derive(Clone, Debug)]
pub struct FreshIntegrationWorkspace {
    pub root: PathBuf,
    pub repository_roots: BTreeMap<RepositoryId, PathBuf>,
    receipt: IntegrationReceipt,
}

impl FreshIntegrationWorkspace {
    pub async fn replay(
        executor: &dyn Executor,
        selection: &RepositorySelection,
        plan: &MultiRepoPlan,
        packages: &[ChangePackageDescriptor],
        scratch_root: &Path,
    ) -> Result<Self, String> {
        if !scratch_root.is_absolute() {
            return Err("integration scratch root must be absolute".into());
        }
        let root = scratch_root
            .join("integration")
            .join(Uuid::new_v4().to_string());
        executor.create_dir_all(&root).await?;
        let mut repository_roots = BTreeMap::new();
        let mut repository_result_trees = BTreeMap::new();
        let mut applied_patch_sha256 = Vec::new();
        for (repository_id, selected) in &selection.repositories {
            let destination = root.join(repository_id.as_str());
            clone_at_baseline(executor, &selected.baseline, &destination, &root).await?;
            let mut repository_packages = packages
                .iter()
                .filter(|package| &package.repository_id == repository_id)
                .collect::<Vec<_>>();
            repository_packages.sort_by(|left, right| left.task_id.cmp(&right.task_id));
            for package in &repository_packages {
                plan.validate_change_package(package)?;
                let patch = executor.read(Path::new(&package.artifact_path)).await?;
                if sha256(&patch) != package.patch_sha256 {
                    return Err(format!("patch digest mismatch for {repository_id}"));
                }
                let artifact_arg =
                    crate::git_metadata::shell_path_word(Path::new(&package.artifact_path));
                git_shell(
                    executor,
                    &destination,
                    &format!("apply --whitespace=nowarn -- {artifact_arg}"),
                )
                .await?;
                let result_tree = changed_tree_sha256(
                    executor,
                    &destination,
                    &package.base_head_oid,
                    &package.patch_sha256,
                    &package.changed_paths,
                )
                .await?;
                if result_tree != package.result_tree_sha256 {
                    return Err(format!("result tree mismatch for {repository_id}"));
                }
                applied_patch_sha256.push(package.patch_sha256.clone());
            }
            if let Some(result_tree) = agent_orchestration::repository_result_tree_sha256(
                repository_packages.iter().copied(),
            ) {
                repository_result_trees.insert(repository_id.clone(), result_tree);
            }
            repository_roots.insert(repository_id.clone(), destination);
        }
        let receipt = IntegrationReceipt {
            fresh_workspace: true,
            repository_baselines: selection
                .repositories
                .iter()
                .map(|(id, selected)| (id.clone(), selected.baseline.head_oid.clone()))
                .collect(),
            repository_result_trees,
            applied_patch_sha256,
            checks_run: vec!["independent content-addressed patch replay".into()],
            check_receipts: Vec::new(),
            passed: true,
        };
        Ok(Self {
            root,
            repository_roots,
            receipt,
        })
    }

    pub fn receipt(&self) -> &IntegrationReceipt {
        &self.receipt
    }
}

#[cfg(test)]
#[path = "multi_repo_workspace_tests.rs"]
mod tests;
