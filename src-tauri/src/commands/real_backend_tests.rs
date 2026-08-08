use std::process::Command as StdCommand;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = StdCommand::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git available");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

fn init_repo(dir: &std::path::Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
    std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "initial"]);
}

#[tokio::test]
async fn list_commands_discovers_a_real_claude_commands_file() {
    let dir = tempfile::tempdir().unwrap();
    let cmd_dir = dir.path().join(".claude/commands");
    std::fs::create_dir_all(&cmd_dir).unwrap();
    std::fs::write(
        cmd_dir.join("review.md"),
        "---\ndescription: Review the current diff.\n---\n\nReview the current diff for bugs.",
    )
    .unwrap();

    let found = provider_local::discover_commands(&provider_local::LocalExecutor, dir.path()).await;
    let review = found
        .iter()
        .find(|c| c.name == "review")
        .expect("the real .claude/commands/review.md was discovered");
    assert_eq!(review.description, "Review the current diff.");
    assert_eq!(review.body, "Review the current diff for bugs.");
}

#[tokio::test]
async fn list_commands_is_empty_for_a_project_with_no_commands_dir() {
    let dir = tempfile::tempdir().unwrap();
    let found = provider_local::discover_commands(&provider_local::LocalExecutor, dir.path()).await;
    assert!(found.is_empty());
}

#[tokio::test]
async fn changes_summary_and_diff_see_a_real_edit_against_a_real_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    // A real checkpoint, via the exact function `engine.rs` calls at the
    // start of every turn.
    let base = provider_local::create_checkpoint(&provider_local::LocalExecutor, dir.path())
        .await
        .expect("checkpoint command succeeds")
        .expect("real git repo checkpoints successfully");

    // A real, independent edit after the checkpoint.
    std::fs::write(
        dir.path().join("main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("new_file.rs"), "// new\n").unwrap();

    let cwd = dir.path().to_string_lossy().to_string();
    let summary = provider_local::changes_summary(
        &provider_local::LocalExecutor,
        std::path::Path::new(&cwd),
        &base,
    )
    .await
    .expect("changes_summary succeeds against a real checkpoint");
    assert!(summary
        .iter()
        .any(|f| f.path == "main.rs" && f.status == "modified"));
    assert!(summary
        .iter()
        .any(|f| f.path == "new_file.rs" && f.status == "added"));

    let diff = provider_local::changes_diff(
        &provider_local::LocalExecutor,
        std::path::Path::new(&cwd),
        &base,
        "main.rs",
        None,
    )
    .await
    .expect("changes_diff succeeds");
    assert!(
        diff.contains("println"),
        "real diff should show the real edit: {diff}"
    );

    // Revert just the one file — the real filesystem should show the
    // original content again, and the new file should be untouched.
    provider_local::changes_revert(
        &provider_local::LocalExecutor,
        std::path::Path::new(&cwd),
        &base,
        "main.rs",
        None,
    )
    .await
    .expect("changes_revert succeeds");
    let restored = std::fs::read_to_string(dir.path().join("main.rs")).unwrap();
    assert_eq!(restored, "fn main() {}\n");
    assert!(dir.path().join("new_file.rs").exists());
}
