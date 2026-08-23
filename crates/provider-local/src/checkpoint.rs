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
            crate::git_metadata::required_tree_op_with_env(
                exec,
                root,
                &["read-tree", "HEAD"],
                &[("GIT_INDEX_FILE", index_value.as_str())],
            )
            .await?;
        }
        // `add -A` hashes every modified and untracked file; on a large
        // checkout that exceeds the metadata bound, and a timed-out checkpoint
        // silently strips the run of its undo baseline and Changes panel.
        crate::git_metadata::required_tree_op_with_env(
            exec,
            root,
            &["add", "-A"],
            &[("GIT_INDEX_FILE", index_value.as_str())],
        )
        .await?;
        let tree = crate::git_metadata::required_tree_op_with_env(
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

/// How long a checkpoint ref pins its trees after creation.
///
/// Checkpoints are undo/diff baselines for recent work. Their refs are GC
/// roots, so without a horizon every turn of every conversation pinned its
/// full working tree forever — measured here as a 1.2 GB `.git` — because the
/// only release path was permanent conversation deletion, which most
/// conversations never receive. Past this horizon the ref is dropped; the
/// commit objects remain until normal Git maintenance, and a Changes panel
/// pointed at a pruned baseline reports an error and offers newer baselines.
const CHECKPOINT_RETENTION: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);
/// Bound one sweep so a huge backlog amortizes across runs instead of
/// stalling the run that happened to trip it.
const MAX_PRUNED_PER_SWEEP: usize = 64;

/// Drop checkpoint refs older than the retention horizon. Best-effort: a run
/// must never fail because housekeeping could not.
pub async fn prune_stale_checkpoints(exec: &dyn Executor, root: &Path) {
    let listing = match crate::git_metadata::optional(
        exec,
        root,
        &[
            "for-each-ref",
            "--format=%(refname) %(creatordate:unix)",
            "refs/agent/checkpoints/",
        ],
    )
    .await
    {
        Ok(Some(listing)) => listing,
        Ok(None) | Err(_) => return,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let horizon = now.saturating_sub(CHECKPOINT_RETENTION.as_secs());
    let stale = listing
        .lines()
        .filter_map(|line| {
            let (reference, created) = line.rsplit_once(' ')?;
            let created: u64 = created.trim().parse().ok()?;
            (created < horizon && reference.starts_with("refs/agent/checkpoints/"))
                .then(|| reference.to_string())
        })
        .take(MAX_PRUNED_PER_SWEEP);
    for reference in stale {
        if let Err(error) =
            crate::git_metadata::required(exec, root, &["update-ref", "-d", &reference]).await
        {
            tracing::debug!(%error, reference, "stale checkpoint ref not pruned");
        }
    }
}

/// Release checkpoint-retention refs after their owning conversation is
/// permanently deleted. The commit objects remain recoverable until normal Git
/// maintenance; only Clark Code's explicit retention roots are removed.
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

    /// Records which time bound each git command was given. The distinction is
    /// the contract under test: `add -A`/`write-tree` scale with the checkout
    /// and must get the tree bound, while metadata stays on the short bound so
    /// a wedged repository still fails fast. When this was uniform at the
    /// metadata bound, checkpointing on large checkouts timed out and the run
    /// silently lost its undo baseline.
    struct TimeoutProbe {
        calls: std::sync::Mutex<Vec<(String, std::time::Duration)>>,
    }

    #[async_trait::async_trait]
    impl crate::exec::Executor for TimeoutProbe {
        async fn read(&self, _: &Path) -> exec_core::ExecResult<Vec<u8>> {
            unreachable!("not used by working_tree")
        }
        async fn write(&self, _: &Path, _: &[u8]) -> exec_core::ExecResult<()> {
            unreachable!("not used by working_tree")
        }
        async fn create_dir_all(&self, _: &Path) -> exec_core::ExecResult<()> {
            unreachable!("not used by working_tree")
        }
        async fn remove_file(&self, _: &Path) -> exec_core::ExecResult<()> {
            Ok(())
        }
        async fn remove_dir_all(&self, _: &Path) -> exec_core::ExecResult<()> {
            unreachable!("not used by working_tree")
        }
        async fn rename(&self, _: &Path, _: &Path) -> exec_core::ExecResult<()> {
            unreachable!("not used by working_tree")
        }
        async fn read_dir(&self, _: &Path) -> exec_core::ExecResult<Vec<exec_core::DirEntry>> {
            unreachable!("not used by working_tree")
        }
        async fn metadata(&self, _: &Path) -> exec_core::ExecResult<exec_core::FileMeta> {
            unreachable!("not used by working_tree")
        }
        async fn canonicalize(&self, _: &Path) -> exec_core::ExecResult<PathBuf> {
            unreachable!("not used by working_tree")
        }
        async fn home_dir(&self, _: &Path) -> exec_core::ExecResult<PathBuf> {
            unreachable!("not used by working_tree")
        }
        async fn walk(&self, _: &Path) -> exec_core::ExecResult<Vec<exec_core::WalkEntry>> {
            unreachable!("not used by working_tree")
        }
        async fn exec(
            &self,
            command: &str,
            _cwd: &Path,
            timeout: std::time::Duration,
            _cancel: &tokio_util::sync::CancellationToken,
        ) -> exec_core::ExecResult<exec_core::ExecOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((command.to_string(), timeout));
            let ok = |stdout: &str| exec_core::ExecOutput {
                stdout: stdout.as_bytes().to_vec(),
                stderr: Vec::new(),
                code: Some(0),
            };
            if command.contains("--git-path") {
                return Ok(ok("/tmp/probe-checkpoint.idx\n"));
            }
            if command.contains("rev-parse --verify HEAD") {
                // No HEAD: exercises the empty-repository branch and skips
                // read-tree, keeping the scripted surface small.
                return Ok(exec_core::ExecOutput {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    code: Some(1),
                });
            }
            if command.contains("write-tree") {
                return Ok(ok("0123456789abcdef0123456789abcdef01234567\n"));
            }
            Ok(ok(""))
        }
    }

    #[tokio::test]
    async fn repository_scaled_commands_get_the_tree_bound() {
        let probe = TimeoutProbe {
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let tree = working_tree(&probe, Path::new("/repo")).await.unwrap();
        assert_eq!(tree, "0123456789abcdef0123456789abcdef01234567");

        let calls = probe.calls.lock().unwrap();
        assert!(!calls.is_empty());
        for (command, timeout) in calls.iter() {
            let expected = if command.contains("add -A") || command.contains("write-tree") {
                crate::git_metadata::TREE_OP_TIMEOUT
            } else {
                crate::git_metadata::COMMAND_TIMEOUT
            };
            assert_eq!(
                *timeout, expected,
                "unexpected bound for {command:?}: {timeout:?}"
            );
        }
        let hashed_everything = calls.iter().any(|(command, _)| command.contains("add -A"));
        assert!(hashed_everything, "working_tree no longer stages the tree");
    }

    #[tokio::test]
    async fn pruning_drops_only_refs_past_the_retention_horizon() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        std::fs::write(root.join("file.txt"), "content\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "base"]);

        let fresh = create_checkpoint(&LocalExecutor, root)
            .await
            .unwrap()
            .unwrap();

        // Manufacture a checkpoint whose committer date is past the horizon.
        // The date env vars take strict formats only; epoch form is exact.
        let tree = git(root, &["rev-parse", "HEAD^{tree}"]);
        let forty_days_ago = format!(
            "@{} +0000",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                - 40 * 24 * 60 * 60
        );
        let stale = {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["commit-tree", tree.trim(), "-m", "agent checkpoint old"])
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .env("GIT_COMMITTER_DATE", &forty_days_ago)
                .env("GIT_AUTHOR_DATE", &forty_days_ago)
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        git(
            root,
            &[
                "update-ref",
                &format!("refs/agent/checkpoints/{stale}"),
                &stale,
            ],
        );

        prune_stale_checkpoints(&LocalExecutor, root).await;

        let refs = git(root, &["for-each-ref", "refs/agent/checkpoints/"]);
        assert!(
            refs.contains(&fresh),
            "fresh checkpoint must survive: {refs}"
        );
        assert!(
            !refs.contains(&stale),
            "stale checkpoint must be pruned: {refs}"
        );
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
