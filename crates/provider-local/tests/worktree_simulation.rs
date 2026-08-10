mod support;

use std::path::Path;
#[cfg(unix)]
use std::time::Duration;

use agent_core::domain::AgentEvent;
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_local::{
    changes_summary, create_checkpoint, discover_repositories, inspect_repository, LocalExecutor,
};
use serde_json::json;

use support::{canonical, final_body, scripted_model, tool_call_body, GitFixture};

fn contains_path_evidence(text: &str, expected: &Path) -> bool {
    #[cfg(windows)]
    {
        let normalize = |value: &str| {
            value
                .replace("\\\\?\\UNC\\", "//")
                .replace("\\\\?\\", "")
                .replace('\\', "/")
                .to_ascii_lowercase()
        };
        normalize(text).contains(&normalize(&expected.to_string_lossy()))
    }
    #[cfg(not(windows))]
    {
        text.contains(expected.to_string_lossy().as_ref())
    }
}

#[tokio::test]
async fn discovers_linked_worktrees_outside_the_selected_checkout() {
    let fixture = GitFixture::new();

    let repositories = discover_repositories(&LocalExecutor, &fixture.detached)
        .await
        .expect("discover repository family");
    let roots = repositories
        .iter()
        .map(|repository| canonical(repository.root.as_ref()))
        .collect::<Vec<_>>();

    assert!(
        roots.contains(&fixture.main),
        "missing main checkout: {roots:?}"
    );
    assert!(
        roots.contains(&fixture.detached),
        "missing detached sibling worktree: {roots:?}"
    );
    assert!(
        roots.contains(&fixture.spaced),
        "missing sibling worktree whose path contains spaces: {roots:?}"
    );
}

#[tokio::test]
async fn detached_dirty_worktree_has_its_own_repository_state() {
    let fixture = GitFixture::new();
    fixture.make_detached_dirty();

    let detached = inspect_repository(&LocalExecutor, &fixture.detached)
        .await
        .expect("inspect detached worktree")
        .expect("detached worktree identity");
    let main = inspect_repository(&LocalExecutor, &fixture.main)
        .await
        .expect("inspect main checkout")
        .expect("main checkout identity");

    assert_eq!(canonical(detached.root.as_ref()), fixture.detached);
    assert_eq!(detached.current_branch, None);
    assert!(detached.dirty);
    assert_eq!(main.current_branch.as_deref(), Some("main"));
    assert!(!main.dirty);
    assert_eq!(detached.fingerprint, main.fingerprint);
    assert_eq!(
        detached.canonical_remote.as_deref(),
        Some("example.com/agent/simulation")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn repository_inspection_never_executes_configured_helpers() {
    let fixture = GitFixture::new();
    let helpers = fixture.install_hostile_helpers();

    let repository = tokio::time::timeout(
        Duration::from_secs(2),
        inspect_repository(&LocalExecutor, &fixture.detached),
    )
    .await
    .expect("metadata inspection must not wait for configured helpers")
    .expect("inspect hostile repository")
    .expect("hostile repository identity");

    assert_eq!(canonical(repository.root.as_ref()), fixture.detached);
    assert!(
        !helpers.fsmonitor_marker.exists(),
        "configured fsmonitor helper was executed"
    );
    assert!(
        !helpers.credential_marker.exists(),
        "configured credential helper was executed"
    );
}

#[tokio::test]
async fn checkpoint_and_change_review_are_scoped_to_the_linked_worktree() {
    let fixture = GitFixture::new();
    #[cfg(unix)]
    let helpers = fixture.install_hostile_helpers();
    let checkpoint = create_checkpoint(&LocalExecutor, &fixture.detached)
        .await
        .expect("checkpoint command")
        .expect("worktree checkpoint");

    std::fs::write(fixture.detached.join("tracked.txt"), "detached edit\n")
        .expect("edit worktree file");
    std::fs::write(fixture.detached.join("created.txt"), "worktree only\n")
        .expect("create worktree file");
    let changes = changes_summary(&LocalExecutor, &fixture.detached, &checkpoint)
        .await
        .expect("worktree changes");

    assert!(changes.iter().any(|change| change.path == "tracked.txt"));
    assert!(changes.iter().any(|change| change.path == "created.txt"));
    assert_eq!(
        std::fs::read_to_string(fixture.main.join("tracked.txt")).expect("read main file"),
        "main\n"
    );
    assert!(!fixture.main.join("created.txt").exists());
    #[cfg(unix)]
    {
        assert!(!helpers.fsmonitor_marker.exists());
        assert!(!helpers.credential_marker.exists());
    }
}

#[tokio::test]
async fn local_agent_runs_real_git_and_edits_only_the_selected_worktree() {
    let fixture = GitFixture::new();
    #[cfg(unix)]
    let helpers = fixture.install_hostile_helpers();
    // Deliberately use plain Git here. The executor must supply the safe
    // optional-lock/fsmonitor environment even when the model omits flags.
    let git_command =
        "git rev-parse --show-toplevel; git rev-parse --abbrev-ref HEAD; git status --short";
    let git_timeout_ms = if cfg!(windows) { 30_000 } else { 5_000 };
    let (base_url, captured) = scripted_model(vec![
        tool_call_body(
            "git-state",
            "bash",
            json!({"command": git_command, "timeout_ms": git_timeout_ms}),
        ),
        tool_call_body(
            "write-marker",
            "write_file",
            json!({"path": "worktree-only.txt", "content": "written by Clark Code\n"}),
        ),
        final_body("Git ran in the detached worktree and the marker was written there."),
    ])
    .await;

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": base_url,
                "model": "scripted-worktree-model",
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .expect("connect Clark Code test provider");
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(fixture.detached.to_string_lossy().into_owned()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .expect("start Clark Code in detached worktree");
    let environment = session.environment.as_ref().expect("session environment");
    assert_eq!(
        canonical(std::path::Path::new(
            environment.checkout_root.as_deref().unwrap()
        )),
        fixture.detached
    );
    assert_eq!(
        canonical(std::path::Path::new(
            environment.repository_root.as_deref().unwrap()
        )),
        fixture.main
    );
    assert_eq!(
        environment.workspace_roots[0],
        fixture.detached.to_string_lossy()
    );
    assert!(!environment.remote);
    let mut events = provider
        .prompt(
            &session.id,
            PromptInput::text("Inspect this worktree with Git, then create the requested marker."),
        )
        .await
        .expect("start scripted Clark Code run");

    let mut finished = false;
    while let Some(event) = events.next().await {
        match event {
            AgentEvent::PermissionRequest { request } => {
                provider
                    .respond(
                        &session.id,
                        ClientResponse::Permission {
                            request: request.id,
                            option: "allow_once".into(),
                            feedback: None,
                        },
                    )
                    .await
                    .expect("approve simulated tool call");
            }
            AgentEvent::RunFinished { outcome, .. } => {
                assert_eq!(outcome.status, agent_core::domain::RunStatus::Done);
                finished = true;
                break;
            }
            _ => {}
        }
    }
    assert!(finished, "scripted Clark Code run did not finish");

    assert_eq!(
        std::fs::read_to_string(fixture.detached.join("worktree-only.txt"))
            .expect("read worktree marker"),
        "written by Clark Code\n"
    );
    assert!(!fixture.main.join("worktree-only.txt").exists());
    #[cfg(unix)]
    {
        assert!(!helpers.fsmonitor_marker.exists());
        assert!(!helpers.credential_marker.exists());
    }

    let requests = captured.await.expect("collect scripted model requests");
    assert_eq!(requests.len(), 3);
    let git_result_request = requests[1].tool_results().join("\n");
    assert!(
        contains_path_evidence(&git_result_request, &fixture.detached),
        "Git tool result did not report the selected worktree: {git_result_request}"
    );
    assert!(
        git_result_request.contains("HEAD"),
        "detached HEAD evidence missing from Git tool result: {git_result_request}"
    );
    let first_user_context = requests[0].messages_for_role("user").join("\n");
    assert!(
        contains_path_evidence(&first_user_context, &fixture.detached),
        "first turn did not carry the selected checkout context: {first_user_context}"
    );
}
