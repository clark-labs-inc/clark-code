use super::*;

#[tokio::test]
async fn research_journals_do_not_change_scan_or_working_tree_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Research Test"],
        vec!["config", "user.email", "research@example.com"],
    ] {
        git(root, &args);
    }
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "initial\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "fixture"]);
    std::fs::write(root.join("src/lib.rs"), "reviewed\n").unwrap();

    for scope in [root.to_path_buf(), root.join("src")] {
        let baseline = snapshots(root, &scope).await;
        for journal_root in [
            root.join(".agent/research-trees"),
            root.join("src/.agent/research-trees"),
        ] {
            std::fs::create_dir_all(&journal_root).unwrap();
            std::fs::write(journal_root.join("tree.json"), "{\"events\":[1]}").unwrap();
            std::fs::write(journal_root.join("tree.lock"), "").unwrap();
            std::fs::write(journal_root.join(".pending-write"), "partial").unwrap();
        }
        assert_eq!(baseline, snapshots(root, &scope).await);
        std::fs::write(
            root.join("src/.agent/research-trees/tree.json"),
            "{\"events\":[1,2]}",
        )
        .unwrap();
        assert_eq!(baseline, snapshots(root, &scope).await);
        std::fs::write(root.join("src/lib.rs"), "changed since review\n").unwrap();
        let changed = snapshots(root, &scope).await;
        assert_ne!(baseline.0, changed.0, "source edits invalidate inventory");
        assert_ne!(baseline.1, changed.1, "source edits invalidate diff target");
        std::fs::write(root.join("src/lib.rs"), "reviewed\n").unwrap();
    }
}

#[test]
fn research_output_exclusion_does_not_hide_similarly_named_source_paths() {
    for path in [
        ".agent/research-trees/tree.json",
        "src/.agent/research-trees/tree.json",
        "nested/src/.agent/research-trees",
    ] {
        assert!(is_security_output(path), "{path}");
    }
    for path in [
        ".agent/research-trees.rs",
        ".agent/research-trees-source/tree.json",
        "src/not.agent/research-trees/tree.json",
        "src/.agent/research-tree/tree.json",
    ] {
        assert!(!is_security_output(path), "{path}");
    }
}

async fn snapshots(root: &Path, scope: &Path) -> (String, String) {
    let inventory = collect_security_inventory(&crate::exec::LocalExecutor, root, scope)
        .await
        .unwrap();
    let diff = collect_security_diff_inventory(
        &crate::exec::LocalExecutor,
        root,
        scope,
        SecurityDiffKind::WorkingTree,
        "HEAD",
        None,
    )
    .await
    .unwrap();
    assert_eq!(inventory.paths, ["src/lib.rs"]);
    assert_eq!(diff.changed_files.len(), 1);
    assert_eq!(diff.changed_files[0].path, "src/lib.rs");
    (inventory.inventory_id, diff.target.target_id)
}

fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
