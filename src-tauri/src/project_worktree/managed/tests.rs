use super::{
    cleanup_managed_worktree, project_managed_worktree_create, project_managed_worktree_list,
    project_managed_worktree_save_branch, project_worktree_transition_plan, ManagedWorktreeBase,
    ManagedWorktreeRequest, ManagedWorktreeState, WorktreePreservation, WorktreeTransitionAction,
};
use crate::{state::HostSession, AppState};
use agent_core::{
    provider::{ProviderCapabilities, Session, SessionEnvironment},
    ProviderId, SessionId, Snapshot,
};
use std::{path::Path, process::Command, sync::Arc};
use tokio::sync::Mutex;

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn initialized_repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("project");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "--initial-branch=main"]);
    git(&repo, &["config", "user.email", "test@example.local"]);
    git(&repo, &["config", "user.name", "Agent Test"]);
    std::fs::write(repo.join("README.md"), "initial\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    temp
}

#[tokio::test]
async fn dirty_checkout_plan_requires_explicit_preservation() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    std::fs::write(repo.join("README.md"), "local change\n").unwrap();

    let plan = project_worktree_transition_plan(repo.to_string_lossy().into_owned(), None)
        .await
        .unwrap();

    assert_eq!(plan.action, WorktreeTransitionAction::CreateIsolated);
    assert_eq!(
        plan.preservation,
        WorktreePreservation::ChangesRemainInSource
    );
    assert!(plan.requires_confirmation);
    assert_eq!(plan.source_changes.changed_files, 1);
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "local change\n"
    );
}

#[tokio::test]
async fn dirty_branch_transition_starts_a_branch_backed_target_continuation() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    git(&repo, &["switch", "-qc", "feature/target"]);
    std::fs::write(repo.join("README.md"), "target branch\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "target"]);
    git(&repo, &["switch", "-q", "main"]);
    std::fs::write(repo.join("README.md"), "source remains dirty\n").unwrap();
    let source_status = git_text(&repo, &["status", "--short"]);

    let plan = project_worktree_transition_plan(
        repo.to_string_lossy().into_owned(),
        Some("feature/target".into()),
    )
    .await
    .unwrap();
    assert_eq!(plan.action, WorktreeTransitionAction::PreserveChanges);
    assert_eq!(
        plan.preservation,
        WorktreePreservation::ChangesRemainInSource
    );

    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: Some("target-continuation".into()),
            target_branch: Some("feature/target".into()),
        },
    )
    .await
    .unwrap();
    let continuation = Path::new(&created.path);
    assert_eq!(
        git_text(continuation, &["branch", "--show-current"]),
        format!("agent/{}", created.id)
    );
    assert_eq!(
        std::fs::read_to_string(continuation.join("README.md")).unwrap(),
        "target branch\n"
    );
    assert_eq!(git_text(&repo, &["status", "--short"]), source_status);
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "source remains dirty\n"
    );
}

#[tokio::test]
async fn occupied_branch_plan_routes_to_its_owner() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    let owner = temp.path().join("main-owner");
    git(&repo, &["switch", "-qc", "feature/local"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            owner.to_string_lossy().as_ref(),
            "main",
        ],
    );

    let plan =
        project_worktree_transition_plan(repo.to_string_lossy().into_owned(), Some("main".into()))
            .await
            .unwrap();

    assert_eq!(plan.action, WorktreeTransitionAction::OpenOwner);
    assert_eq!(plan.preservation, WorktreePreservation::OwnerCheckout);
    assert_eq!(
        plan.target_checkout_path.as_deref(),
        Some(owner.canonicalize().unwrap().to_string_lossy().as_ref())
    );
    assert_eq!(
        git_text(&repo, &["branch", "--show-current"]),
        "feature/local"
    );
}

#[tokio::test]
async fn managed_checkout_cannot_be_repurposed_by_switching_its_branch() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    git(&repo, &["branch", "feature/other"]);
    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: Some("pinned".into()),
            target_branch: None,
        },
    )
    .await
    .unwrap();

    let error =
        project_worktree_transition_plan(created.path.clone(), Some("feature/other".into()))
            .await
            .unwrap_err();
    assert!(error.contains("managed checkout is pinned"));
    assert_eq!(
        git_text(Path::new(&created.path), &["branch", "--show-current"]),
        format!("agent/{}", created.id)
    );
}

#[tokio::test]
async fn unavailable_branch_owner_requires_manual_repair() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    let owner = temp.path().join("main-owner");
    git(&repo, &["switch", "-qc", "feature/local"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            owner.to_string_lossy().as_ref(),
            "main",
        ],
    );
    std::fs::remove_dir_all(&owner).unwrap();

    let error =
        project_worktree_transition_plan(repo.to_string_lossy().into_owned(), Some("main".into()))
            .await
            .unwrap_err();

    assert!(error.contains("registered to unavailable checkout"));
    assert_eq!(
        git_text(&repo, &["branch", "--show-current"]),
        "feature/local"
    );
}

#[tokio::test]
async fn creates_and_explicitly_cleans_a_branch_backed_managed_worktree() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    std::fs::write(repo.join("README.md"), "source stays dirty\n").unwrap();
    let source_status = git_text(&repo, &["status", "--short"]);

    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: None,
            target_branch: None,
        },
    )
    .await
    .unwrap();
    let created_path = Path::new(&created.path);
    assert!(created_path.is_dir());
    assert!(created.id.starts_with("project-main-"), "{}", created.id);
    assert_eq!(created.label, "project-main");
    assert_eq!(
        git_text(created_path, &["branch", "--show-current"]),
        format!("agent/{}", created.id)
    );
    assert_eq!(
        created.preserved_branch.as_deref(),
        Some(format!("agent/{}", created.id).as_str())
    );
    assert_eq!(git_text(&repo, &["status", "--short"]), source_status);
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "source stays dirty\n"
    );

    let listed = project_managed_worktree_list(repo.to_string_lossy().into_owned())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert_eq!(listed[0].state, ManagedWorktreeState::Ready);
    let managed_plan = project_worktree_transition_plan(created.path.clone(), None)
        .await
        .unwrap();
    assert!(managed_plan.source_is_managed);
    let nested_error = project_managed_worktree_create(
        created.path.clone(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: Some("nested".into()),
            target_branch: None,
        },
    )
    .await
    .unwrap_err();
    assert!(nested_error.contains("already a app-managed isolated worktree"));

    let project_path = repo.to_string_lossy().into_owned();
    let receipt = cleanup_managed_worktree(&project_path, &created.id, &AppState::new())
        .await
        .unwrap();
    assert!(receipt.removed);
    assert!(!created_path.exists());
    assert_eq!(git_text(&repo, &["status", "--short"]), source_status);
}

#[tokio::test]
async fn default_base_uses_the_repository_default_branch_not_the_feature_head() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    git(&repo, &["switch", "-qc", "feature/local"]);
    std::fs::write(repo.join("README.md"), "feature head\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "feature"]);
    let source_head = git_text(&repo, &["rev-parse", "HEAD"]);

    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Default,
            label: Some("default-base".into()),
            target_branch: None,
        },
    )
    .await
    .unwrap();

    let created_path = Path::new(&created.path);
    assert_eq!(created.base_reference, "main");
    assert_eq!(
        std::fs::read_to_string(created_path.join("README.md")).unwrap(),
        "initial\n"
    );
    assert_eq!(
        git_text(created_path, &["branch", "--show-current"]),
        format!("agent/{}", created.id)
    );
    assert_eq!(
        git_text(&repo, &["branch", "--show-current"]),
        "feature/local"
    );
    assert_eq!(git_text(&repo, &["rev-parse", "HEAD"]), source_head);
}

#[tokio::test]
async fn default_base_refreshes_the_remote_default_before_creating_a_session() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    let remote = temp.path().join("origin.git");
    let remote_path = remote.to_string_lossy().into_owned();
    git(temp.path(), &["init", "--bare", "-q", &remote_path]);
    git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(&repo, &["remote", "add", "origin", &remote_path]);
    git(&repo, &["push", "-qu", "origin", "main"]);

    let publisher = temp.path().join("publisher");
    let publisher_path = publisher.to_string_lossy().into_owned();
    git(temp.path(), &["clone", "-q", &remote_path, &publisher_path]);
    git(&publisher, &["config", "user.email", "test@example.local"]);
    git(&publisher, &["config", "user.name", "Agent Test"]);
    std::fs::write(publisher.join("README.md"), "fresh remote default\n").unwrap();
    git(&publisher, &["add", "README.md"]);
    git(&publisher, &["commit", "-qm", "fresh default"]);
    git(&publisher, &["push", "-q", "origin", "main"]);
    let remote_head = git_text(&publisher, &["rev-parse", "HEAD"]);

    git(&repo, &["switch", "-qc", "feature/local"]);
    std::fs::write(repo.join("README.md"), "local feature\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "feature head"]);

    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Default,
            label: Some("fresh-default".into()),
            target_branch: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(created.base_reference, "origin/main");
    assert_eq!(created.base_revision, remote_head);
    assert_eq!(
        std::fs::read_to_string(Path::new(&created.path).join("README.md")).unwrap(),
        "fresh remote default\n"
    );
}

#[tokio::test]
async fn cleanup_refuses_a_dirty_managed_worktree_without_force() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: None,
            target_branch: None,
        },
    )
    .await
    .unwrap();
    std::fs::write(Path::new(&created.path).join("scratch.txt"), "keep\n").unwrap();

    let project_path = repo.to_string_lossy().into_owned();
    let error = cleanup_managed_worktree(&project_path, &created.id, &AppState::new())
        .await
        .unwrap_err();
    assert!(error.contains("has local changes"));
    assert!(Path::new(&created.path).is_dir());
    assert_eq!(
        project_managed_worktree_list(repo.to_string_lossy().into_owned())
            .await
            .unwrap()[0]
            .state,
        ManagedWorktreeState::Dirty
    );
}

#[tokio::test]
async fn cleanup_refuses_a_clean_externally_detached_worktree_with_unprotected_commits() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: Some("private-commit".into()),
            target_branch: None,
        },
    )
    .await
    .unwrap();
    let managed = Path::new(&created.path);
    // A normal Agent Desktop checkout is branch-backed. Simulate an external detach to
    // prove the recovery gate still protects an old or manually altered one.
    git(managed, &["switch", "--detach", "-q"]);
    std::fs::write(managed.join("README.md"), "private commit\n").unwrap();
    git(managed, &["add", "README.md"]);
    git(managed, &["commit", "-qm", "private managed commit"]);

    let project_path = repo.to_string_lossy().into_owned();
    let listed = project_managed_worktree_list(project_path.clone())
        .await
        .unwrap();
    assert_eq!(listed[0].state, ManagedWorktreeState::Committed);
    assert_ne!(
        listed[0].head_revision.as_deref(),
        Some(created.base_revision.as_str())
    );
    assert_eq!(
        listed[0].preserved_branch.as_deref(),
        Some(format!("agent/{}", created.id).as_str())
    );

    let error = cleanup_managed_worktree(&project_path, &created.id, &AppState::new())
        .await
        .unwrap_err();
    assert!(error.contains("new commits that are not protected"));
    assert!(managed.is_dir());
}

#[tokio::test]
async fn saving_externally_detached_commits_as_a_branch_allows_archiving_the_checkout() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: Some("ship-me".into()),
            target_branch: None,
        },
    )
    .await
    .unwrap();
    let managed = Path::new(&created.path);
    git(managed, &["switch", "--detach", "-q"]);
    std::fs::write(managed.join("README.md"), "ship this\n").unwrap();
    git(managed, &["add", "README.md"]);
    git(managed, &["commit", "-qm", "ship managed commit"]);
    let private_head = git_text(managed, &["rev-parse", "HEAD"]);

    let project_path = repo.to_string_lossy().into_owned();
    let saved = project_managed_worktree_save_branch(project_path.clone(), created.id.clone())
        .await
        .unwrap();
    assert_eq!(saved.branch, format!("agent/{}-saved", created.id));
    assert_eq!(saved.head_revision, private_head);
    assert_eq!(
        git_text(
            &repo,
            &["rev-parse", &format!("refs/heads/{}", saved.branch)]
        ),
        private_head
    );

    let listed = project_managed_worktree_list(project_path.clone())
        .await
        .unwrap();
    assert_eq!(listed[0].state, ManagedWorktreeState::Saved);
    assert_eq!(
        listed[0].preserved_branch.as_deref(),
        Some(saved.branch.as_str())
    );

    cleanup_managed_worktree(&project_path, &created.id, &AppState::new())
        .await
        .unwrap();
    assert!(!managed.exists());
    assert_eq!(
        git_text(
            &repo,
            &["rev-parse", &format!("refs/heads/{}", saved.branch)]
        ),
        private_head
    );
}

#[tokio::test]
async fn branch_backed_commits_remain_safe_to_archive() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: Some("branch-safe".into()),
            target_branch: None,
        },
    )
    .await
    .unwrap();
    let managed = Path::new(&created.path);
    std::fs::write(managed.join("README.md"), "safe branch commit\n").unwrap();
    git(managed, &["add", "README.md"]);
    git(managed, &["commit", "-qm", "managed branch commit"]);
    let committed_head = git_text(managed, &["rev-parse", "HEAD"]);
    let project_path = repo.to_string_lossy().into_owned();

    let listed = project_managed_worktree_list(project_path.clone())
        .await
        .unwrap();
    assert_eq!(listed[0].state, ManagedWorktreeState::Saved);
    let branch = listed[0].preserved_branch.as_deref().unwrap();
    assert_eq!(git_text(&repo, &["rev-parse", branch]), committed_head);

    cleanup_managed_worktree(&project_path, &created.id, &AppState::new())
        .await
        .unwrap();
    assert!(!managed.exists());
    assert_eq!(git_text(&repo, &["rev-parse", branch]), committed_head);
}

#[tokio::test]
async fn cleanup_refuses_an_idle_live_session_using_the_checkout() {
    let temp = initialized_repo();
    let repo = temp.path().join("project");
    let created = project_managed_worktree_create(
        repo.to_string_lossy().into_owned(),
        ManagedWorktreeRequest {
            base: ManagedWorktreeBase::Current,
            label: Some("idle-chat".into()),
            target_branch: None,
        },
    )
    .await
    .unwrap();
    let state = AppState::new();
    let session = Session {
        id: SessionId::new("idle-chat"),
        provider: ProviderId::new("local"),
        capabilities: ProviderCapabilities::default(),
        mode: None,
        collaboration_mode: Default::default(),
        environment: Some(SessionEnvironment {
            checkout_root: Some(created.path.clone()),
            ..Default::default()
        }),
    };
    state
        .runtime_registry
        .bind_session(
            None,
            crate::runtime_registry::SessionKey::parse("idle-chat").unwrap(),
            Arc::new(Mutex::new(HostSession {
                account: None,
                provider: Box::new(provider_local::LocalAgentProvider::new()),
                session,
                snapshot: Snapshot::new(),
                trajectory: None,
                projection_gate: Arc::new(Mutex::new(())),
                closing: false,
            })),
        )
        .await
        .unwrap();

    let project_path = repo.to_string_lossy().into_owned();
    let error = cleanup_managed_worktree(&project_path, &created.id, &state)
        .await
        .unwrap_err();
    assert!(error.contains("1 live desktop session"));
    assert!(Path::new(&created.path).is_dir());
}
