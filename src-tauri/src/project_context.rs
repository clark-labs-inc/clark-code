use std::{path::Path, time::Duration};

use provider_local::Executor;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

const GIT_CONTEXT_TIMEOUT: Duration = Duration::from_secs(5);
const GIT_CONTEXT_COMMAND: &str = "git rev-parse --is-inside-work-tree && \
git rev-parse --show-toplevel && \
(git symbolic-ref --quiet --short HEAD || { printf 'detached:'; git rev-parse --short HEAD; }) && \
git rev-parse --path-format=absolute --git-dir && \
git rev-parse --path-format=absolute --git-common-dir";

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectContext {
    pub branch: String,
    pub detached: bool,
    pub is_worktree: bool,
    pub worktree_root: String,
}

pub async fn inspect_project_context(
    executor: &dyn Executor,
    cwd: &Path,
) -> Result<Option<ProjectContext>, String> {
    let output = executor
        .exec(
            GIT_CONTEXT_COMMAND,
            cwd,
            GIT_CONTEXT_TIMEOUT,
            &CancellationToken::new(),
        )
        .await?;
    if output.code != Some(0) {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    if lines.next() != Some("true") {
        return Ok(None);
    }
    let Some(worktree_root) = lines.next().filter(|line| !line.is_empty()) else {
        return Ok(None);
    };
    let Some(branch_line) = lines.next().filter(|line| !line.is_empty()) else {
        return Ok(None);
    };
    let Some(git_dir) = lines.next().filter(|line| !line.is_empty()) else {
        return Ok(None);
    };
    let Some(git_common_dir) = lines.next().filter(|line| !line.is_empty()) else {
        return Ok(None);
    };
    let (detached, branch) = branch_line
        .strip_prefix("detached:")
        .map_or((false, branch_line), |commit| (true, commit));

    Ok(Some(ProjectContext {
        branch: branch.to_string(),
        detached,
        is_worktree: git_dir != git_common_dir,
        worktree_root: worktree_root.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::inspect_project_context;
    use provider_local::LocalExecutor;
    use std::{path::Path, process::Command};

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn distinguishes_the_main_checkout_from_a_linked_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        let linked = temp.path().join("project-feature");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "test@clark.local"]);
        git(&repo, &["config", "user.name", "Clark Test"]);
        std::fs::write(repo.join("README.md"), "fixture\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature/context-bar",
                linked.to_str().unwrap(),
            ],
        );

        let main = inspect_project_context(&LocalExecutor, &repo)
            .await
            .unwrap()
            .unwrap();
        let worktree = inspect_project_context(&LocalExecutor, &linked)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(main.branch, "main");
        assert!(!main.is_worktree);
        assert_eq!(worktree.branch, "feature/context-bar");
        assert!(worktree.is_worktree);
        assert_eq!(
            worktree.worktree_root,
            linked.canonicalize().unwrap().to_string_lossy()
        );
    }
}
