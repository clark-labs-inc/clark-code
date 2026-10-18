use super::*;

async fn commit(
    state: &AppState,
    expected: Option<&str>,
    generation: u64,
    next: &str,
) -> Result<(), String> {
    commit_account_session(
        state,
        expected,
        generation,
        "https://product.invalid".into(),
        "account-a".into(),
        "opaque-bearer".into(),
        next.into(),
    )
    .await
}

#[tokio::test]
async fn renewal_can_commit_inside_an_admitted_cloud_read() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    state.credentials.configure(root.path().into()).unwrap();
    commit(&state, None, 0, r#"{"revision":1}"#).await.unwrap();
    let _read = state.account_lifecycle.read().await;
    let generation = state.runtime_registry.cloud_account_generation();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        commit(
            &state,
            Some(r#"{"revision":1}"#),
            generation,
            r#"{"revision":2}"#,
        ),
    )
    .await
    .expect("renewal must not upgrade the cloud lifecycle read lock")
    .unwrap();
    assert_eq!(
        state
            .credentials
            .retained_auth()
            .await
            .unwrap()
            .unwrap()
            .as_str(),
        r#"{"revision":2}"#
    );
}

#[tokio::test]
async fn stale_renewal_cannot_overwrite_a_newer_session() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    state.credentials.configure(root.path().into()).unwrap();
    commit(&state, None, 0, r#"{"revision":1}"#).await.unwrap();
    let observed = state.runtime_registry.cloud_account_generation();
    commit(
        &state,
        Some(r#"{"revision":1}"#),
        observed,
        r#"{"revision":2}"#,
    )
    .await
    .unwrap();
    let error = commit(
        &state,
        Some(r#"{"revision":1}"#),
        observed,
        r#"{"revision":3}"#,
    )
    .await
    .unwrap_err();
    assert_eq!(
        serde_json::from_str::<ProductAuthFailure>(&error)
            .unwrap()
            .code,
        ProductAuthFailureCode::AccountChanged
    );
    assert_eq!(
        state
            .credentials
            .retained_auth()
            .await
            .unwrap()
            .unwrap()
            .as_str(),
        r#"{"revision":2}"#
    );
}

#[tokio::test]
async fn late_initial_sign_in_cannot_resurrect_an_explicit_sign_out() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    state.credentials.configure(root.path().into()).unwrap();
    let initial_generation = state.runtime_registry.cloud_account_generation();
    // The signed-out credential value is None both before and after sign-out;
    // the generation must still invalidate an already-open browser sign-in.
    let mut generation = state
        .runtime_registry
        .cloud_account_generation_write()
        .await;
    state.credentials.sign_out(None).await.unwrap();
    *generation = None;
    drop(generation);
    assert!(
        commit(&state, None, initial_generation, r#"{"revision":1}"#)
            .await
            .is_err()
    );
    assert!(state.credentials.retained_auth().await.unwrap().is_none());
    assert!(state.runtime_registry.cloud_account().await.is_none());
}
