use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::scout::enterprise::contract::{
    AuthorityRef, EnterpriseBatch, EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact,
    EnterpriseId, EnterpriseProvenance, GraphEntityObservation,
};

fn fixture(
    roles: BTreeSet<EnterpriseSignerRole>,
) -> (
    EnterpriseTrustChain,
    EnterpriseSignerGrant,
    EnterpriseSigningKey,
    EnterpriseBatch,
) {
    let enterprise = EnterpriseId::new("acme").unwrap();
    let coordinator = EnterpriseSigningKey::from_seed([7; 32]);
    let collector = EnterpriseSigningKey::from_seed([8; 32]);
    let manifest = EnterpriseTrustManifest::initial(
        enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000001".into(),
        100,
        10_000,
        &coordinator,
    )
    .unwrap();
    let scope = EnterpriseGrantScope {
        machine_id: "machine-a".into(),
        run_id: "run-a".into(),
        adapter_instance_id: "aws-prod".into(),
        auth_context_id: "auth-read-only".into(),
        discovery_epoch: "epoch-1".into(),
        discovery_epoch_sequence: 1,
        first_source_sequence: 1,
        last_source_sequence: 100,
    };
    let grant = EnterpriseSignerGrant::issue(
        &manifest,
        collector.signer_id(),
        collector.public_key_hex(),
        roles,
        scope,
        100,
        5_000,
        &[&coordinator],
    )
    .unwrap();
    let observation = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:prod", "service:checkout").unwrap(),
        BTreeSet::from(["checkout".into()]),
        BTreeSet::from(["a".repeat(64)]),
    )
    .unwrap();
    let event = EnterpriseEvent::new(
        enterprise.clone(),
        EnterpriseProvenance {
            machine_id: "machine-a".into(),
            run_id: "run-a".into(),
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
    let batch = EnterpriseBatch::new(enterprise, [event]).unwrap();
    (
        EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest],
        },
        grant,
        collector,
        batch,
    )
}

#[test]
fn signed_collector_batch_verifies_against_pinned_chain() {
    let (chain, grant, collector, batch) =
        fixture(BTreeSet::from([EnterpriseSignerRole::Collector]));
    let envelope =
        EnterpriseSignedBatch::sign(batch, &chain.manifests[0], grant, 1_000, &collector).unwrap();
    let verified = chain.verify_signed_batch(envelope).unwrap();
    assert_eq!(verified.batch().events.len(), 1);
}

#[test]
fn signing_key_matches_the_rfc8032_public_key_vector() {
    let seed = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    assert_eq!(
        EnterpriseSigningKey::from_seed(seed).public_key_hex(),
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
    );
}

#[test]
fn signer_proposal_proves_key_possession_and_rejects_tampering() {
    let (chain, grant, collector, _) = fixture(BTreeSet::from([EnterpriseSignerRole::Collector]));
    let proposal = EnterpriseSignerProposal::create(
        grant.enterprise_id,
        chain.anchor_manifest_id,
        grant.roles,
        grant.scope,
        grant.not_before_ms,
        grant.expires_at_ms,
        &collector,
    )
    .unwrap();
    proposal.verify().unwrap();

    let mut tampered = proposal;
    tampered.scope.machine_id = "substituted-machine".into();
    assert!(tampered.verify().is_err());
}

#[test]
fn tampering_and_cross_enterprise_replay_fail_closed() {
    let (chain, grant, collector, batch) =
        fixture(BTreeSet::from([EnterpriseSignerRole::Collector]));
    let envelope =
        EnterpriseSignedBatch::sign(batch, &chain.manifests[0], grant, 1_000, &collector).unwrap();

    let mut bad_signature = envelope.clone();
    bad_signature.signature.replace_range(0..2, "00");
    assert!(chain.verify_signed_batch(bad_signature).is_err());

    let mut bad_time = envelope.clone();
    bad_time.signed_at_ms += 1;
    assert!(chain.verify_signed_batch(bad_time).is_err());

    let mut wrong_enterprise = chain.clone();
    wrong_enterprise.manifests[0].enterprise_id = EnterpriseId::new("other").unwrap();
    assert!(wrong_enterprise
        .verify(&EnterpriseId::new("other").unwrap())
        .is_err());
}

#[test]
fn worker_scope_and_coordinator_only_facts_are_enforced() {
    let (chain, grant, collector, mut batch) =
        fixture(BTreeSet::from([EnterpriseSignerRole::Collector]));
    batch.events[0].provenance.machine_id = "machine-b".into();
    assert!(EnterpriseSignedBatch::sign(
        batch.clone(),
        &chain.manifests[0],
        grant.clone(),
        1_000,
        &collector
    )
    .and_then(|envelope| chain.verify_signed_batch(envelope))
    .is_err());

    let (chain, grant, collector, mut batch) =
        fixture(BTreeSet::from([EnterpriseSignerRole::Collector]));
    batch.events[0].fact = EnterpriseFact::ObservationRetracted {
        target_event_id: batch.events[0].event_id.clone(),
        reason: "coordinator decision".into(),
        evidence_digests: BTreeSet::from(["b".repeat(64)]),
    };
    batch = EnterpriseBatch::new(
        batch.enterprise_id.clone(),
        [EnterpriseEvent::new(
            batch.enterprise_id.clone(),
            batch.events[0].provenance.clone(),
            batch.events[0].fact.clone(),
        )
        .unwrap()],
    )
    .unwrap();
    let envelope =
        EnterpriseSignedBatch::sign(batch, &chain.manifests[0], grant, 1_000, &collector).unwrap();
    assert!(chain.verify_signed_batch(envelope).is_err());
}

#[test]
fn manifest_chain_rejects_skips_rollbacks_and_forks() {
    let (chain, _, _, _) = fixture(BTreeSet::from([EnterpriseSignerRole::Collector]));
    let coordinator = EnterpriseSigningKey::from_seed([7; 32]);
    let parent = &chain.manifests[0];
    let successor = EnterpriseTrustManifest::successor(
        parent,
        EnterpriseTrustPolicy {
            issued_at_ms: 200,
            expires_at_ms: 20_000,
            coordinator_threshold: 1,
            coordinators: parent.coordinators.clone(),
            revoked_signer_ids: BTreeMap::new(),
            revoked_grant_ids: BTreeMap::new(),
        },
        &[&coordinator],
    )
    .unwrap();
    let valid = EnterpriseTrustChain {
        anchor_manifest_id: parent.manifest_id.clone(),
        manifests: vec![parent.clone(), successor.clone()],
    };
    valid.verify(&parent.enterprise_id).unwrap();

    let alternate = EnterpriseTrustManifest::successor(
        parent,
        EnterpriseTrustPolicy {
            issued_at_ms: 201,
            expires_at_ms: 20_001,
            coordinator_threshold: 1,
            coordinators: parent.coordinators.clone(),
            revoked_signer_ids: BTreeMap::new(),
            revoked_grant_ids: BTreeMap::new(),
        },
        &[&coordinator],
    )
    .unwrap();
    assert!(EnterpriseTrustChain::detect_forks([successor, alternate]).is_err());

    let mut rollback = valid;
    rollback.manifests[1].revoked_signer_ids = BTreeMap::from([(coordinator.signer_id(), 300)]);
    rollback.manifests[1].manifest_id = rollback.manifests[1].content_id().unwrap();
    rollback.manifests[1].approvals = BTreeMap::new();
    assert!(rollback.verify(&parent.enterprise_id).is_err());
}

#[test]
fn later_revocation_preserves_history_but_rejects_post_revocation_signatures() {
    let (root_chain, grant, collector, batch) =
        fixture(BTreeSet::from([EnterpriseSignerRole::Collector]));
    let coordinator = EnterpriseSigningKey::from_seed([7; 32]);
    let root = &root_chain.manifests[0];
    let successor = EnterpriseTrustManifest::successor(
        root,
        EnterpriseTrustPolicy {
            issued_at_ms: 200,
            expires_at_ms: 20_000,
            coordinator_threshold: 1,
            coordinators: root.coordinators.clone(),
            revoked_signer_ids: BTreeMap::new(),
            revoked_grant_ids: BTreeMap::from([(grant.grant_id.clone(), 2_000)]),
        },
        &[&coordinator],
    )
    .unwrap();
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: root.manifest_id.clone(),
        manifests: vec![root.clone(), successor],
    };

    let historical =
        EnterpriseSignedBatch::sign(batch.clone(), root, grant.clone(), 1_000, &collector).unwrap();
    chain.verify_signed_batch(historical).unwrap();

    let post_revocation =
        EnterpriseSignedBatch::sign(batch, root, grant, 3_000, &collector).unwrap();
    assert!(chain.verify_signed_batch(post_revocation).is_err());
}
