use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_orchestration::{
    EnterpriseId, EnterpriseSigningKey, EnterpriseTrustChain, EnterpriseTrustManifest,
};
use provider_local::{Executor, RemoteExecutor};
use scout_store::{ScoutStoreRequest, ScoutStoreResponse, SERVICE_NAME};

const TOKEN: &str = "target-service-security-token";
const ABOVE_DEFAULT_WEBSOCKET_LIMIT: usize = 16 * 1024 * 1024 + 1;

async fn start_server(root: PathBuf) -> String {
    let server = exec_server::bind(exec_server::Config {
        token: TOKEN.into(),
        root: Some(root),
        home: None,
        addr: "127.0.0.1:0".into(),
    })
    .await
    .expect("bind exec-server");
    let address = server.local_addr().expect("server address");
    tokio::spawn(server.serve());
    format!("ws://{address}")
}

fn initialize_scout_root(root: &Path) -> EnterpriseId {
    std::fs::create_dir_all(root.join("trust")).unwrap();
    std::fs::create_dir_all(root.join("batches")).unwrap();
    std::fs::create_dir_all(root.join("private")).unwrap();

    let enterprise = EnterpriseId::new("remote-target-service-security").unwrap();
    let coordinator = EnterpriseSigningKey::from_seed([0x62; 32]);
    let manifest = EnterpriseTrustManifest::initial(
        enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000062".into(),
        1,
        1_000_000,
        &coordinator,
    )
    .unwrap();
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: manifest.manifest_id.clone(),
        manifests: vec![manifest],
    };
    std::fs::write(
        root.join("trust/chain.json"),
        serde_json::to_vec(&chain).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("private/anchor-manifest-id"),
        chain.anchor_manifest_id.as_bytes(),
    )
    .unwrap();
    enterprise
}

fn rebuild_request(enterprise_id: EnterpriseId) -> Vec<u8> {
    serde_json::to_vec(&ScoutStoreRequest::Rebuild { enterprise_id }).unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_scout_root_cannot_escape_configured_remote_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let remote_root = temp.path().join("remote-root");
    let outside_scout_root = temp.path().join("outside-scout");
    std::fs::create_dir_all(&remote_root).unwrap();
    let enterprise = initialize_scout_root(&outside_scout_root);
    let symlinked_scout_root = remote_root.join("scout-link");
    symlink(&outside_scout_root, &symlinked_scout_root).unwrap();

    let url = start_server(remote_root).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();
    let error = remote
        .target_service_call(
            SERVICE_NAME,
            &symlinked_scout_root,
            &rebuild_request(enterprise),
        )
        .await
        .unwrap_err();

    assert!(
        error.contains("escapes project root or target home"),
        "{error}"
    );
    assert!(
        !outside_scout_root.join("index-v3.sqlite3").exists(),
        "the rejected request must not mutate the escaped Scout root"
    );
}

#[tokio::test]
async fn scout_target_service_request_above_16_mib_round_trips_over_loopback() {
    let temp = tempfile::tempdir().unwrap();
    let remote_root = temp.path().join("remote-root");
    let scout_root = remote_root.join("scout");
    std::fs::create_dir_all(&remote_root).unwrap();
    let enterprise = initialize_scout_root(&scout_root);
    let mut request = rebuild_request(enterprise);
    // JSON permits trailing whitespace, so this remains a valid Scout request
    // while making the single decoded service payload exactly 16 MiB + 1 byte.
    request.resize(ABOVE_DEFAULT_WEBSOCKET_LIMIT, b' ');
    assert_eq!(request.len(), ABOVE_DEFAULT_WEBSOCKET_LIMIT);

    let url = start_server(remote_root).await;
    let remote = RemoteExecutor::connect(&url, TOKEN).await.unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        remote.target_service_call(SERVICE_NAME, &scout_root, &request),
    )
    .await
    .expect("target-service transport timed out")
    .expect("large target-service request");
    let response: ScoutStoreResponse = serde_json::from_slice(&response).unwrap();

    let ScoutStoreResponse::Rebuilt(receipt) = response else {
        panic!("wrong Scout response");
    };
    assert!(receipt.rebuilt);
}
