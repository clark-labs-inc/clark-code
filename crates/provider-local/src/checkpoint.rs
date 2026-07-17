//! Working-tree checkpoints used as per-run change-tracking baselines.
//!
//! Checkpoints use a throwaway index through the session's executor. The real
//! Git index and HEAD are never changed, and local and remote sessions follow
//! exactly the same bounded command path.

use std::path::{Path, PathBuf};

use crate::exec::Executor;

const IDENTITY: &[(&str, &str)] = &[
    ("GIT_AUTHOR_NAME", "Clark Code"),
    ("GIT_AUTHOR_EMAIL", "checkpoint@clark.local"),
    ("GIT_COMMITTER_NAME", "Clark Code"),
    ("GIT_COMMITTER_EMAIL", "checkpoint@clark.local"),
];

/// Whether `root` names a Git working tree on the executor target.
pub async fn is_git_repo(exec: &dyn Executor, root: &Path) -> bool {
    crate::git_metadata::succeeds(exec, root, &["rev-parse", "--is-inside-work-tree"])
        .await
        .unwrap_or(false)
}

async fn temp_index(exec: &dyn Executor, root: &Path) -> Result<PathBuf, String> {
    let name = format!("clark-checkpoint-{}.idx", uuid::Uuid::new_v4());
    let path = crate::git_metadata::required(
        exec,
        root,
        &["rev-parse", "--path-format=absolute", "--git-path", &name],
    )
    .await?;
    let path = PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Err("Git returned an empty temporary-index path".into());
    }
    Ok(path)
}

async fn remove_temp_index(exec: &dyn Executor, root: &Path, index: &Path) {
    if exec.remove_file(index).await.is_ok() {
        return;
    }
    // A linked worktree's Git dir can live outside the checkout root. Remote
    // filesystem RPCs are correctly root-confined, so clean this trusted,
    // Git-derived temporary path through the target shell instead.
    let command = format!(
        "rm -f -- {}",
        crate::git_metadata::shell_word(&index.to_string_lossy())
    );
    let _ = exec
        .exec(
            &command,
            root,
            std::time::Duration::from_secs(5),
            &tokio_util::sync::CancellationToken::new(),
        )
        .await;
}

/// Write the complete working tree (including untracked files) to a Git tree
/// without touching the user's real index.
pub(crate) async fn working_tree(exec: &dyn Executor, root: &Path) -> Result<String, String> {
    let index = temp_index(exec, root).await?;
    let index_value = index.to_string_lossy().into_owned();
    let result = async {
        crate::git_metadata::required_with_env(
            exec,
            root,
            &["add", "-A"],
            &[("GIT_INDEX_FILE", index_value.as_str())],
        )
        .await?;
        let tree = crate::git_metadata::required_with_env(
            exec,
            root,
            &["write-tree"],
            &[("GIT_INDEX_FILE", index_value.as_str())],
        )
        .await?;
        let tree = tree.trim().to_string();
        if tree.is_empty() {
            Err("Git returned an empty working-tree object".into())
        } else {
            Ok(tree)
        }
    }
    .await;
    remove_temp_index(exec, root, &index).await;
    result
}

/// Snapshot the working tree. `Ok(None)` means the selected root is not a Git
/// checkout; operational failures remain visible as `Err`.
pub async fn create_checkpoint(exec: &dyn Executor, root: &Path) -> Result<Option<String>, String> {
    if !is_git_repo(exec, root).await {
        return Ok(None);
    }
    let tree = working_tree(exec, root).await?;
    let head = crate::git_metadata::optional(exec, root, &["rev-parse", "HEAD"]).await?;
    let mut args = vec!["commit-tree", tree.as_str(), "-m", "clark checkpoint"];
    if let Some(head) = head.as_deref() {
        args.extend(["-p", head]);
    }
    let sha = crate::git_metadata::required_with_env(exec, root, &args, IDENTITY).await?;
    let sha = sha.trim().to_string();
    if sha.is_empty() {
        return Err("Git returned an empty checkpoint id".into());
    }
    let reference = format!("refs/clark/checkpoints/{sha}");
    crate::git_metadata::required(exec, root, &["update-ref", &reference, &sha]).await?;
    Ok(Some(sha))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    #[tokio::test]
    async fn non_git_dir_has_no_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            create_checkpoint(&LocalExecutor, dir.path()).await.unwrap(),
            None
        );
    }
}
