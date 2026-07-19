use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agent_orchestration::{
    ChangePackageDescriptor, MultiRepoPlan, MultiRepoTaskRole, RepositoryId, TaskId,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::git::{git_shell, sha256, working_paths, working_state_sha256};
use super::RepositorySelection;
use crate::exec::Executor;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimaryApplicationReceipt {
    pub repository_ids: Vec<RepositoryId>,
    pub task_ids: Vec<TaskId>,
    pub patch_sha256: Vec<String>,
    pub changed_paths: BTreeMap<RepositoryId, BTreeSet<String>>,
    pub before_state_sha256: BTreeMap<RepositoryId, String>,
    pub after_state_sha256: BTreeMap<RepositoryId, String>,
    pub head_unchanged: bool,
    pub preexisting_changes_preserved: bool,
}

struct PreparedApplication {
    repository_id: RepositoryId,
    root: PathBuf,
    patch_path: PathBuf,
    changed_paths: BTreeSet<String>,
    patch_sha256: Vec<String>,
}

impl RepositorySelection {
    pub fn validate_delegated_scope(&self, plan: &MultiRepoPlan) -> Result<(), String> {
        plan.validate()?;
        for (repository_id, selected) in &self.repositories {
            let delegated_paths = &plan.repositories[repository_id].allowed_changed_paths;
            if !selected.primary_changed_paths.is_disjoint(delegated_paths) {
                return Err(format!(
                    "delegated paths overlap pre-existing user changes in repository {repository_id}"
                ));
            }
        }
        Ok(())
    }

    /// Apply a coordinator-validated package set to the selected primary
    /// checkouts after a full no-write preflight. Existing dirty paths may be
    /// present, but delegated writers cannot lease or touch them.
    pub async fn apply_verified_packages(
        &self,
        executor: &dyn Executor,
        plan: &MultiRepoPlan,
        packages: &[ChangePackageDescriptor],
        scratch_root: &Path,
    ) -> Result<PrimaryApplicationReceipt, String> {
        plan.validate()?;
        self.verify_primaries_unchanged(executor).await?;
        validate_scratch_root(self, scratch_root)?;

        let expected_tasks = plan
            .tasks
            .iter()
            .filter(|task| task.role == MultiRepoTaskRole::Writer)
            .map(|task| task.id.clone())
            .collect::<BTreeSet<_>>();
        let actual_tasks = packages
            .iter()
            .map(|package| package.task_id.clone())
            .collect::<BTreeSet<_>>();
        if actual_tasks != expected_tasks || packages.len() != expected_tasks.len() {
            return Err("primary apply requires exactly one verified package per writer".into());
        }

        let apply_root = scratch_root
            .join("primary-apply")
            .join(Uuid::new_v4().to_string());
        executor.create_dir_all(&apply_root).await?;
        let prepared = match self
            .prepare_applications(executor, plan, packages, &apply_root)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = executor.remove_dir_all(&apply_root).await;
                return Err(error);
            }
        };

        for application in &prepared {
            let patch_arg =
                crate::git_metadata::shell_word(&application.patch_path.to_string_lossy());
            if let Err(error) = git_shell(
                executor,
                &application.root,
                &format!("apply --check --whitespace=nowarn -- {patch_arg}"),
            )
            .await
            {
                let _ = executor.remove_dir_all(&apply_root).await;
                return Err(format!(
                    "primary apply preflight failed for {}: {error}",
                    application.repository_id
                ));
            }
        }
        self.verify_primaries_unchanged(executor).await?;

        let mut applied = Vec::<&PreparedApplication>::new();
        for application in &prepared {
            let patch_arg =
                crate::git_metadata::shell_word(&application.patch_path.to_string_lossy());
            if let Err(error) = git_shell(
                executor,
                &application.root,
                &format!("apply --whitespace=nowarn -- {patch_arg}"),
            )
            .await
            {
                let rollback = rollback(executor, &applied).await;
                let _ = executor.remove_dir_all(&apply_root).await;
                return Err(match rollback {
                    Ok(()) => format!(
                        "primary apply failed for {}; earlier applications were rolled back: {error}",
                        application.repository_id
                    ),
                    Err(rollback_error) => format!(
                        "primary apply failed for {} and rollback also failed: {error}; {rollback_error}",
                        application.repository_id
                    ),
                });
            }
            applied.push(application);
        }

        match self
            .application_receipt(executor, packages, &prepared)
            .await
        {
            Ok(receipt) => {
                let _ = executor.remove_dir_all(&apply_root).await;
                Ok(receipt)
            }
            Err(error) => {
                let rollback = rollback(executor, &applied).await;
                let _ = executor.remove_dir_all(&apply_root).await;
                Err(match rollback {
                    Ok(()) => format!(
                        "primary application verification failed and changes were rolled back: {error}"
                    ),
                    Err(rollback_error) => format!(
                        "primary application verification failed and rollback also failed: {error}; {rollback_error}"
                    ),
                })
            }
        }
    }

    async fn prepare_applications(
        &self,
        executor: &dyn Executor,
        plan: &MultiRepoPlan,
        packages: &[ChangePackageDescriptor],
        apply_root: &Path,
    ) -> Result<Vec<PreparedApplication>, String> {
        let mut prepared = Vec::new();
        for (repository_id, selected) in &self.repositories {
            let mut repository_packages = packages
                .iter()
                .filter(|package| &package.repository_id == repository_id)
                .collect::<Vec<_>>();
            if repository_packages.is_empty() {
                continue;
            }
            repository_packages.sort_by(|left, right| left.task_id.cmp(&right.task_id));
            let mut patch = Vec::new();
            let mut changed_paths = BTreeSet::new();
            let mut patch_sha256 = Vec::new();
            for package in repository_packages {
                plan.validate_change_package(package)?;
                if !changed_paths.is_disjoint(&package.changed_paths) {
                    return Err(format!(
                        "verified packages overlap in repository {repository_id}"
                    ));
                }
                if !selected
                    .primary_changed_paths
                    .is_disjoint(&package.changed_paths)
                {
                    return Err(format!(
                        "delegated changes overlap pre-existing user changes in repository {repository_id}"
                    ));
                }
                let bytes = executor.read(Path::new(&package.artifact_path)).await?;
                if sha256(&bytes) != package.patch_sha256 {
                    return Err(format!("patch digest mismatch for {repository_id}"));
                }
                patch.extend_from_slice(&bytes);
                if !patch.ends_with(b"\n") {
                    patch.push(b'\n');
                }
                changed_paths.extend(package.changed_paths.iter().cloned());
                patch_sha256.push(package.patch_sha256.clone());
            }
            let patch_path = apply_root.join(format!("{repository_id}.patch"));
            executor.write(&patch_path, &patch).await?;
            prepared.push(PreparedApplication {
                repository_id: repository_id.clone(),
                root: PathBuf::from(&selected.baseline.checkout_root),
                patch_path,
                changed_paths,
                patch_sha256,
            });
        }
        Ok(prepared)
    }

    async fn application_receipt(
        &self,
        executor: &dyn Executor,
        packages: &[ChangePackageDescriptor],
        prepared: &[PreparedApplication],
    ) -> Result<PrimaryApplicationReceipt, String> {
        let mut before_state_sha256 = BTreeMap::new();
        let mut after_state_sha256 = BTreeMap::new();
        let mut changed_paths = BTreeMap::new();
        for application in prepared {
            let selected = &self.repositories[&application.repository_id];
            let current_paths = working_paths(executor, &application.root).await?;
            let expected_paths = selected
                .primary_changed_paths
                .union(&application.changed_paths)
                .cloned()
                .collect::<BTreeSet<_>>();
            if current_paths != expected_paths {
                return Err(format!(
                    "primary checkout changed unexpectedly while applying repository {}",
                    application.repository_id
                ));
            }
            let head = super::git::git_text(
                executor,
                &application.root,
                &["rev-parse", "--verify", "HEAD"],
            )
            .await?;
            if head != selected.baseline.head_oid {
                return Err(format!(
                    "primary checkout HEAD moved while applying repository {}",
                    application.repository_id
                ));
            }
            before_state_sha256.insert(
                application.repository_id.clone(),
                selected.primary_state_sha256.clone(),
            );
            after_state_sha256.insert(
                application.repository_id.clone(),
                working_state_sha256(executor, &application.root).await?,
            );
            changed_paths.insert(
                application.repository_id.clone(),
                application.changed_paths.clone(),
            );
        }
        Ok(PrimaryApplicationReceipt {
            repository_ids: prepared
                .iter()
                .map(|application| application.repository_id.clone())
                .collect(),
            task_ids: packages
                .iter()
                .map(|package| package.task_id.clone())
                .collect(),
            patch_sha256: prepared
                .iter()
                .flat_map(|application| application.patch_sha256.iter().cloned())
                .collect(),
            changed_paths,
            before_state_sha256,
            after_state_sha256,
            head_unchanged: true,
            preexisting_changes_preserved: true,
        })
    }
}

async fn rollback(executor: &dyn Executor, applied: &[&PreparedApplication]) -> Result<(), String> {
    for application in applied.iter().rev() {
        let patch_arg = crate::git_metadata::shell_word(&application.patch_path.to_string_lossy());
        git_shell(
            executor,
            &application.root,
            &format!("apply -R --whitespace=nowarn -- {patch_arg}"),
        )
        .await?;
    }
    Ok(())
}

fn validate_scratch_root(
    selection: &RepositorySelection,
    scratch_root: &Path,
) -> Result<(), String> {
    if !scratch_root.is_absolute() {
        return Err("primary application scratch root must be absolute".into());
    }
    for selected in selection.repositories.values() {
        let primary = Path::new(&selected.baseline.checkout_root);
        if scratch_root.starts_with(primary) || primary.starts_with(scratch_root) {
            return Err("primary application scratch root overlaps a selected checkout".into());
        }
    }
    Ok(())
}
