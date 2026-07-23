//! Real Git workflow through the WebSocket executor, including checkpoint-backed
//! change review. This covers the same path used by remote Changes UI commands.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use exec_core::Executor;
use exec_server::Config;
use provider_local::{
    changes_summary, create_checkpoint, list_project_files, load_index, memory_dir, RemoteExecutor,
};
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "remote-git-test-token";

async fn start_server(root: PathBuf) -> String {
    let server = exec_server::bind(Config {
        token: TOKEN.to_string(),
        root: Some(root),
        home: None,
        addr: "127.0.0.1:0".into(),
    })
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(server.serve());
    format!("ws://{addr}")
}

#[tokio::test]
async fn remote_checkpoint_review_round_trip() {
    let fixture = support::GitFixture::new();
    let main = fixture.main.clone();
    let root = fixture.detached.clone();
    #[cfg(unix)]
    let helpers = fixture.install_hostile_helpers();
    let remote = RemoteExecutor::connect(&start_server(root.clone()).await, TOKEN)
        .await
        .unwrap();
    let status = remote
        .exec_streaming_pty(
            "git status --short",
            &root,
            Duration::from_secs(2),
            &CancellationToken::new(),
            &|_, _| {},
        )
        .await
        .expect("plain remote Git must not wait for the configured fsmonitor helper");
    assert_eq!(status.code, Some(0));
    #[cfg(unix)]
    {
        assert!(!helpers.fsmonitor_marker.exists());
        assert!(!helpers.credential_marker.exists());
    }
    let files = list_project_files(&remote, &root).await;
    assert!(files.contains(&"tracked.txt".to_string()));
    let memory = memory_dir(&root);
    std::fs::create_dir_all(&memory).unwrap();
    std::fs::write(memory.join("MEMORY.md"), "- remote project fact\n").unwrap();
    assert_eq!(
        load_index(&remote, &memory).await.as_deref(),
        Some("- remote project fact")
    );
    let checkpoint = create_checkpoint(&remote, &root)
        .await
        .unwrap()
        .expect("remote Git checkpoint");
    assert!(!walkdir::WalkDir::new(main.join(".git"))
        .into_iter()
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with("clark-checkpoint-")));

    std::fs::write(root.join("tracked.txt"), "remote edit\n").unwrap();
    std::fs::write(root.join("created.txt"), "remote new\n").unwrap();
    let changes = changes_summary(&remote, &root, &checkpoint).await.unwrap();
    assert!(changes.iter().any(|change| change.path == "tracked.txt"));
    assert!(changes.iter().any(|change| change.path == "created.txt"));

    assert_eq!(
        std::fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "remote edit\n"
    );
    assert!(root.join("created.txt").exists());
    assert_eq!(
        std::fs::read_to_string(main.join("tracked.txt")).unwrap(),
        "main\n"
    );
}
