//! Working-tree checkpoints for one-click "undo this run".
//!
//! Checkpoints use a throwaway index through the session's executor. The real
//! Git index and HEAD are never changed, and local and remote sessions follow
//! exactly the same bounded command path.

use std::path::{Component, Path, PathBuf};

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
pub(crate) async fn working_tree(
    exec: &dyn Executor,
    root: &Path,
) -> Result<String, String> {
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
pub async fn create_checkpoint(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Option<String>, String> {
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

/// Whether a checkpoint commit still resolves.
pub async fn checkpoint_exists(exec: &dyn Executor, root: &Path, sha: &str) -> bool {
    if sha.is_empty() {
        return false;
    }
    crate::git_metadata::succeeds(exec, root, &["cat-file", "-e", &format!("{sha}^{{commit}}")])
        .await
        .unwrap_or(false)
}

fn safe_relative(path: &str) -> Option<&Path> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return None;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then_some(path)
}

async fn remove_added_path(exec: &dyn Executor, root: &Path, path: &str) -> Result<(), String> {
    let relative = safe_relative(path).ok_or_else(|| "Git returned an unsafe path".to_string())?;
    let target = root.join(relative);
    match exec.metadata(&target).await {
        Ok(meta) if meta.is_dir && !meta.is_symlink => exec.remove_dir_all(&target).await,
        Ok(_) => exec.remove_file(&target).await,
        Err(_) => Ok(()),
    }
}

/// Restore only the working tree to a checkpoint. The user's real index and
/// HEAD are preserved; files created after the checkpoint are removed using
/// executor filesystem primitives instead of a repository-wide `git clean`.
pub async fn restore_checkpoint(
    exec: &dyn Executor,
    root: &Path,
    sha: &str,
) -> Result<(), String> {
    if !is_git_repo(exec, root).await {
        return Err("Undo needs a git repository.".to_string());
    }
    if !checkpoint_exists(exec, root, sha).await {
        return Err("This checkpoint is no longer available.".to_string());
    }

    let current = working_tree(exec, root).await?;
    let added = crate::git_metadata::required(
        exec,
        root,
        &[
            "diff",
            "--no-renames",
            "--name-only",
            "-z",
            "--diff-filter=A",
            sha,
            &current,
        ],
    )
    .await?;
    crate::git_metadata::required(
        exec,
        root,
        &["restore", "--source", sha, "--worktree", "--", "."],
    )
    .await?;
    for path in added.split('\0').filter(|path| !path.is_empty()) {
        remove_added_path(exec, root, path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;
    use std::fs;
    use std::process::Command;

    fn run(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    fn init(root: &Path) {
        run(root, &["init", "-q"]);
        fs::write(root.join("keep.txt"), "original\n").unwrap();
        fs::write(root.join("delete_me.txt"), "bye\n").unwrap();
        run(root, &["add", "-A"]);
        run(root, &["commit", "-qm", "init"]);
    }

    #[tokio::test]
    async fn non_git_dir_has_no_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(create_checkpoint(&LocalExecutor, dir.path()).await.unwrap(), None);
        assert!(restore_checkpoint(&LocalExecutor, dir.path(), "deadbeef")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn checkpoint_restores_tree_without_changing_real_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        fs::write(root.join("staged.txt"), "staged before checkpoint\n").unwrap();
        run(root, &["add", "staged.txt"]);
        let index_before = Command::new("git")
            .args(["-C", root.to_str().unwrap(), "diff", "--cached", "--raw"])
            .output()
            .unwrap()
            .stdout;

        let sha = create_checkpoint(&LocalExecutor, root)
            .await
            .unwrap()
            .expect("checkpoint");
        fs::write(root.join("keep.txt"), "MANGLED\n").unwrap();
        fs::write(root.join("new_file.txt"), "agent made this\n").unwrap();
        fs::remove_file(root.join("delete_me.txt")).unwrap();

        restore_checkpoint(&LocalExecutor, root, &sha).await.unwrap();
        assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "original\n");
        assert!(!root.join("new_file.txt").exists());
        assert_eq!(fs::read_to_string(root.join("delete_me.txt")).unwrap(), "bye\n");
        let index_after = Command::new("git")
            .args(["-C", root.to_str().unwrap(), "diff", "--cached", "--raw"])
            .output()
            .unwrap()
            .stdout;
        assert_eq!(index_after, index_before, "restore must preserve the real index");
    }

    #[tokio::test]
    async fn checkpoint_captures_untracked_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        fs::write(root.join("untracked.txt"), "keep me\n").unwrap();
        let sha = create_checkpoint(&LocalExecutor, root)
            .await
            .unwrap()
            .expect("checkpoint");
        fs::remove_file(root.join("untracked.txt")).unwrap();
        restore_checkpoint(&LocalExecutor, root, &sha).await.unwrap();
        assert_eq!(fs::read_to_string(root.join("untracked.txt")).unwrap(), "keep me\n");
    }
}
