use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use crate::{HarnessKind, TaskId};
use sha2::{Digest, Sha256};

#[path = "multi_repo/contract.rs"]
mod contract;
pub use contract::*;
#[path = "multi_repo/runtime.rs"]
mod runtime;
pub use runtime::*;

impl MultiRepoPlan {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_repositories()?;
        let tasks = self.validate_tasks()?;
        self.validate_contracts(&tasks)?;
        self.validate_completion_graph(&tasks)?;
        self.validate_integration_checks()?;
        Ok(())
    }

    pub fn decomposition_decision(&self) -> Result<DecompositionDecision, String> {
        self.validate()?;
        let task_map = self
            .tasks
            .iter()
            .map(|task| (task.id.clone(), task))
            .collect::<BTreeMap<_, _>>();
        let layers = topological_layers(&task_map)?;
        let mut writer_batches = Vec::new();
        for layer in layers {
            let writers = layer
                .into_iter()
                .filter(|id| task_map[id].role == MultiRepoTaskRole::Writer)
                .collect::<Vec<_>>();
            for chunk in writers.chunks(self.max_parallel_writers) {
                if !chunk.is_empty() {
                    writer_batches.push(chunk.to_vec());
                }
            }
        }
        let maximum_parallel = writer_batches.iter().map(Vec::len).max().unwrap_or(0);
        let delegated = maximum_parallel >= 2;
        let reasons = if delegated {
            vec![format!(
                "{maximum_parallel} repository-scoped writers are dependency-independent"
            )]
        } else {
            vec![
                "writer dependencies form a sequential chain; parallel delegation adds no value"
                    .to_string(),
            ]
        };
        Ok(DecompositionDecision {
            delegated,
            parallel_writer_batches: writer_batches,
            reasons,
        })
    }

    pub fn validate_change_package(&self, package: &ChangePackageDescriptor) -> Result<(), String> {
        let repository = self
            .repositories
            .get(&package.repository_id)
            .ok_or_else(|| "change package names an unknown repository".to_string())?;
        let writer = self
            .tasks
            .iter()
            .find(|task| task.id == package.task_id && task.role == MultiRepoTaskRole::Writer)
            .ok_or_else(|| "change package does not belong to a writer task".to_string())?;
        if writer.repository_id.as_ref() != Some(&package.repository_id) {
            return Err("writer task and change package repository differ".to_string());
        }
        if package.base_head_oid != repository.head_oid {
            return Err("change package baseline does not match the selected checkout".to_string());
        }
        if package.changed_paths.is_empty()
            || !package
                .changed_paths
                .is_subset(&writer.allowed_changed_paths)
        {
            return Err("change package paths exceed the writer lease".to_string());
        }
        for path in &package.changed_paths {
            validate_relative_path(path)?;
        }
        for (name, value) in [
            ("patch", package.patch_sha256.as_str()),
            ("result tree", package.result_tree_sha256.as_str()),
        ] {
            if !is_sha256(value) {
                return Err(format!("change package {name} digest is invalid"));
            }
        }
        if package.artifact_path.trim().is_empty() {
            return Err("change package artifact path is empty".to_string());
        }
        let expected_isolation = match writer.harness_kind {
            HarnessKind::ClarkCloud => IsolationKind::CloudEphemeralClone,
            HarnessKind::Local | HarnessKind::Acp => IsolationKind::LocalEphemeralClone,
        };
        if package.isolation != expected_isolation
            && package.isolation != IsolationKind::DetachedWorktree
        {
            return Err("change package isolation does not match its harness".to_string());
        }
        Ok(())
    }

    fn validate_repositories(&self) -> Result<(), String> {
        if self.repositories.is_empty() {
            return Err("repository plans require at least one repository".to_string());
        }
        if self.max_parallel_writers == 0 || self.max_parallel_writers > 8 {
            return Err("max_parallel_writers must be between 1 and 8".to_string());
        }
        let mut fingerprints = BTreeSet::new();
        let mut roots: Vec<&Path> = Vec::new();
        for (id, repository) in &self.repositories {
            if id != &repository.repository_id {
                return Err("repository map key does not match repository id".to_string());
            }
            if repository.repository_fingerprint.trim().is_empty()
                || !fingerprints.insert(repository.repository_fingerprint.clone())
            {
                return Err(
                    "each selected repository must have a unique stable fingerprint".to_string(),
                );
            }
            if !is_git_oid(&repository.head_oid) {
                return Err("repository baseline must pin an exact Git object id".to_string());
            }
            if !is_sha256(&repository.dirty_tree_sha256) {
                return Err("repository dirty-tree digest must be SHA-256".to_string());
            }
            let root = Path::new(&repository.checkout_root);
            if !root.is_absolute() {
                return Err("repository checkout roots must be absolute".to_string());
            }
            for existing in &roots {
                if root.starts_with(existing) || existing.starts_with(root) {
                    return Err(
                        "selected repository roots must be disjoint checkout boundaries"
                            .to_string(),
                    );
                }
            }
            roots.push(root);
            for path in &repository.allowed_changed_paths {
                validate_relative_path(path)?;
            }
        }
        Ok(())
    }

    fn validate_tasks(&self) -> Result<BTreeMap<TaskId, &MultiRepoTask>, String> {
        let mut tasks = BTreeMap::new();
        for task in &self.tasks {
            if tasks.insert(task.id.clone(), task).is_some() {
                return Err(format!("duplicate task id: {}", task.id));
            }
            if task.objective.trim().is_empty()
                || task.harness.trim().is_empty()
                || task.model.trim().is_empty()
                || task.budget_reservation == 0
            {
                return Err(format!("task {} is incomplete", task.id));
            }
            if task.dependencies.contains(&task.id) {
                return Err(format!("task {} depends on itself", task.id));
            }
            match task.role {
                MultiRepoTaskRole::Writer => {
                    let repository_id = task
                        .repository_id
                        .as_ref()
                        .ok_or_else(|| "writer tasks require a repository".to_string())?;
                    let repository = self.repositories.get(repository_id).ok_or_else(|| {
                        format!("writer task {} names an unknown repository", task.id)
                    })?;
                    if task.model_tier != ModelTier::Strong {
                        return Err("writer tasks require a strong model tier".to_string());
                    }
                    if task.allowed_changed_paths.is_empty()
                        || !task
                            .allowed_changed_paths
                            .is_subset(&repository.allowed_changed_paths)
                    {
                        return Err("writer lease exceeds its repository path scope".to_string());
                    }
                    if task.harness_kind == HarnessKind::ClarkCloud && !repository.cloud_eligible {
                        return Err(
                            "cloud writer selected a repository without cloud consent".to_string()
                        );
                    }
                }
                MultiRepoTaskRole::Reader => {
                    if task.repository_id.is_none() || !task.allowed_changed_paths.is_empty() {
                        return Err(
                            "reader tasks require one repository and cannot hold a writer lease"
                                .to_string(),
                        );
                    }
                    if task.model_tier == ModelTier::Reviewer {
                        return Err("reader tasks cannot use the reviewer tier".to_string());
                    }
                }
                MultiRepoTaskRole::Reviewer => {
                    if task.model_tier != ModelTier::Reviewer
                        || task.repository_id.is_some()
                        || !task.allowed_changed_paths.is_empty()
                    {
                        return Err("reviewer tasks must be independent and read-only".to_string());
                    }
                }
                MultiRepoTaskRole::Planner | MultiRepoTaskRole::Integrator => {
                    if task.model_tier != ModelTier::Strong
                        || task.repository_id.is_some()
                        || !task.allowed_changed_paths.is_empty()
                    {
                        return Err(
                            "planner and integrator tasks are strong, global, and lease-free"
                                .to_string(),
                        );
                    }
                }
            }
        }
        for task in self.tasks.iter() {
            for dependency in &task.dependencies {
                if !tasks.contains_key(dependency) {
                    return Err(format!(
                        "task {} has unknown dependency {dependency}",
                        task.id
                    ));
                }
            }
        }
        topological_layers(&tasks)?;
        Ok(tasks)
    }

    fn validate_contracts(&self, tasks: &BTreeMap<TaskId, &MultiRepoTask>) -> Result<(), String> {
        let mut edge_ids = BTreeSet::new();
        for edge in &self.contracts {
            if edge.id.trim().is_empty() || !edge_ids.insert(edge.id.clone()) {
                return Err("contract edge ids must be non-empty and unique".to_string());
            }
            if !self.repositories.contains_key(&edge.producer)
                || edge.consumers.is_empty()
                || edge.consumers.iter().any(|consumer| {
                    consumer == &edge.producer || !self.repositories.contains_key(consumer)
                })
                || edge.artifact.trim().is_empty()
                || edge.compatibility_rule.trim().is_empty()
            {
                return Err(format!("contract edge {} is invalid", edge.id));
            }
            let decisions = self
                .contract_decisions
                .iter()
                .filter(|decision| decision.edge_id == edge.id)
                .collect::<Vec<_>>();
            if decisions.len() != 1
                || decisions[0].compatibility_rule != edge.compatibility_rule
                || !is_sha256(&decisions[0].artifact_sha256)
                || tasks
                    .get(&decisions[0].decided_by)
                    .is_none_or(|task| task.role != MultiRepoTaskRole::Planner)
            {
                return Err(format!(
                    "contract edge {} requires one planner-approved exact decision",
                    edge.id
                ));
            }
        }
        if self.contract_decisions.len() != self.contracts.len() {
            return Err("contract decisions must match edges one-to-one".to_string());
        }
        Ok(())
    }

    fn validate_completion_graph(
        &self,
        tasks: &BTreeMap<TaskId, &MultiRepoTask>,
    ) -> Result<(), String> {
        let planners = tasks
            .values()
            .filter(|task| task.role == MultiRepoTaskRole::Planner)
            .collect::<Vec<_>>();
        let integrators = tasks
            .values()
            .filter(|task| task.role == MultiRepoTaskRole::Integrator)
            .collect::<Vec<_>>();
        let reviewers = tasks
            .values()
            .filter(|task| task.role == MultiRepoTaskRole::Reviewer)
            .collect::<Vec<_>>();
        if planners.len() != 1 || integrators.len() != 1 {
            return Err("plans require exactly one planner and one integrator".to_string());
        }
        if reviewers.len() != usize::from(self.requires_independent_review) {
            return Err(
                "review task count does not match the independent-review policy".to_string(),
            );
        }
        let mut writers_by_repo = BTreeMap::<_, Vec<&MultiRepoTask>>::new();
        for task in tasks
            .values()
            .filter(|task| task.role == MultiRepoTaskRole::Writer)
        {
            let repository_id = task
                .repository_id
                .as_ref()
                .expect("validated writer has a repository");
            writers_by_repo
                .entry(repository_id)
                .or_default()
                .push(*task);
        }
        let expected = self
            .repositories
            .values()
            .filter(|repo| !repo.allowed_changed_paths.is_empty())
            .map(|repo| &repo.repository_id)
            .collect::<BTreeSet<_>>();
        if writers_by_repo.keys().copied().collect::<BTreeSet<_>>() != expected {
            return Err("every writable repository requires at least one writer task".to_string());
        }
        for (repository_id, writers) in &writers_by_repo {
            let repository = &self.repositories[*repository_id];
            let mut leased_paths = BTreeSet::new();
            for writer in writers {
                if !leased_paths.is_disjoint(&writer.allowed_changed_paths) {
                    return Err(format!(
                        "writer leases overlap within repository {repository_id}"
                    ));
                }
                leased_paths.extend(writer.allowed_changed_paths.iter().cloned());
            }
            if leased_paths != repository.allowed_changed_paths {
                return Err(format!(
                    "writer leases must exactly cover repository {repository_id} change scope"
                ));
            }
        }
        let writer_ids = writers_by_repo
            .values()
            .flatten()
            .map(|task| task.id.clone())
            .collect::<BTreeSet<_>>();
        let review_gate = reviewers.first().copied();
        if let Some(reviewer) = review_gate {
            if !writer_ids.is_subset(&reviewer.dependencies) {
                return Err("the independent reviewer must depend on every writer".to_string());
            }
        }
        let integrator = integrators[0];
        let required_dependencies = if let Some(reviewer) = review_gate {
            BTreeSet::from([reviewer.id.clone()])
        } else {
            writer_ids
        };
        if !required_dependencies.is_subset(&integrator.dependencies) {
            return Err("fresh integration must follow all writer/reviewer gates".to_string());
        }
        Ok(())
    }

    fn validate_integration_checks(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for check in &self.integration_checks {
            if check.id.trim().is_empty()
                || !ids.insert(check.id.clone())
                || !self.repositories.contains_key(&check.repository_id)
                || check.argv.is_empty()
                || check.argv.len() > 256
                || check
                    .argv
                    .iter()
                    .any(|argument| argument.is_empty() || argument.contains('\0'))
                || !(1..=600_000).contains(&check.timeout_ms)
            {
                return Err(format!("integration check {} is invalid", check.id));
            }
        }
        Ok(())
    }
}

/// Produce the repository-level result digest proven by fresh replay.
///
/// A single writer keeps the historical package digest. Multiple disjoint
/// writers are folded in task-id order so one repository still has one stable
/// receipt value without losing package coverage.
pub fn repository_result_tree_sha256<'a>(
    packages: impl IntoIterator<Item = &'a ChangePackageDescriptor>,
) -> Option<String> {
    let mut packages = packages.into_iter().collect::<Vec<_>>();
    packages.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    if packages.len() == 1 {
        return Some(packages[0].result_tree_sha256.clone());
    }
    if packages.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"clark-repository-result-tree-v1");
    for package in packages {
        hasher.update([0]);
        hasher.update(package.task_id.0.as_bytes());
        hasher.update([0]);
        hasher.update(package.patch_sha256.as_bytes());
        hasher.update([0]);
        hasher.update(package.result_tree_sha256.as_bytes());
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn topological_layers(
    tasks: &BTreeMap<TaskId, &MultiRepoTask>,
) -> Result<Vec<Vec<TaskId>>, String> {
    let mut remaining = tasks.keys().cloned().collect::<BTreeSet<_>>();
    let mut completed = BTreeSet::new();
    let mut layers = Vec::new();
    while !remaining.is_empty() {
        let layer = remaining
            .iter()
            .filter(|id| tasks[*id].dependencies.is_subset(&completed))
            .cloned()
            .collect::<Vec<_>>();
        if layer.is_empty() {
            return Err("task dependency graph contains a cycle".to_string());
        }
        for id in &layer {
            remaining.remove(id);
            completed.insert(id.clone());
        }
        layers.push(layer);
    }
    Ok(layers)
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || path
            .components()
            .any(|component| component.as_os_str() == ".git")
    {
        return Err(format!("unsafe repository-relative path: {value}"));
    }
    Ok(())
}

fn is_git_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "multi_repo_tests.rs"]
mod tests;
