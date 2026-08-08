//! Working-tree checkpoints used as per-run change-tracking baselines.
//!
//! Checkpoints use a throwaway index through the session's executor. The real
//! Git index and HEAD are never changed, and local and remote sessions follow
//! exactly the same bounded command path.

use std::path::{Path, PathBuf};

use crate::exec::Executor;

const IDENTITY: &[(&str, &str)] = &[
    ("GIT_AUTHOR_NAME", "local agent"),
    ("GIT_AUTHOR_EMAIL", "checkpoint@agent.local"),
    ("GIT_COMMITTER_NAME", "local agent"),
    ("GIT_COMMITTER_EMAIL", "checkpoint@agent.local"),
];

/// Whether `root` names a Git working tree on the executor target.
pub async fn is_git_repo(exec: &dyn Executor, root: &Path) -> bool {
    crate::git_metadata::succeeds(exec, root, &["rev-parse", "--is-inside-work-tree"])
        .await
        .unwrap_or(false)
}

async fn temp_index(exec: &dyn Executor, root: &Path) -> Result<PathBuf, String> {
    let name = format!("agent-checkpoint-{}.idx", uuid::Uuid::new_v4());
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
    let path = crate::git_metadata::shell_path_word(index);
    let command = match exec_core::scripted_shell_kind() {
        exec_core::ShellKind::Posix => format!("rm -f -- {path}"),
        exec_core::ShellKind::PowerShell => {
            format!("Remove-Item -LiteralPath {path} -Force -ErrorAction SilentlyContinue")
        }
        exec_core::ShellKind::Cmd => format!("del /F /Q {path}"),
    };
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
        // Start from HEAD so tracked files stay present even when a later
        // .gitignore rule matches them. An empty temporary index followed by
        // `git add -A` silently drops those files because Git treats them as
        // ignored, despite their tracked status in the real repository.
        if crate::git_metadata::optional(exec, root, &["rev-parse", "--verify", "HEAD"])
            .await?
            .is_some()
        {
            crate::git_metadata::required_with_env(
                exec,
                root,
                &["read-tree", "HEAD"],
                &[("GIT_INDEX_FILE", index_value.as_str())],
            )
            .await?;
        }
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
    // Include a nonce so simultaneous sessions that snapshot the same tree in
    // the same second never share a commit/ref. That makes per-conversation
    // release safe when its durable history is deleted.
    let message = format!("agent checkpoint {}", uuid::Uuid::new_v4());
    let mut args = vec!["commit-tree", tree.as_str(), "-m", message.as_str()];
    if let Some(head) = head.as_deref() {
        args.extend(["-p", head]);
    }
    let sha = crate::git_metadata::required_with_env(exec, root, &args, IDENTITY).await?;
    let sha = sha.trim().to_string();
    if sha.is_empty() {
        return Err("Git returned an empty checkpoint id".into());
    }
    let reference = format!("refs/agent/checkpoints/{sha}");
    crate::git_metadata::required(exec, root, &["update-ref", &reference, &sha]).await?;
    Ok(Some(sha))
}

/// Release checkpoint-retention refs after their owning conversation is
/// permanently deleted. The commit objects remain recoverable until normal Git
/// maintenance; only Agent Desktop's explicit retention roots are removed.
pub async fn release_checkpoints(
    exec: &dyn Executor,
    root: &Path,
    checkpoints: &[String],
) -> Result<(), String> {
    if !is_git_repo(exec, root).await {
        return Ok(());
    }
    for checkpoint in checkpoints {
        if !matches!(checkpoint.len(), 40 | 64)
            || !checkpoint.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("invalid checkpoint id".into());
        }
        let reference = format!("refs/agent/checkpoints/{checkpoint}");
        crate::git_metadata::required(exec, root, &["update-ref", "-d", &reference, checkpoint])
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Agent Test")
            .env("GIT_AUTHOR_EMAIL", "agent@example.com")
            .env("GIT_COMMITTER_NAME", "Agent Test")
            .env("GIT_COMMITTER_EMAIL", "agent@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    #[tokio::test]
    async fn non_git_dir_has_no_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            create_checkpoint(&LocalExecutor, dir.path()).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn checkpoint_keeps_modified_tracked_file_that_is_now_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        std::fs::write(root.join("tracked.log"), "before\n").unwrap();
        git(root, &["add", "tracked.log"]);
        git(root, &["commit", "-q", "-m", "initial"]);

        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        git(root, &["add", ".gitignore"]);
        git(root, &["commit", "-q", "-m", "ignore logs"]);
        std::fs::write(root.join("tracked.log"), "after\n").unwrap();

        let checkpoint = create_checkpoint(&LocalExecutor, root)
            .await
            .unwrap()
            .expect("checkpoint");
        assert_eq!(
            git(root, &["show", &format!("{checkpoint}:tracked.log")]),
            "after\n"
        );
    }

    #[tokio::test]
    async fn releasing_one_checkpoint_keeps_other_conversations_retained() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        std::fs::write(root.join("tracked.txt"), "one\n").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-q", "-m", "initial"]);

        let first = create_checkpoint(&LocalExecutor, root)
            .await
            .unwrap()
            .unwrap();
        let second = create_checkpoint(&LocalExecutor, root)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(first, second, "checkpoint nonces prevent shared refs");

        release_checkpoints(&LocalExecutor, root, std::slice::from_ref(&first))
            .await
            .unwrap();
        assert!(crate::git_metadata::succeeds(
            &LocalExecutor,
            root,
            &[
                "show-ref",
                "--verify",
                &format!("refs/agent/checkpoints/{second}")
            ],
        )
        .await
        .unwrap());
        assert!(!crate::git_metadata::succeeds(
            &LocalExecutor,
            root,
            &[
                "show-ref",
                "--verify",
                &format!("refs/agent/checkpoints/{first}")
            ],
        )
        .await
        .unwrap());
    }
}
