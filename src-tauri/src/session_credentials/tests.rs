use std::collections::HashMap;

use super::SessionCredentials;

#[tokio::test]
async fn encrypted_credentials_persist_without_plaintext_and_are_account_partitioned() {
    let root = tempfile::tempdir().unwrap();
    let first = SessionCredentials::new();
    first.configure(root.path().join("credentials")).unwrap();
    first
        .set_code_key("account-a", "ck_live_account_a_secret".into())
        .await
        .unwrap();
    first
        .set_retained_auth(r#"{"token":"cloud-session-secret"}"#.into())
        .await
        .unwrap();
    assert!(first.code_key("account-b").await.unwrap().is_none());

    let encrypted = std::fs::read(root.path().join("credentials/credentials.enc")).unwrap();
    assert!(!String::from_utf8_lossy(&encrypted).contains("ck_live_account_a_secret"));
    assert!(!String::from_utf8_lossy(&encrypted).contains("cloud-session-secret"));

    let reopened = SessionCredentials::new();
    reopened.configure(root.path().join("credentials")).unwrap();
    assert_eq!(
        reopened
            .code_key("account-a")
            .await
            .unwrap()
            .unwrap()
            .as_str(),
        "ck_live_account_a_secret"
    );
    assert_eq!(
        reopened.retained_auth().await.unwrap().unwrap().as_str(),
        r#"{"token":"cloud-session-secret"}"#
    );
    reopened.sign_out(Some("account-a")).await.unwrap();
    assert!(reopened.retained_auth().await.unwrap().is_none());
    assert!(reopened.code_key("account-a").await.unwrap().is_none());
}

#[tokio::test]
async fn tampering_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let credentials = SessionCredentials::new();
    credentials
        .configure(root.path().join("credentials"))
        .unwrap();
    credentials
        .set_code_key("account-a", "ck_live_account_a_secret".into())
        .await
        .unwrap();
    let path = root.path().join("credentials/credentials.enc");
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(path, bytes).unwrap();

    let reopened = SessionCredentials::new();
    reopened.configure(root.path().join("credentials")).unwrap();
    assert!(reopened.code_key("account-a").await.is_err());
}

#[tokio::test]
async fn obsolete_webview_credential_payload_is_deleted_not_migrated() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("credentials/credentials.enc");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"CLKCRD01obsolete-bearer-payload").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let credentials = SessionCredentials::new();
    credentials
        .configure(root.path().join("credentials"))
        .unwrap();

    assert!(credentials.retained_auth().await.unwrap().is_none());
    assert!(!path.exists());
}

#[tokio::test]
async fn mcp_secrets_persist_by_account_and_blank_values_retain_existing_secrets() {
    let root = tempfile::tempdir().unwrap();
    let credentials = SessionCredentials::new();
    credentials
        .configure(root.path().join("credentials"))
        .unwrap();
    credentials
        .sync_mcp_environment(
            "account-a",
            HashMap::from([(
                "github-server".into(),
                HashMap::from([("GITHUB_TOKEN".into(), "secret-a".into())]),
            )]),
        )
        .await
        .unwrap();
    credentials
        .sync_mcp_environment(
            "account-a",
            HashMap::from([(
                "github-server".into(),
                HashMap::from([("GITHUB_TOKEN".into(), String::new())]),
            )]),
        )
        .await
        .unwrap();

    let reopened = SessionCredentials::new();
    reopened.configure(root.path().join("credentials")).unwrap();
    assert_eq!(
        reopened
            .mcp_environment("account-a", "github-server", &["GITHUB_TOKEN".into()])
            .await
            .unwrap(),
        HashMap::from([("GITHUB_TOKEN".into(), "secret-a".into())])
    );
    assert!(reopened
        .mcp_environment("account-b", "github-server", &["GITHUB_TOKEN".into()])
        .await
        .is_err());

    let encrypted = std::fs::read(root.path().join("credentials/credentials.enc")).unwrap();
    assert!(!String::from_utf8_lossy(&encrypted).contains("secret-a"));
}
