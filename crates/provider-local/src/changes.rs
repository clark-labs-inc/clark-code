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
use std::collections::HashMap;
use std::path::Component;

use crate::exec::Executor;

/// One changed file relative to the baseline.
#[derive(Clone, Debug, Serialize)]
pub struct ChangedFile {
    /// Repo-relative path.
    pub path: String,
    /// Original repo-relative path for a rename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
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

fn parse_name_status(raw: &str) -> HashMap<String, (String, Option<String>)> {
    let mut fields = raw.split_terminator('\0');
    let mut out = HashMap::new();
    while let Some(letter) = fields.next() {
        if letter.starts_with('R') {
            let (Some(previous), Some(path)) = (fields.next(), fields.next()) else {
                break;
            };
            out.insert(
                path.to_string(),
                (status_label(letter).to_string(), Some(previous.to_string())),
            );
        } else {
            let Some(path) = fields.next() else { break };
            out.insert(path.to_string(), (status_label(letter).to_string(), None));
        }
    }
    out
}

fn parse_numstat(
    raw: &str,
    statuses: &HashMap<String, (String, Option<String>)>,
) -> Vec<ChangedFile> {
    let mut records = raw.split_terminator('\0');
    let mut out = Vec::new();
    while let Some(record) = records.next() {
        let mut parts = record.splitn(3, '\t');
        let (Some(add), Some(del), Some(path_field)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        // With `--numstat -z`, a rename has an empty path in the first record,
        // followed by separate old/new NUL-terminated paths.
        let path = if path_field.is_empty() {
            let (Some(_previous), Some(path)) = (records.next(), records.next()) else {
                break;
            };
            path
        } else {
            path_field
        };
        let (status, previous_path) = statuses
            .get(path)
            .cloned()
            .unwrap_or_else(|| ("modified".into(), None));
        out.push(ChangedFile {
            path: path.to_string(),
            previous_path,
            additions: add.parse().unwrap_or(0),
            deletions: del.parse().unwrap_or(0),
            status,
        });
    }
    out
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
    // `-z` is required here: human-readable Git output quotes non-ASCII paths
    // and encodes renames as display-only `old => new` strings that cannot be
    // passed back to diff or restore.
    let numstat_args = [
        "diff",
        "--find-renames",
        "--numstat",
        "-z",
        base,
        tree.as_str(),
    ];
    let name_status_args = [
        "diff",
        "--find-renames",
        "--name-status",
        "-z",
        base,
        tree.as_str(),
    ];
    let (numstat, name_status) = tokio::join!(
        crate::git_metadata::required(exec, root, &numstat_args),
        crate::git_metadata::required(exec, root, &name_status_args),
    );
    let numstat = numstat?;
    let name_status = name_status?;

    Ok(parse_numstat(&numstat, &parse_name_status(&name_status)))
}

/// Unified diff of one file against the baseline.
pub async fn changes_diff(
    exec: &dyn Executor,
    root: &Path,
    base: &str,
    path: &str,
    previous_path: Option<&str>,
) -> Result<String, String> {
    let tree = crate::checkpoint::working_tree(exec, root).await?;
    let mut args = vec!["diff", "--find-renames", base, &tree, "--"];
    if let Some(previous) = previous_path.filter(|previous| *previous != path) {
        args.push(previous);
    }
    args.push(path);
    crate::git_metadata::required(exec, root, &args).await
}

fn validate_path(root: &Path, path: &str) -> Result<(), String> {
    let relative = Path::new(path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || !root.join(relative).starts_with(root)
    {
        return Err("path escapes the project root".into());
    }
    Ok(())
}

async fn remove_created_path(exec: &dyn Executor, root: &Path, path: &str) -> Result<(), String> {
    let joined = root.join(path);
    match exec.metadata(&joined).await {
        Ok(meta) if meta.is_dir && !meta.is_symlink => exec.remove_dir_all(&joined).await,
        Ok(_) => exec.remove_file(&joined).await,
        Err(_) => Ok(()),
    }
    .map_err(|error| format!("removing {path}: {error}"))
}

/// Restore one file to its baseline state: files that existed at the baseline
/// come back to that content (worktree only — the user's index is untouched);
/// files the session created are removed.
pub async fn changes_revert(
    exec: &dyn Executor,
    root: &Path,
    base: &str,
    path: &str,
    previous_path: Option<&str>,
) -> Result<(), String> {
    validate_path(root, path)?;
    if let Some(previous) = previous_path {
        validate_path(root, previous)?;
        crate::git_metadata::required(
            exec,
            root,
            &["restore", "--source", base, "--worktree", "--", previous],
        )
        .await?;
        if previous != path {
            remove_created_path(exec, root, path).await?;
        }
        return Ok(());
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
        remove_created_path(exec, root, path).await
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
        run(&["config", "core.autocrlf", "false"]);
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
        let diff = changes_diff(&LocalExecutor, root, &base, "keep.txt", None)
            .await
            .expect("diff");
        assert!(diff.contains("-two"), "{diff}");
        assert!(diff.contains("+CHANGED"), "{diff}");

        // Revert the edit → original content; revert the creation → file gone.
        changes_revert(&LocalExecutor, root, &base, "keep.txt", None)
            .await
            .expect("revert edit");
        assert_eq!(
            std::fs::read_to_string(root.join("keep.txt")).unwrap(),
            "one\ntwo\n"
        );
        changes_revert(&LocalExecutor, root, &base, "new.txt", None)
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
        assert!(
            changes_revert(&LocalExecutor, tmp.path(), &base, "../evil", None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rename_with_unicode_whitespace_can_be_diffed_and_reverted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        init_repo(root);
        let base = create_checkpoint(&LocalExecutor, root)
            .await
            .unwrap()
            .unwrap();
        let renamed = "café name\u{2003}new.txt";
        std::fs::rename(root.join("keep.txt"), root.join(renamed)).unwrap();

        let summary = changes_summary(&LocalExecutor, root, &base).await.unwrap();
        assert_eq!(summary.len(), 1, "{summary:?}");
        let changed = &summary[0];
        assert_eq!(changed.path, renamed);
        assert_eq!(changed.previous_path.as_deref(), Some("keep.txt"));
        assert_eq!(changed.status, "renamed");

        let diff = changes_diff(
            &LocalExecutor,
            root,
            &base,
            &changed.path,
            changed.previous_path.as_deref(),
        )
        .await
        .unwrap();
        assert!(diff.contains("similarity index 100%"), "{diff}");

        changes_revert(
            &LocalExecutor,
            root,
            &base,
            &changed.path,
            changed.previous_path.as_deref(),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("keep.txt")).unwrap(),
            "one\ntwo\n"
        );
        assert!(!root.join(renamed).exists());
    }
}
