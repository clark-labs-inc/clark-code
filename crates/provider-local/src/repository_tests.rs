use super::*;
use crate::exec::LocalExecutor;

async fn git(root: &Path, command: &str) {
    let output = tokio::process::Command::new("git")
        .args(command.split_whitespace())
        .current_dir(root)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test]
async fn identifies_clone_equivalent_repository_by_remote() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), "init").await;
    git(dir.path(), "config user.name Clark").await;
    git(dir.path(), "config user.email clark@example.com").await;
    tokio::fs::write(dir.path().join("README.md"), "hello")
        .await
        .unwrap();
    git(dir.path(), "add README.md").await;
    git(dir.path(), "commit -m initial").await;
    git(
        dir.path(),
        "remote add origin git@github.com:Clark-Labs-Inc/Clark.git",
    )
    .await;

    let identity = inspect_repository(&LocalExecutor, dir.path())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        identity.canonical_remote.as_deref(),
        Some("github.com/clark-labs-inc/clark")
    );
    assert!(identity.fingerprint.starts_with("git:"));
    assert_eq!(identity.commit_count, 1);

    let relocated = tempfile::tempdir().unwrap();
    git(relocated.path(), "init").await;
    git(relocated.path(), "config user.name Clark").await;
    git(relocated.path(), "config user.email clark@example.com").await;
    tokio::fs::write(relocated.path().join("README.md"), "hello elsewhere")
        .await
        .unwrap();
    git(relocated.path(), "add README.md").await;
    git(relocated.path(), "commit -m relocated").await;
    git(
        relocated.path(),
        "remote add origin https://github.com/clark-labs-inc/clark.git",
    )
    .await;

    let relocated_identity = inspect_repository(&LocalExecutor, relocated.path())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(identity.root, relocated_identity.root);
    assert_eq!(identity.fingerprint, relocated_identity.fingerprint);
}

#[tokio::test]
async fn history_is_paged_and_preserves_commit_metadata() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), "init").await;
    git(dir.path(), "config user.name Clark").await;
    git(dir.path(), "config user.email clark@example.com").await;
    for index in 0..3 {
        tokio::fs::write(dir.path().join("value.txt"), index.to_string())
            .await
            .unwrap();
        git(dir.path(), "add value.txt").await;
        git(dir.path(), &format!("commit -m commit-{index}")).await;
    }

    let first = load_git_history(&LocalExecutor, dir.path(), 0, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.commits.len(), 2);
    assert!(!first.complete);
    assert_eq!(first.next_offset, 2);
    assert_eq!(first.commits[0].author_name, "Clark");
    let second = load_git_history(&LocalExecutor, dir.path(), first.next_offset, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.commits.len(), 1);
    assert!(second.complete);
}

#[tokio::test]
async fn working_tree_snapshot_lists_dirty_and_untracked() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), "init").await;
    git(dir.path(), "config user.name Clark").await;
    git(dir.path(), "config user.email clark@example.com").await;
    tokio::fs::write(dir.path().join("a.txt"), "one")
        .await
        .unwrap();
    git(dir.path(), "add a.txt").await;
    git(dir.path(), "commit -m initial").await;

    // Clean tree says so explicitly (so the model doesn't guess).
    let clean = working_tree_snapshot(&LocalExecutor, dir.path())
        .await
        .unwrap();
    assert!(clean.contains("Branch: "));
    assert!(clean.contains("No uncommitted changes."));

    // A modified and an untracked file both show up as entries.
    tokio::fs::write(dir.path().join("a.txt"), "two")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("b.txt"), "new")
        .await
        .unwrap();
    let dirty = working_tree_snapshot(&LocalExecutor, dir.path())
        .await
        .unwrap();
    assert!(dirty.contains("a.txt"), "{dirty}");
    assert!(dirty.contains("b.txt"), "{dirty}");
    assert!(dirty.contains("leave them alone"), "{dirty}");
}

#[tokio::test]
async fn working_tree_snapshot_is_none_outside_git() {
    let dir = tempfile::tempdir().unwrap();
    assert!(working_tree_snapshot(&LocalExecutor, dir.path())
        .await
        .is_none());
}

#[tokio::test]
async fn working_tree_snapshot_preserves_every_change_beyond_the_old_cap() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), "init").await;
    for index in 0..45 {
        tokio::fs::write(dir.path().join(format!("change-{index}.txt")), "new")
            .await
            .unwrap();
    }

    let snapshot = working_tree_snapshot(&LocalExecutor, dir.path())
        .await
        .unwrap();
    assert!(snapshot.contains("change-0.txt"), "{snapshot}");
    assert!(snapshot.contains("change-44.txt"), "{snapshot}");
    assert!(!snapshot.contains("… and "), "{snapshot}");
}

#[tokio::test]
async fn discovers_nested_git_repositories() {
    let parent = tempfile::tempdir().unwrap();
    for name in ["one", "two"] {
        let root = parent.path().join(name);
        tokio::fs::create_dir_all(&root).await.unwrap();
        git(&root, "init").await;
        git(&root, "config user.name Clark").await;
        git(&root, "config user.email clark@example.com").await;
        tokio::fs::write(root.join("README.md"), name)
            .await
            .unwrap();
        git(&root, "add README.md").await;
        git(&root, "commit -m initial").await;
    }

    let repositories = discover_repositories(&LocalExecutor, parent.path())
        .await
        .unwrap();
    assert_eq!(repositories.len(), 2);
}

#[tokio::test]
async fn discovers_repositories_beyond_the_old_depth_cap() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent
        .path()
        .join("one/two/three/four/five/six/seven/eight/nine/deep-repo");
    tokio::fs::create_dir_all(&root).await.unwrap();
    git(&root, "init").await;

    let repositories = discover_repositories(&LocalExecutor, parent.path())
        .await
        .unwrap();
    let canonical_root = tokio::fs::canonicalize(&root).await.unwrap();
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0].root, canonical_root.to_string_lossy());
}

#[test]
fn remote_sanitization_removes_credentials_and_normalizes_identity() {
    let (url, canonical) = sanitize_remote("https://token@example.com/Org/Repo.git").unwrap();
    assert!(!url.contains("token"));
    assert_eq!(canonical, "example.com/org/repo");
}
