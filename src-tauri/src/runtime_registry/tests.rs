use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use agent_core::provider::EventStream;
use agent_core::{
    ClientResponse, Error, PromptInput, Provider, ProviderCapabilities, ProviderConfig, ProviderId,
    RunId, Session, SessionId, SessionOptions, Snapshot,
};
use futures::stream::{self, StreamExt};
use tokio::sync::Mutex;

use super::{
    retryable_connect_error, AccountKey, CloudAccountState, ConnectCircuits, ProjectKey,
    RuntimeRegistry, SessionKey, WorkerHandle, WorkerKey, CIRCUIT_OPEN_DURATION,
    MAX_CIRCUIT_ENTRIES,
};
use crate::state::HostSession;

struct CloseRecordingProvider(Arc<AtomicBool>);

#[async_trait::async_trait]
impl Provider for CloseRecordingProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("shutdown-test")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    async fn connect(&mut self, _config: ProviderConfig) -> agent_core::Result<()> {
        Ok(())
    }

    async fn new_session(&mut self, _options: SessionOptions) -> agent_core::Result<Session> {
        Err(Error::Unsupported("not used by this test".into()))
    }

    async fn load_session(&mut self, _id: SessionId) -> agent_core::Result<Session> {
        Err(Error::Unsupported("not used by this test".into()))
    }

    async fn prompt(
        &mut self,
        _session: &SessionId,
        _input: PromptInput,
    ) -> agent_core::Result<EventStream> {
        Ok(stream::empty().boxed())
    }

    async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> agent_core::Result<()> {
        Ok(())
    }

    async fn close_session(&mut self, _session: &SessionId) -> agent_core::Result<()> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn respond(
        &mut self,
        _session: &SessionId,
        _response: ClientResponse,
    ) -> agent_core::Result<()> {
        Ok(())
    }
}

fn test_session(account: AccountKey, id: &str) -> Arc<Mutex<HostSession>> {
    Arc::new(Mutex::new(HostSession {
        account: Some(account),
        provider: Box::new(CloseRecordingProvider(Arc::new(AtomicBool::new(false)))),
        session: Session {
            id: SessionId::new(id),
            provider: ProviderId::new("partition-test"),
            capabilities: ProviderCapabilities::default(),
            mode: None,
            collaboration_mode: Default::default(),
            environment: None,
        },
        snapshot: Snapshot::default(),
        trajectory: None,
        projection_gate: Arc::new(Mutex::new(())),
        closing: false,
    }))
}

#[test]
fn account_keys_reject_empty_or_control_bearing_owners() {
    assert!(AccountKey::new("").is_err());
    assert!(AccountKey::new("account\nother").is_err());
    assert!(AccountKey::new("account-a").is_ok());
}

#[test]
fn session_keys_reject_unbounded_or_control_bearing_wire_values() {
    assert!(SessionKey::parse("").is_err());
    assert!(SessionKey::parse("session\nother").is_err());
    assert!(SessionKey::parse("s".repeat(513)).is_err());
    assert_eq!(
        SessionKey::parse("conversation-1").unwrap().as_str(),
        "conversation-1"
    );
}

#[test]
fn project_keys_accept_only_bounded_portable_worker_ids() {
    assert!(ProjectKey::parse("").is_err());
    assert!(ProjectKey::parse("project/other").is_err());
    assert!(ProjectKey::parse("p".repeat(129)).is_err());
    assert_eq!(
        ProjectKey::parse("project-1").unwrap().as_str(),
        "project-1"
    );
}

#[test]
fn worker_handles_are_strict_opaque_capabilities() {
    assert!(WorkerHandle::parse("worker-0123456789abcdef0123456789abcdef").is_ok());
    assert!(WorkerHandle::parse("headless-0123456789abcdef0123456789abcdef").is_err());
    assert!(WorkerHandle::parse("worker-../../account-a").is_err());
}

#[tokio::test]
async fn empty_registry_has_no_live_resources() {
    let registry = RuntimeRegistry::new();
    assert_eq!(registry.worker_count().await, 0);
    assert!(registry.session_entries().await.is_empty());
    assert_eq!(registry.shutdown_all().await, Default::default());
}

#[tokio::test]
async fn identical_session_ids_are_isolated_by_native_account_partition() {
    let registry = RuntimeRegistry::new();
    let account_a = AccountKey::new("account-a").unwrap();
    let account_b = AccountKey::new("account-b").unwrap();
    let session_key = SessionKey::parse("shared-conversation").unwrap();
    let session_a = test_session(account_a.clone(), session_key.as_str());
    let session_b = test_session(account_b.clone(), session_key.as_str());

    registry
        .bind_session(
            Some(account_a.clone()),
            session_key.clone(),
            session_a.clone(),
        )
        .await
        .unwrap();
    registry
        .bind_session(
            Some(account_b.clone()),
            session_key.clone(),
            session_b.clone(),
        )
        .await
        .unwrap();
    registry
        .set_cloud_account(Some(CloudAccountState {
            rest_base: "https://product.example".into(),
            account: account_a.clone(),
            token: zeroize::Zeroizing::new("token-a".into()),
        }))
        .await;
    assert!(Arc::ptr_eq(
        &registry.current_session_entry(&session_key).await.unwrap(),
        &session_a
    ));

    let removed = registry.take_account_sessions(&account_a).await;
    assert_eq!(removed.len(), 1);
    assert!(Arc::ptr_eq(&removed[0], &session_a));
    registry
        .set_cloud_account(Some(CloudAccountState {
            rest_base: "https://product.example".into(),
            account: account_b,
            token: zeroize::Zeroizing::new("token-b".into()),
        }))
        .await;
    assert!(Arc::ptr_eq(
        &registry.current_session_entry(&session_key).await.unwrap(),
        &session_b
    ));
}

#[tokio::test]
async fn skill_catalog_authorities_are_account_partitioned_and_retired() {
    let registry = RuntimeRegistry::new();
    let account_a = AccountKey::new("account-a").unwrap();
    let account_b = AccountKey::new("account-b").unwrap();
    registry
        .set_cloud_account(Some(CloudAccountState {
            rest_base: "https://product.example".into(),
            account: account_a.clone(),
            token: zeroize::Zeroizing::new("token-a".into()),
        }))
        .await;
    let catalog_a = registry.current_skill_catalogs().await;
    registry
        .set_cloud_account(Some(CloudAccountState {
            rest_base: "https://product.example".into(),
            account: account_b,
            token: zeroize::Zeroizing::new("token-b".into()),
        }))
        .await;
    let catalog_b = registry.current_skill_catalogs().await;
    assert!(!Arc::ptr_eq(&catalog_a, &catalog_b));

    registry.disconnect_account(&account_a).await;
    registry
        .set_cloud_account(Some(CloudAccountState {
            rest_base: "https://product.example".into(),
            account: account_a,
            token: zeroize::Zeroizing::new("token-a-2".into()),
        }))
        .await;
    let replacement_a = registry.current_skill_catalogs().await;
    assert!(!Arc::ptr_eq(&catalog_a, &replacement_a));
    let shutdown = registry.shutdown_all().await;
    assert_eq!(shutdown.skill_catalogs, 2);
}

#[tokio::test]
async fn session_publication_rejects_a_mismatched_account_partition() {
    let registry = RuntimeRegistry::new();
    let account_a = AccountKey::new("account-a").unwrap();
    let account_b = AccountKey::new("account-b").unwrap();
    let session_key = SessionKey::parse("conversation-1").unwrap();

    let Err(error) = registry
        .bind_session(
            Some(account_a),
            session_key,
            test_session(account_b, "conversation-1"),
        )
        .await
    else {
        panic!("mismatched account partition must fail");
    };

    assert!(error.contains("does not match"));
    assert!(registry.session_entries().await.is_empty());
}

#[tokio::test]
async fn whole_app_shutdown_unpublishes_and_closes_live_sessions() {
    let registry = RuntimeRegistry::new();
    let closed = Arc::new(AtomicBool::new(false));
    let session_id = SessionId::new("shutdown-session");
    let session_key = SessionKey::from_session(&session_id).unwrap();
    registry
        .bind_session(
            Some(AccountKey::new("account-a").unwrap()),
            session_key,
            Arc::new(Mutex::new(HostSession {
                account: Some(AccountKey::new("account-a").unwrap()),
                provider: Box::new(CloseRecordingProvider(closed.clone())),
                session: Session {
                    id: session_id,
                    provider: ProviderId::new("shutdown-test"),
                    capabilities: ProviderCapabilities::default(),
                    mode: None,
                    collaboration_mode: Default::default(),
                    environment: None,
                },
                snapshot: Snapshot::default(),
                trajectory: None,
                projection_gate: Arc::new(Mutex::new(())),
                closing: false,
            })),
        )
        .await
        .unwrap();

    let receipt = registry.shutdown_all().await;

    assert_eq!(receipt.sessions, 1);
    assert_eq!(receipt.workers, 0);
    assert!(closed.load(Ordering::SeqCst));
    assert!(registry.session_entries().await.is_empty());
}

#[tokio::test]
async fn account_generation_transition_blocks_readers_until_it_is_complete() {
    let registry = std::sync::Arc::new(RuntimeRegistry::new());
    registry
        .set_cloud_account(Some(CloudAccountState {
            rest_base: "https://product.example".into(),
            account: AccountKey::new("account-a").unwrap(),
            token: zeroize::Zeroizing::new("native-token".into()),
        }))
        .await;
    let mut generation = registry.cloud_account_generation_write().await;
    let reading = {
        let registry = registry.clone();
        tokio::spawn(async move { registry.cloud_account().await })
    };
    tokio::task::yield_now().await;
    assert!(!reading.is_finished());

    *generation = None;
    drop(generation);
    assert!(reading.await.unwrap().is_none());
}

#[tokio::test]
async fn command_claims_are_native_account_and_host_bound() {
    let registry = RuntimeRegistry::new();
    let account_a = AccountKey::new("account-a").unwrap();
    let account_b = AccountKey::new("account-b").unwrap();
    registry
        .store_command_claim(
            account_a.clone(),
            "command-a".into(),
            "host-a".into(),
            "instance-a".into(),
            "claim-secret".into(),
        )
        .await
        .unwrap();

    assert_eq!(
        registry
            .command_claim(&account_a, "command-a", "host-a", "instance-a")
            .await
            .unwrap(),
        "claim-secret"
    );
    assert!(registry
        .command_claim(&account_b, "command-a", "host-a", "instance-a")
        .await
        .is_err());
    assert!(registry
        .command_claim(&account_a, "command-a", "host-b", "instance-a")
        .await
        .is_err());
    registry.remove_command_claim(&account_a, "command-a").await;
    assert!(registry
        .command_claim(&account_a, "command-a", "host-a", "instance-a")
        .await
        .is_err());
}

#[tokio::test]
async fn expired_single_flight_gates_are_reclaimed() {
    let registry = RuntimeRegistry::new();
    let first = registry.connect_gate(&WorkerKey("first".into())).await;
    assert_eq!(registry.connect_gates.lock().await.len(), 1);
    drop(first);

    let _second = registry.connect_gate(&WorkerKey("second".into())).await;
    let gates = registry.connect_gates.lock().await;
    assert_eq!(gates.len(), 1);
    assert!(gates.contains_key(&WorkerKey("second".into())));
}

#[test]
fn repeated_failures_open_a_bounded_expiring_circuit() {
    let mut circuits = ConnectCircuits::default();
    let key = WorkerKey("worker-a".into());
    let now = Instant::now();
    for offset in 0..3 {
        circuits.record_failure(key.clone(), now + Duration::from_millis(offset));
    }
    assert!(circuits.permit(&key, now + Duration::from_secs(1)).is_err());
    assert!(circuits
        .permit(&key, now + CIRCUIT_OPEN_DURATION + Duration::from_millis(3))
        .is_ok());

    for index in 0..(MAX_CIRCUIT_ENTRIES + 4) {
        circuits.record_failure(
            WorkerKey(format!("worker-{index}")),
            now + Duration::from_secs(index as u64),
        );
    }
    assert!(circuits.states.len() <= MAX_CIRCUIT_ENTRIES);
}

#[test]
fn native_retry_policy_is_typed_and_bounded_to_transient_connect_failures() {
    assert!(retryable_connect_error(
        &code_remote::RemoteWorkerError::Io("reset".into())
    ));
    assert!(retryable_connect_error(
        &code_remote::RemoteWorkerError::Artifact("ssh failed".into())
    ));
    assert!(retryable_connect_error(
        &code_remote::RemoteWorkerError::Transport("control master reset".into())
    ));
    assert!(!retryable_connect_error(
        &code_remote::RemoteWorkerError::Spec("invalid".into())
    ));
    assert!(!retryable_connect_error(
        &code_remote::RemoteWorkerError::CredentialInvalid("TOKEN".into())
    ));
}
