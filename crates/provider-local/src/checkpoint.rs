//! Working-tree checkpoints for one-click "undo this run".
//!
//! Before each run we snapshot the project's full working state — tracked AND
//! untracked source files — as a dangling git commit built through a throwaway
//! index, so the user's real index / HEAD / stash are never touched. Restoring
//! returns the working tree to that snapshot: edits revert, files the agent
//! created are removed, files it deleted come back. Build output (gitignored) is
//! left alone, and the user's commits (HEAD) are never moved.
//!
//! Non-git projects get no checkpoint; the caller surfaces that to the user.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git unavailable: {e}"))
}

fn git_ok(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = git(root, args)?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn is_git_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn temp_index() -> PathBuf {
    std::env::temp_dir().join(format!("clark-ckpt-{}.idx", uuid::Uuid::new_v4()))
}

/// Snapshot the working tree; returns the checkpoint commit SHA, or `None` if
/// this isn't a git repo or git is unavailable.
pub fn create_checkpoint(root: &Path) -> Option<String> {
    if !is_git_repo(root) {
        return None;
    }

    // Stage everything into a THROWAWAY index so the user's real index is intact.
    let idx = temp_index();
    let staged = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["add", "-A"])
        .env("GIT_INDEX_FILE", &idx)
        .output()
        .ok()
        .filter(|o| o.status.success());
    if staged.is_none() {
        let _ = std::fs::remove_file(&idx);
        return None;
    }

    let tree = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["write-tree"])
        .env("GIT_INDEX_FILE", &idx)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let _ = std::fs::remove_file(&idx);
    let tree = tree?;
    if tree.is_empty() {
        return None;
    }

    // Build a commit for the snapshot, parented on HEAD when one exists.
    let head = git_ok(root, &["rev-parse", "HEAD"]).ok();
    let mut args: Vec<&str> = vec!["commit-tree", tree.as_str(), "-m", "clark checkpoint"];
    if let Some(h) = head.as_deref() {
        args.push("-p");
        args.push(h);
    }
    let sha = git_ok(root, &args).ok()?;
    if sha.is_empty() {
        return None;
    }

    // Anchor under a ref so `gc` can't reap it before the user undoes.
    let _ = git(
        root,
        &["update-ref", &format!("refs/clark/checkpoints/{sha}"), &sha],
    );
    Some(sha)
}

/// Whether a checkpoint commit still resolves.
pub fn checkpoint_exists(root: &Path, sha: &str) -> bool {
    !sha.is_empty()
        && git(root, &["cat-file", "-e", &format!("{sha}^{{commit}}")])
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Restore the working tree to a checkpoint. Source files match the snapshot,
/// files created since are removed, deletions are undone; gitignored output is
/// preserved and HEAD is not moved.
pub fn restore_checkpoint(root: &Path, sha: &str) -> Result<(), String> {
    if !is_git_repo(root) {
        return Err("Undo needs a git repository.".to_string());
    }
    if !checkpoint_exists(root, sha) {
        return Err("This checkpoint is no longer available.".to_string());
    }
    let tree = format!("{sha}^{{tree}}");
    // Working tree + index -> checkpoint tree (restores & deletes tracked paths).
    git_ok(root, &["read-tree", "-u", "--reset", &tree])?;
    // Remove source files created since the checkpoint (respects .gitignore).
    git_ok(root, &["clean", "-fd"])?;
    // Unstage so the restored state shows as a normal working diff vs HEAD.
    let _ = git(root, &["reset", "-q"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn run(root: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?}");
    }

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    #[test]
    fn non_git_dir_has_no_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        assert!(create_checkpoint(dir.path()).is_none());
        assert!(restore_checkpoint(dir.path(), "deadbeef").is_err());
    }

    #[test]
    fn checkpoint_restores_edits_additions_and_deletions() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-q"]);
        fs::write(root.join("keep.txt"), "original\n").unwrap();
        fs::write(root.join("delete_me.txt"), "bye\n").unwrap();
        run(root, &["add", "-A"]);
        run(root, &["commit", "-qm", "init"]);

        // Snapshot the clean state.
        let sha = create_checkpoint(root).expect("checkpoint");
        assert!(checkpoint_exists(root, &sha));

        // Agent makes a mess: edit, create, delete.
        fs::write(root.join("keep.txt"), "MANGLED\n").unwrap();
        fs::write(root.join("new_file.txt"), "agent made this\n").unwrap();
        fs::remove_file(root.join("delete_me.txt")).unwrap();

        // Undo.
        restore_checkpoint(root, &sha).expect("restore");

        assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "original\n");
        assert!(!root.join("new_file.txt").exists(), "created file should be gone");
        assert_eq!(
            fs::read_to_string(root.join("delete_me.txt")).unwrap(),
            "bye\n",
            "deleted file should be restored"
        );
    }

    #[test]
    fn checkpoint_captures_untracked_files() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-q"]);
        fs::write(root.join("a.txt"), "a\n").unwrap();
        run(root, &["add", "-A"]);
        run(root, &["commit", "-qm", "init"]);
        // Untracked file present at checkpoint time.
        fs::write(root.join("untracked.txt"), "keep me\n").unwrap();

        let sha = create_checkpoint(root).expect("checkpoint");
        fs::remove_file(root.join("untracked.txt")).unwrap();
        restore_checkpoint(root, &sha).expect("restore");

        assert_eq!(
            fs::read_to_string(root.join("untracked.txt")).unwrap(),
            "keep me\n",
            "untracked file in the snapshot should be restored"
        );
    }
}
