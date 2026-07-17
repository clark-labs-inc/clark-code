//! Session-wide change review — the data layer behind the Changes panel.
//!
//! Everything diffs against a run checkpoint (see [`crate::checkpoint`]): the
//! panel's baseline is the conversation's FIRST checkpoint, so it shows the sum
//! of what the agent (and the user, in that window) changed. Because checkpoints
//! snapshot the full working tree — untracked files included — comparing the
//! baseline commit against a throwaway snapshot of the CURRENT tree covers
//! creations, edits, and deletions uniformly, without touching the user's index.

use std::path::Path;

use serde::Serialize;

use crate::exec::Executor;

/// One changed file relative to the baseline.
#[derive(Clone, Debug, Serialize)]
pub struct ChangedFile {
    /// Repo-relative path.
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    /// "added" | "modified" | "deleted" | "renamed".
    pub status: String,
}

/// Write the current working tree (untracked included) to a throwaway tree
/// object — same trick as checkpointing, minus the commit + ref.
fn status_label(letter: &str) -> &'static str {
    match letter.chars().next() {
        Some('A') => "added",
        Some('D') => "deleted",
        Some('R') => "renamed",
        _ => "modified",
    }
}

/// Every file that differs between the baseline checkpoint and the current
/// working tree, with per-file +/- line counts.
pub async fn changes_summary(
    exec: &dyn Executor,
    root: &Path,
    base: &str,
) -> Result<Vec<ChangedFile>, String> {
    if !crate::checkpoint::is_git_repo(exec, root).await {
        return Err("not a git repository".into());
    }
    let tree = crate::checkpoint::working_tree(exec, root).await?;
    let numstat_args = ["diff", "--numstat", base, tree.as_str()];
    let name_status_args = ["diff", "--name-status", base, tree.as_str()];
    let (numstat, name_status) = tokio::join!(
        crate::git_metadata::required(exec, root, &numstat_args),
        crate::git_metadata::required(exec, root, &name_status_args),
    );
    let numstat = numstat?;
    let name_status = name_status?;

    let mut status_by_path = std::collections::HashMap::new();
    for line in name_status.lines() {
        let mut parts = line.split('\t');
        let (Some(letter), Some(path)) = (parts.next(), parts.next()) else {
            continue;
        };
        // Renames carry "old\tnew" — key on the new path.
        let path = parts.next().unwrap_or(path);
        status_by_path.insert(path.to_string(), status_label(letter).to_string());
    }

    let mut out = Vec::new();
    for line in numstat.lines() {
        let mut parts = line.split('\t');
        let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        out.push(ChangedFile {
            path: path.to_string(),
            // "-" for binary files → 0.
            additions: add.parse().unwrap_or(0),
            deletions: del.parse().unwrap_or(0),
            status: status_by_path
                .get(path)
                .cloned()
                .unwrap_or_else(|| "modified".into()),
        });
    }
    Ok(out)
}

/// Unified diff of one file against the baseline.
pub async fn changes_diff(
    exec: &dyn Executor,
    root: &Path,
    base: &str,
    path: &str,
) -> Result<String, String> {
    let tree = crate::checkpoint::working_tree(exec, root).await?;
    crate::git_metadata::required(exec, root, &["diff", base, &tree, "--", path]).await
}

/// Restore one file to its baseline state: files that existed at the baseline
/// come back to that content (worktree only — the user's index is untouched);
/// files the session created are removed.
pub async fn changes_revert(
    exec: &dyn Executor,
    root: &Path,
    base: &str,
    path: &str,
) -> Result<(), String> {
    // Containment: the path must stay inside the repo root.
    let joined = root.join(path);
    if !joined.starts_with(root) || path.contains("..") {
        return Err("path escapes the project root".into());
    }
    let existed =
        crate::git_metadata::succeeds(exec, root, &["cat-file", "-e", &format!("{base}:{path}")])
            .await
            .unwrap_or(false);
    if existed {
        crate::git_metadata::required(
            exec,
            root,
            &["restore", "--source", base, "--worktree", "--", path],
        )
        .await?;
        Ok(())
    } else {
        match exec.metadata(&joined).await {
            Ok(meta) if meta.is_dir && !meta.is_symlink => exec.remove_dir_all(&joined).await,
            Ok(_) => exec.remove_file(&joined).await,
            Err(_) => Ok(()),
        }
        .map_err(|error| format!("removing {path}: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::create_checkpoint;
    use crate::exec::LocalExecutor;
    use std::process::Command;

    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            assert!(Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
                .status
                .success());
        };
        run(&["init", "-q"]);
        std::fs::write(dir.join("keep.txt"), "one\ntwo\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "init"]);
    }

    #[tokio::test]
    async fn summary_diff_and_revert_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        init_repo(root);
        let base = create_checkpoint(&LocalExecutor, root)
            .await
            .unwrap()
            .expect("checkpoint");

        // Session activity: edit a tracked file, create a new (untracked) one,
        // delete another.
        std::fs::write(root.join("keep.txt"), "one\nCHANGED\n").unwrap();
        std::fs::write(root.join("new.txt"), "fresh\n").unwrap();

        let summary = changes_summary(&LocalExecutor, root, &base)
            .await
            .expect("summary");
        let paths: Vec<_> = summary.iter().map(|c| c.path.as_str()).collect();
        assert!(paths.contains(&"keep.txt"), "{paths:?}");
        assert!(paths.contains(&"new.txt"), "{paths:?}");
        let new = summary.iter().find(|c| c.path == "new.txt").unwrap();
        assert_eq!(new.status, "added");
        assert_eq!(new.additions, 1);

        // Per-file diff renders a unified diff.
        let diff = changes_diff(&LocalExecutor, root, &base, "keep.txt")
            .await
            .expect("diff");
        assert!(diff.contains("-two"), "{diff}");
        assert!(diff.contains("+CHANGED"), "{diff}");

        // Revert the edit → original content; revert the creation → file gone.
        changes_revert(&LocalExecutor, root, &base, "keep.txt")
            .await
            .expect("revert edit");
        assert_eq!(
            std::fs::read_to_string(root.join("keep.txt")).unwrap(),
            "one\ntwo\n"
        );
        changes_revert(&LocalExecutor, root, &base, "new.txt")
            .await
            .expect("revert creation");
        assert!(!root.join("new.txt").exists());

        // Everything reverted → empty summary.
        let after = changes_summary(&LocalExecutor, root, &base)
            .await
            .expect("summary after");
        assert!(after.is_empty(), "{after:?}");
    }

    #[tokio::test]
    async fn revert_rejects_escaping_paths() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let base = create_checkpoint(&LocalExecutor, tmp.path())
            .await
            .unwrap()
            .unwrap();
        assert!(changes_revert(&LocalExecutor, tmp.path(), &base, "../evil")
            .await
            .is_err());
    }
}
