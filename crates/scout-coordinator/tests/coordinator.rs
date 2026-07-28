use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};
use std::thread;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseBatchBundle, EnterpriseEntityKind, EnterpriseEvent,
    EnterpriseFact, EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseTrustChain, EnterpriseTrustManifest, GraphEntityObservation,
};
use scout_accumulator::{verify_proof, ProofStatus};
use scout_coordinator::CoordinatorStore;
use scout_ingest_protocol::{CoordinatorSigningKey, IngestRequest, ScoutTenantId};

struct Fixture {
    tenant_id: ScoutTenantId,
    enterprise_id: EnterpriseId,
    chain: EnterpriseTrustChain,
    grant: EnterpriseSignerGrant,
    collector: EnterpriseSigningKey,
}

impl Fixture {
    fn new(enterprise: &str) -> Self {
        let enterprise_id = EnterpriseId::new(enterprise).unwrap();
        let administrator = EnterpriseSigningKey::from_seed([7; 32]);
        let collector = EnterpriseSigningKey::from_seed([8; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise_id.clone(),
            format!(
                "trust:{}",
                if enterprise == "acme" {
                    "a".repeat(64)
                } else {
                    "b".repeat(64)
                }
            ),
            100,
            100_000,
            &administrator,
        )
        .unwrap();
        let grant = EnterpriseSignerGrant::issue(
            &manifest,
            collector.signer_id(),
            collector.public_key_hex(),
            BTreeSet::from([EnterpriseSignerRole::Collector]),
            EnterpriseGrantScope {
                machine_id: "machine-a".into(),
                run_id: "run-a".into(),
                adapter_instance_id: "aws-prod".into(),
                auth_context_id: "auth-read-only".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: 1,
                last_source_sequence: 10_000,
            },
            100,
            90_000,
            &[&administrator],
        )
        .unwrap();
        Self {
            tenant_id: ScoutTenantId::new(format!("organization:{enterprise}")).unwrap(),
            enterprise_id,
            chain: EnterpriseTrustChain {
                anchor_manifest_id: manifest.manifest_id.clone(),
                manifests: vec![manifest],
            },
            grant,
            collector,
        }
    }

    fn request(&self, sequence: u64) -> IngestRequest {
        let observation = GraphEntityObservation::new(
            &self.enterprise_id,
            EnterpriseEntityKind::Service,
            AuthorityRef::new(
                "aws",
                "account:prod",
                format!("service:checkout-{sequence}"),
            )
            .unwrap(),
            BTreeSet::from([format!("checkout-{sequence}")]),
            BTreeSet::from([format!("{sequence:064x}")]),
        )
        .unwrap();
        let event = EnterpriseEvent::new(
            self.enterprise_id.clone(),
            EnterpriseProvenance {
                machine_id: "machine-a".into(),
                run_id: "run-a".into(),
                adapter_instance_id: "aws-prod".into(),
                auth_context_id: "auth-read-only".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                source_sequence: sequence,
                observed_at_ms: 1_000 + sequence,
                source_fingerprint: format!("{:064x}", sequence + 10_000),
            },
            EnterpriseFact::EntityObserved(observation),
        )
        .unwrap();
        let batch = EnterpriseBatch::new(self.enterprise_id.clone(), [event]).unwrap();
        let signed_batch = EnterpriseSignedBatch::sign(
            batch,
            &self.chain.manifests[0],
            self.grant.clone(),
            1_000 + sequence,
            &self.collector,
        )
        .unwrap();
        IngestRequest::new(
            self.tenant_id.clone(),
            format!("outbox-attempt:{:064x}", sequence + 20_000),
            EnterpriseBatchBundle {
                trust_chain: self.chain.clone(),
                signed_batch,
            },
        )
        .unwrap()
    }
}

fn coordinator_store(root: &std::path::Path) -> CoordinatorStore {
    CoordinatorStore::open(root, CoordinatorSigningKey::from_seed([42; 32])).unwrap()
}

#[test]
fn pin_is_explicit_and_signing_identity_is_durable() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("acme");
    let store = coordinator_store(root.path());
    assert!(store
        .status(&fixture.tenant_id, &fixture.enterprise_id)
        .unwrap()
        .is_none());
    assert!(store
        .pin_enterprise(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            "trust-manifest:wrong",
            &fixture.chain,
        )
        .is_err());
    let status = store
        .pin_enterprise(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.chain.anchor_manifest_id,
            &fixture.chain,
        )
        .unwrap();
    assert_eq!(status.trust_generation, 1);
    assert_eq!(status.accepted_batches, 0);
    assert_eq!(
        store
            .pin_enterprise(
                &fixture.tenant_id,
                &fixture.enterprise_id,
                &fixture.chain.anchor_manifest_id,
                &fixture.chain,
            )
            .unwrap(),
        status
    );
    assert!(
        CoordinatorStore::open(root.path(), CoordinatorSigningKey::from_seed([43; 32])).is_err()
    );
}

#[test]
fn duplicate_and_crash_retry_return_the_exact_durable_receipt() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("acme");
    let store = coordinator_store(root.path());
    store
        .pin_enterprise(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.chain.anchor_manifest_id,
            &fixture.chain,
        )
        .unwrap();
    let request = fixture.request(1);
    let receipt = store.ingest(&fixture.tenant_id, &request, 10_000).unwrap();
    assert_eq!(receipt.sequence, 1);
    assert_eq!(receipt.issued_at_ms, 10_000);

    let mut retried = request.clone();
    retried.attempt_id = format!("outbox-attempt:{}", "f".repeat(64));
    assert_eq!(
        store.ingest(&fixture.tenant_id, &retried, 20_000).unwrap(),
        receipt
    );
    drop(store);

    let reopened = coordinator_store(root.path());
    assert_eq!(
        reopened
            .ingest(&fixture.tenant_id, &retried, 30_000)
            .unwrap(),
        receipt
    );
    assert_eq!(
        reopened
            .receipt(
                &fixture.tenant_id,
                &fixture.enterprise_id,
                receipt.batch_id.as_str(),
            )
            .unwrap()
            .unwrap(),
        receipt
    );
    assert_eq!(
        reopened
            .status(&fixture.tenant_id, &fixture.enterprise_id)
            .unwrap()
            .unwrap()
            .accepted_batches,
        1
    );
}

#[test]
fn identical_enterprise_names_are_isolated_by_authorization_tenant() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("acme");
    let other_tenant = ScoutTenantId::new("organization:other").unwrap();
    let store = coordinator_store(root.path());
    for tenant in [&fixture.tenant_id, &other_tenant] {
        store
            .pin_enterprise(
                tenant,
                &fixture.enterprise_id,
                &fixture.chain.anchor_manifest_id,
                &fixture.chain,
            )
            .unwrap();
    }
    let first = store
        .ingest(&fixture.tenant_id, &fixture.request(1), 10_000)
        .unwrap();
    let mut second_request = fixture.request(1);
    second_request.tenant_id = other_tenant.clone();
    assert!(store
        .ingest(&fixture.tenant_id, &second_request, 10_000)
        .is_err());
    let second = store
        .ingest(&other_tenant, &second_request, 10_000)
        .unwrap();
    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 1);
    assert_ne!(first.receipt_id, second.receipt_id);
    assert_eq!(
        store
            .status(&fixture.tenant_id, &fixture.enterprise_id)
            .unwrap()
            .unwrap()
            .accepted_batches,
        1
    );
    assert_eq!(
        store
            .status(&other_tenant, &fixture.enterprise_id)
            .unwrap()
            .unwrap()
            .accepted_batches,
        1
    );
}

#[test]
fn conflicting_envelope_and_unpinned_enterprise_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("acme");
    let store = coordinator_store(root.path());
    store
        .pin_enterprise(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.chain.anchor_manifest_id,
            &fixture.chain,
        )
        .unwrap();
    let request = fixture.request(1);
    store.ingest(&fixture.tenant_id, &request, 10_000).unwrap();

    let mut conflicting = request.clone();
    conflicting.bundle.signed_batch = EnterpriseSignedBatch::sign(
        conflicting.bundle.signed_batch.batch.clone(),
        &fixture.chain.manifests[0],
        conflicting.bundle.signed_batch.grant.clone(),
        conflicting.bundle.signed_batch.signed_at_ms + 1,
        &fixture.collector,
    )
    .unwrap();
    conflicting.validate().unwrap();
    assert!(store
        .ingest(&fixture.tenant_id, &conflicting, 10_001)
        .is_err());

    let other = Fixture::new("other");
    assert!(store
        .ingest(&other.tenant_id, &other.request(1), 10_000)
        .is_err());
    assert_eq!(
        store
            .status(&fixture.tenant_id, &fixture.enterprise_id)
            .unwrap()
            .unwrap()
            .accepted_batches,
        1
    );
}

#[test]
fn concurrent_machine_uploads_form_one_monotonic_receipt_chain() {
    const BATCHES: u64 = 24;
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("acme");
    let store = coordinator_store(root.path());
    store
        .pin_enterprise(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.chain.anchor_manifest_id,
            &fixture.chain,
        )
        .unwrap();
    let requests = (1..=BATCHES)
        .map(|sequence| fixture.request(sequence))
        .collect::<Vec<_>>();
    let barrier = Arc::new(Barrier::new(BATCHES as usize));
    let mut threads = Vec::new();
    for request in requests {
        let worker = store.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            worker.ingest(&request.tenant_id, &request, 20_000)
        }));
    }
    let mut receipts = threads
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    receipts.sort_by_key(|receipt| receipt.sequence);
    assert_eq!(receipts.len(), BATCHES as usize);
    for (index, receipt) in receipts.iter().enumerate() {
        assert_eq!(receipt.sequence, index as u64 + 1);
        assert_eq!(receipt.batch_accumulator_count, receipt.sequence);
        assert_eq!(
            receipt.previous_receipt_id.as_ref(),
            index
                .checked_sub(1)
                .map(|previous| &receipts[previous].receipt_id)
        );
        receipt.verify(&store.coordinator_public_key()).unwrap();
    }
    let status = store
        .status(&fixture.tenant_id, &fixture.enterprise_id)
        .unwrap()
        .unwrap();
    assert_eq!(status.accepted_batches, BATCHES);
    assert_eq!(status.next_sequence, BATCHES + 1);
    assert_eq!(
        status.last_receipt_id.as_ref(),
        receipts.last().map(|receipt| &receipt.receipt_id)
    );
    assert_eq!(
        status.batch_accumulator_root,
        receipts.last().unwrap().batch_accumulator_root
    );
    for receipt in &receipts {
        let proof = store
            .batch_proof(
                &fixture.tenant_id,
                &fixture.enterprise_id,
                receipt.batch_id.as_str(),
            )
            .unwrap();
        assert_eq!(proof.root.digest.to_string(), status.batch_accumulator_root);
        assert_eq!(
            verify_proof(&proof.root, &proof.proof).unwrap(),
            ProofStatus::Member
        );
    }
    let missing = store
        .batch_proof(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &format!("batch:{}", "0".repeat(64)),
        )
        .unwrap();
    assert_eq!(
        verify_proof(&missing.root, &missing.proof).unwrap(),
        ProofStatus::NonMember
    );
}
