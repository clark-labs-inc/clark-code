use agent_core::provider::{Provider, ProviderConfig, SessionOptions};
use provider_local::{initialize_quick_chat_workspace, LocalAgentProvider};

#[tokio::test]
async fn quick_chat_uses_one_non_git_checkout_for_files_and_documents() {
    let workspace = tempfile::tempdir().expect("temporary Quick Chat workspace");
    initialize_quick_chat_workspace(workspace.path()).expect("Quick Chat marker");
    assert!(!workspace.path().join(".git").exists());

    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            // This fixture validates Quick Chat workspace identity, not a
            // platform sandbox installation. Keep it portable to clean CI
            // runners that deliberately do not bundle bubblewrap.
            extra: serde_json::json!({ "sandbox_mode": "disabled" }),
            ..Default::default()
        })
        .await
        .expect("connect local provider");
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(workspace.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .expect("open Quick Chat session");
    let environment = session.environment.expect("session environment");
    let checkout = workspace.path().canonicalize().unwrap();

    assert_eq!(environment.checkout_root.as_deref(), checkout.to_str());
    assert_eq!(environment.docs_root.as_deref(), checkout.to_str());
    assert_eq!(environment.repository_root, None);
    assert_eq!(
        environment.workspace_roots,
        vec![checkout.to_string_lossy()]
    );
}
