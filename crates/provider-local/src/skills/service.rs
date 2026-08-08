use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::{loader, SkillCatalog, SkillCatalogSnapshot};
use crate::exec::Executor;

#[derive(Clone)]
struct CatalogPolicy {
    available_tools: HashSet<String>,
    disabled_names: Vec<String>,
}

struct CachedCatalog {
    catalog: Arc<SkillCatalog>,
    policy: Option<CatalogPolicy>,
}

/// One process-wide catalog authority shared by the composer and local-agent
/// providers. Discovery is executor-aware, while each returned `Arc` is an
/// immutable snapshot suitable for one agent run.
#[derive(Default)]
pub struct SkillCatalogService {
    catalogs: RwLock<HashMap<String, CachedCatalog>>,
}

impl SkillCatalogService {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn refresh_for_provider(
        &self,
        exec: &dyn Executor,
        project_root: &Path,
        environment_id: &str,
        available_tools: &HashSet<String>,
        disabled_names: &[String],
    ) -> Arc<SkillCatalog> {
        let policy = CatalogPolicy {
            available_tools: available_tools.clone(),
            disabled_names: disabled_names.to_vec(),
        };
        self.refresh(exec, project_root, environment_id, Some(policy))
            .await
    }

    /// Re-scan using the same capability policy as the active provider. If the
    /// composer asks before a provider session exists, discovery still works
    /// and capability resolution is finalized when that session starts.
    pub async fn refresh_snapshot(
        &self,
        exec: &dyn Executor,
        project_root: &Path,
        environment_id: &str,
    ) -> SkillCatalogSnapshot {
        let policy = self
            .catalogs
            .read()
            .await
            .get(environment_id)
            .and_then(|cached| cached.policy.clone());
        self.refresh(exec, project_root, environment_id, policy)
            .await
            .snapshot(environment_id, project_root)
    }

    pub async fn current_snapshot(
        &self,
        project_root: &Path,
        environment_id: &str,
    ) -> Option<SkillCatalogSnapshot> {
        self.catalogs
            .read()
            .await
            .get(environment_id)
            .map(|cached| cached.catalog.snapshot(environment_id, project_root))
    }

    async fn refresh(
        &self,
        exec: &dyn Executor,
        project_root: &Path,
        environment_id: &str,
        policy: Option<CatalogPolicy>,
    ) -> Arc<SkillCatalog> {
        let mut catalog = loader::discover_catalog(exec, project_root).await;
        if let Some(policy) = &policy {
            catalog.resolve_capabilities(&policy.available_tools, &policy.disabled_names);
        }
        let catalog = Arc::new(catalog);
        self.catalogs.write().await.insert(
            environment_id.to_string(),
            CachedCatalog {
                catalog: catalog.clone(),
                policy,
            },
        );
        catalog
    }
}

pub fn skill_environment_id(project_root: &Path, remote_identity: Option<&str>) -> String {
    match remote_identity {
        Some(remote) => format!("remote:{remote}:{}", project_root.display()),
        None => format!("local:{}", project_root.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    fn write_skill(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!("---\nname: review\ndescription: Review changes\n---\n\n{body}\n"),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn provider_and_ui_share_revisioned_snapshots_and_refresh_between_runs() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repo");
        let source = project.join(".agent/skills/review/SKILL.md");
        write_skill(&source, "Version one.");
        let service = SkillCatalogService::new();
        let environment = skill_environment_id(&project, None);
        let tools = HashSet::from(["bash".to_string()]);

        let provider = service
            .refresh_for_provider(&LocalExecutor, &project, &environment, &tools, &[])
            .await;
        let provider_snapshot = provider.snapshot(&environment, &project);
        let ui_snapshot = service
            .current_snapshot(&project, &environment)
            .await
            .unwrap();
        assert_eq!(provider_snapshot.revision, ui_snapshot.revision);
        let provider_review = provider_snapshot
            .skills
            .iter()
            .find(|skill| skill.name == "review")
            .unwrap();
        let ui_review = ui_snapshot
            .skills
            .iter()
            .find(|skill| skill.name == "review")
            .unwrap();
        assert_eq!(provider_review.id, ui_review.id);

        write_skill(&source, "Version two.");
        let refreshed = service
            .refresh_snapshot(&LocalExecutor, &project, &environment)
            .await;
        assert_ne!(refreshed.revision, ui_snapshot.revision);
        let refreshed_review = refreshed
            .skills
            .iter()
            .find(|skill| skill.name == "review")
            .unwrap();
        assert_eq!(refreshed_review.id, ui_review.id);
        assert_ne!(refreshed_review.revision, ui_review.revision);
    }
}
