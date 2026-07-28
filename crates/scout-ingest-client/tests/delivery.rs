use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseBatchId, EnterpriseEntityKind, EnterpriseEvent,
    EnterpriseFact, EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseTrustChain, EnterpriseTrustManifest, GraphEntityObservation,
};
use async_trait::async_trait;
use scout_coordinator::CoordinatorStore;
use scout_ingest_client::{enqueue, CentralIngestTransport, ScoutIngestClient};
use scout_ingest_protocol::{IngestReceipt, IngestRequest, ScoutTenantId};
use scout_store::{OutboxState, ScoutStoreRequest, ScoutStoreResponse};

struct Fixture {
    tenant_id: ScoutTenantId,
    enterprise_id: EnterpriseId,
    chain: EnterpriseTrustChain,
    envelope: EnterpriseSignedBatch,
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let enterprise_id = EnterpriseId::new("delivery-enterprise").unwrap();
        let signer = EnterpriseSigningKey::from_seed([21; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise_id.clone(),
            format!("trust:{}", "d".repeat(64)),
            100,
            100_000,
            &signer,
        )
        .unwrap();
        let chain = EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest.clone()],
        };
        let observation = GraphEntityObservation::new(
            &enterprise_id,
            EnterpriseEntityKind::Service,
            AuthorityRef::new("aws", "account:prod", "service:delivery").unwrap(),
            BTreeSet::from(["delivery".into()]),
            BTreeSet::from(["e".repeat(64)]),
        )
        .unwrap();
        let event = EnterpriseEvent::new(
            enterprise_id.clone(),
            EnterpriseProvenance {
                machine_id: "machine-delivery".into(),
                run_id: "run-delivery".into(),
                adapter_instance_id: "aws-prod".into(),
                auth_context_id: "auth-read-only".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                source_sequence: 1,
                observed_at_ms: 1_000,
                source_fingerprint: "f".repeat(64),
            },
            EnterpriseFact::EntityObserved(observation),
        )
        .unwrap();
        let batch = EnterpriseBatch::new(enterprise_id.clone(), [event]).unwrap();
        let grant = EnterpriseSignerGrant::issue(
            &manifest,
            signer.signer_id(),
            signer.public_key_hex(),
            BTreeSet::from([
                EnterpriseSignerRole::Collector,
                EnterpriseSignerRole::Coordinator,
            ]),
            EnterpriseGrantScope {
                machine_id: "machine-delivery".into(),
                run_id: "run-delivery".into(),
                adapter_instance_id: "aws-prod".into(),
                auth_context_id: "auth-read-only".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: 1,
                last_source_sequence: 1,
            },
            100,
            90_000,
            &[&signer],
        )
        .unwrap();
        let envelope =
            EnterpriseSignedBatch::sign(batch, &manifest, grant, 1_000, &signer).unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("trust")).unwrap();
        std::fs::create_dir_all(root.path().join("private")).unwrap();
        std::fs::write(
            root.path().join("trust/chain.json"),
            serde_json::to_vec(&chain).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.path().join("private/anchor-manifest-id"),
            chain.anchor_manifest_id.as_bytes(),
        )
        .unwrap();
        let response = scout_store::request(
            root.path(),
            ScoutStoreRequest::Ingest {
                enterprise_id: enterprise_id.clone(),
                envelope: Box::new(envelope.clone()),
            },
        )
        .unwrap();
        assert!(matches!(response, ScoutStoreResponse::Ingested { .. }));
        Self {
            tenant_id: ScoutTenantId::new("organization:delivery").unwrap(),
            enterprise_id,
            chain,
            envelope,
            root,
        }
    }

    fn batch_id(&self) -> EnterpriseBatchId {
        self.envelope.batch.batch_id.clone()
    }

    fn status(&self) -> OutboxState {
        let response = scout_store::request(
            self.root.path(),
            ScoutStoreRequest::OutboxStatus {
                enterprise_id: self.enterprise_id.clone(),
                batch_id: self.batch_id(),
            },
        )
        .unwrap();
        let ScoutStoreResponse::OutboxStatus { entry: Some(entry) } = response else {
            panic!("missing outbox entry")
        };
        entry.state
    }
}

struct TestTransport {
    coordinator: CoordinatorStore,
    lose_response_once: Arc<AtomicBool>,
    forge_response_once: Arc<AtomicBool>,
}

impl TestTransport {
    fn reliable(coordinator: CoordinatorStore) -> Self {
        Self {
            coordinator,
            lose_response_once: Arc::new(AtomicBool::new(false)),
            forge_response_once: Arc::new(AtomicBool::new(false)),
        }
    }

    fn lose_response_once(coordinator: CoordinatorStore) -> Self {
        Self {
            lose_response_once: Arc::new(AtomicBool::new(true)),
            ..Self::reliable(coordinator)
        }
    }

    fn forge_response_once(coordinator: CoordinatorStore) -> Self {
        Self {
            forge_response_once: Arc::new(AtomicBool::new(true)),
            ..Self::reliable(coordinator)
        }
    }
}

#[async_trait]
impl CentralIngestTransport for TestTransport {
    async fn submit(&self, request: &IngestRequest) -> Result<IngestReceipt, String> {
        let mut receipt = self
            .coordinator
            .ingest(&request.tenant_id, request, 20_000)?;
        if self.lose_response_once.swap(false, Ordering::SeqCst) {
            return Err("connection closed after commit".into());
        }
        if self.forge_response_once.swap(false, Ordering::SeqCst) {
            receipt.envelope_sha256 = "0".repeat(64);
        }
        Ok(receipt)
    }
}

fn coordinator(root: &std::path::Path, fixture: &Fixture) -> CoordinatorStore {
    let coordinator = CoordinatorStore::open(
        root,
        scout_ingest_protocol::CoordinatorSigningKey::from_seed([31; 32]),
    )
    .unwrap();
    coordinator
        .pin_enterprise(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.chain.anchor_manifest_id,
            &fixture.chain,
        )
        .unwrap();
    coordinator
}

fn attempt() -> String {
    format!("outbox-attempt:{}", "1".repeat(64))
}

#[tokio::test]
async fn response_loss_after_commit_recovers_without_duplicate_acceptance() {
    let fixture = Fixture::new();
    let central_root = tempfile::tempdir().unwrap();
    let coordinator = coordinator(central_root.path(), &fixture);
    enqueue(
        fixture.root.path(),
        &fixture.enterprise_id,
        &fixture.batch_id(),
    )
    .unwrap();
    let lossy = ScoutIngestClient::new(
        fixture.root.path(),
        fixture.tenant_id.clone(),
        coordinator.coordinator_public_key(),
        TestTransport::lose_response_once(coordinator.clone()),
    );
    assert!(lossy
        .deliver(
            &fixture.enterprise_id,
            &fixture.batch_id(),
            &attempt(),
            None
        )
        .await
        .unwrap_err()
        .contains("connection closed"));
    assert_eq!(
        fixture.status(),
        OutboxState::InFlight {
            attempt_id: attempt()
        }
    );
    assert_eq!(
        coordinator
            .status(&fixture.tenant_id, &fixture.enterprise_id)
            .unwrap()
            .unwrap()
            .accepted_batches,
        1
    );

    let reliable = ScoutIngestClient::new(
        fixture.root.path(),
        fixture.tenant_id.clone(),
        coordinator.coordinator_public_key(),
        TestTransport::reliable(coordinator.clone()),
    );
    let recovered = reliable
        .deliver(
            &fixture.enterprise_id,
            &fixture.batch_id(),
            &attempt(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(recovered.receipt.sequence, 1);
    assert!(matches!(fixture.status(), OutboxState::Acked { .. }));
    assert_eq!(
        coordinator
            .status(&fixture.tenant_id, &fixture.enterprise_id)
            .unwrap()
            .unwrap()
            .accepted_batches,
        1
    );
}

#[tokio::test]
async fn forged_receipt_never_resolves_the_local_outbox() {
    let fixture = Fixture::new();
    let central_root = tempfile::tempdir().unwrap();
    let coordinator = coordinator(central_root.path(), &fixture);
    enqueue(
        fixture.root.path(),
        &fixture.enterprise_id,
        &fixture.batch_id(),
    )
    .unwrap();
    let client = ScoutIngestClient::new(
        fixture.root.path(),
        fixture.tenant_id.clone(),
        coordinator.coordinator_public_key(),
        TestTransport::forge_response_once(coordinator.clone()),
    );
    assert!(client
        .deliver(
            &fixture.enterprise_id,
            &fixture.batch_id(),
            &attempt(),
            None
        )
        .await
        .is_err());
    assert!(matches!(fixture.status(), OutboxState::InFlight { .. }));

    let recovery = ScoutIngestClient::new(
        fixture.root.path(),
        fixture.tenant_id.clone(),
        coordinator.coordinator_public_key(),
        TestTransport::reliable(coordinator),
    );
    recovery
        .deliver(
            &fixture.enterprise_id,
            &fixture.batch_id(),
            &attempt(),
            None,
        )
        .await
        .unwrap();
    assert!(matches!(fixture.status(), OutboxState::Acked { .. }));
}
