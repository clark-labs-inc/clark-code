use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::scout::enterprise::contract::{
    AuthorityRef, EnterpriseBatch, EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact,
    EnterpriseId, EnterpriseProvenance, GraphEntityObservation,
};

mod commitments;
mod projection_v2;

struct Fixture {
    enterprise: EnterpriseId,
    coordinator: EnterpriseSigningKey,
    manifest: EnterpriseTrustManifest,
    chain: EnterpriseTrustChain,
    batches: Vec<EnterpriseBatch>,
}

fn fixture() -> Fixture {
    let enterprise = EnterpriseId::new("acme").unwrap();
    let coordinator = EnterpriseSigningKey::from_seed([17; 32]);
    let manifest = EnterpriseTrustManifest::initial(
        enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000017".into(),
        100,
        20_000,
        &coordinator,
    )
    .unwrap();
    let batches = vec![
        batch(&enterprise, "service:checkout", 1),
        batch(&enterprise, "service:payments", 2),
    ];
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: manifest.manifest_id.clone(),
        manifests: vec![manifest.clone()],
    };
    Fixture {
        enterprise,
        coordinator,
        manifest,
        chain,
        batches,
    }
}

fn batch(enterprise: &EnterpriseId, native_id: &str, source_sequence: u64) -> EnterpriseBatch {
    let observation = GraphEntityObservation::new(
        enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:prod", native_id).unwrap(),
        BTreeSet::from([native_id.to_owned()]),
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
            source_sequence,
            observed_at_ms: 500 + source_sequence,
            source_fingerprint: "f".repeat(64),
        },
        EnterpriseFact::EntityObserved(observation),
    )
    .unwrap();
    EnterpriseBatch::new(enterprise.clone(), [event]).unwrap()
}

fn issue_checkpoint(
    fixture: &Fixture,
    sequence: u64,
    previous: Option<String>,
    issued_at_ms: u64,
    batches: &[EnterpriseBatch],
) -> EnterpriseLedgerCheckpoint {
    let summary =
        EnterpriseLedgerSummary::from_batches(fixture.enterprise.clone(), batches).unwrap();
    EnterpriseLedgerCheckpoint::issue(
        &fixture.manifest,
        sequence,
        previous,
        issued_at_ms,
        &summary,
        None,
        &[&fixture.coordinator],
    )
    .unwrap()
}

fn flip_signature(signature: &mut String) {
    let replacement = if signature.starts_with("00") {
        "01"
    } else {
        "00"
    };
    signature.replace_range(0..2, replacement);
}

fn snapshot_commitment(enterprise_id: &EnterpriseId) -> EnterpriseSnapshotCommitment {
    EnterpriseSnapshotCommitment::new(
        enterprise_id,
        "1".repeat(64),
        format!("scout-event-set-v1:12:2:{}", "2".repeat(64)),
        format!("scout-projection-map-v1:12:3:{}", "3".repeat(64)),
    )
    .unwrap()
}

fn issue_checkpoint_with_commitment(
    fixture: &Fixture,
    commitment: EnterpriseSnapshotCommitment,
) -> EnterpriseLedgerCheckpoint {
    let summary =
        EnterpriseLedgerSummary::from_batches(fixture.enterprise.clone(), &fixture.batches)
            .unwrap();
    EnterpriseLedgerCheckpoint::issue(
        &fixture.manifest,
        1,
        None,
        1_000,
        &summary,
        Some(commitment),
        &[&fixture.coordinator],
    )
    .unwrap()
}

#[test]
fn valid_checkpoint_and_inclusion_verify() {
    let fixture = fixture();
    let checkpoint = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches);
    let verified = fixture
        .chain
        .verify_ledger_checkpoint(checkpoint.clone())
        .unwrap();
    verified.verify_batches(&fixture.batches).unwrap();

    let mut cursor = EnterpriseCheckpointCursor::new(fixture.enterprise.clone());
    assert_eq!(
        cursor.observe(&verified).unwrap(),
        EnterpriseCheckpointObservation::Advanced
    );
    assert_eq!(
        cursor.observe(&verified).unwrap(),
        EnterpriseCheckpointObservation::Duplicate
    );

    let receipt = EnterpriseBatchInclusionReceipt::issue(
        &fixture.manifest,
        &checkpoint,
        &fixture.batches[0],
        1_001,
        &[&fixture.coordinator],
    )
    .unwrap();
    let inclusion = fixture
        .chain
        .verify_batch_inclusion(&verified, receipt, &fixture.batches[0])
        .unwrap();
    assert_eq!(inclusion.receipt().batch_id, fixture.batches[0].batch_id);
}

#[test]
fn authenticated_checkpoint_detects_deleted_batch() {
    let fixture = fixture();
    let checkpoint = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches);
    let verified = fixture.chain.verify_ledger_checkpoint(checkpoint).unwrap();

    assert!(verified.verify_batches(&fixture.batches[..1]).is_err());
}

#[test]
fn cursor_rejects_rollback_and_same_sequence_replay() {
    let fixture = fixture();
    let first = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches[..1]);
    let second = issue_checkpoint(
        &fixture,
        2,
        Some(first.checkpoint_id.clone()),
        1_100,
        &fixture.batches,
    );
    let alternate_second = issue_checkpoint(
        &fixture,
        2,
        Some(first.checkpoint_id.clone()),
        1_101,
        &fixture.batches,
    );
    let first = fixture.chain.verify_ledger_checkpoint(first).unwrap();
    let second = fixture.chain.verify_ledger_checkpoint(second).unwrap();
    let alternate_second = fixture
        .chain
        .verify_ledger_checkpoint(alternate_second)
        .unwrap();
    let mut cursor = EnterpriseCheckpointCursor::new(fixture.enterprise);

    cursor.observe(&first).unwrap();
    cursor.observe(&second).unwrap();
    assert!(cursor.observe(&first).is_err());
    assert!(cursor.observe(&alternate_second).is_err());
}

#[test]
fn inclusion_replay_against_another_checkpoint_fails() {
    let fixture = fixture();
    let first = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches[..1]);
    let second = issue_checkpoint(
        &fixture,
        2,
        Some(first.checkpoint_id.clone()),
        1_100,
        &fixture.batches,
    );
    let receipt = EnterpriseBatchInclusionReceipt::issue(
        &fixture.manifest,
        &first,
        &fixture.batches[0],
        1_001,
        &[&fixture.coordinator],
    )
    .unwrap();
    let second = fixture.chain.verify_ledger_checkpoint(second).unwrap();

    assert!(fixture
        .chain
        .verify_batch_inclusion(&second, receipt, &fixture.batches[0])
        .is_err());
}

#[test]
fn checkpoint_and_receipt_tampering_fail_strict_verification() {
    let fixture = fixture();
    let checkpoint = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches);
    let mut bad_checkpoint_signature = checkpoint.clone();
    flip_signature(
        bad_checkpoint_signature
            .approvals
            .values_mut()
            .next()
            .unwrap(),
    );
    assert!(fixture
        .chain
        .verify_ledger_checkpoint(bad_checkpoint_signature)
        .is_err());

    let mut bad_content = checkpoint.clone();
    bad_content.event_root.replace_range(0..2, "00");
    assert!(fixture.chain.verify_ledger_checkpoint(bad_content).is_err());

    let verified = fixture
        .chain
        .verify_ledger_checkpoint(checkpoint.clone())
        .unwrap();
    let mut receipt = EnterpriseBatchInclusionReceipt::issue(
        &fixture.manifest,
        &checkpoint,
        &fixture.batches[0],
        1_001,
        &[&fixture.coordinator],
    )
    .unwrap();
    flip_signature(receipt.approvals.values_mut().next().unwrap());
    assert!(fixture
        .chain
        .verify_batch_inclusion(&verified, receipt, &fixture.batches[0])
        .is_err());
}

#[test]
fn wrong_enterprise_and_unpinned_manifest_fail_closed() {
    let fixture = fixture();
    let other_enterprise = EnterpriseId::new("other").unwrap();
    let other_manifest = EnterpriseTrustManifest::initial(
        other_enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000018".into(),
        100,
        20_000,
        &fixture.coordinator,
    )
    .unwrap();
    let other_batch = batch(&other_enterprise, "service:other", 1);
    let other_summary =
        EnterpriseLedgerSummary::from_batches(other_enterprise, [&other_batch]).unwrap();
    let other_checkpoint = EnterpriseLedgerCheckpoint::issue(
        &other_manifest,
        1,
        None,
        1_000,
        &other_summary,
        None,
        &[&fixture.coordinator],
    )
    .unwrap();
    assert!(fixture
        .chain
        .verify_ledger_checkpoint(other_checkpoint)
        .is_err());

    let unpinned = EnterpriseTrustManifest::initial(
        fixture.enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000019".into(),
        100,
        20_000,
        &fixture.coordinator,
    )
    .unwrap();
    let summary =
        EnterpriseLedgerSummary::from_batches(fixture.enterprise, &fixture.batches).unwrap();
    let unpinned_checkpoint = EnterpriseLedgerCheckpoint::issue(
        &unpinned,
        1,
        None,
        1_000,
        &summary,
        None,
        &[&fixture.coordinator],
    )
    .unwrap();
    assert!(fixture
        .chain
        .verify_ledger_checkpoint(unpinned_checkpoint)
        .is_err());
}

#[test]
fn cursor_rejects_manifest_and_authoritative_time_rollback() {
    let fixture = fixture();
    let successor = EnterpriseTrustManifest::successor(
        &fixture.manifest,
        EnterpriseTrustPolicy {
            issued_at_ms: 200,
            expires_at_ms: 20_000,
            coordinator_threshold: 1,
            coordinators: fixture.manifest.coordinators.clone(),
            revoked_signer_ids: BTreeMap::new(),
            revoked_grant_ids: BTreeMap::new(),
        },
        &[&fixture.coordinator],
    )
    .unwrap();
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: fixture.manifest.manifest_id.clone(),
        manifests: vec![fixture.manifest.clone(), successor.clone()],
    };
    let summary =
        EnterpriseLedgerSummary::from_batches(fixture.enterprise.clone(), &fixture.batches)
            .unwrap();
    let first = EnterpriseLedgerCheckpoint::issue(
        &successor,
        1,
        None,
        1_000,
        &summary,
        None,
        &[&fixture.coordinator],
    )
    .unwrap();
    let manifest_rollback = EnterpriseLedgerCheckpoint::issue(
        &fixture.manifest,
        2,
        Some(first.checkpoint_id.clone()),
        1_100,
        &summary,
        None,
        &[&fixture.coordinator],
    )
    .unwrap();
    let time_rollback = EnterpriseLedgerCheckpoint::issue(
        &successor,
        2,
        Some(first.checkpoint_id.clone()),
        999,
        &summary,
        None,
        &[&fixture.coordinator],
    )
    .unwrap();
    let first = chain.verify_ledger_checkpoint(first).unwrap();
    let manifest_rollback = chain.verify_ledger_checkpoint(manifest_rollback).unwrap();
    let time_rollback = chain.verify_ledger_checkpoint(time_rollback).unwrap();

    let mut manifest_cursor = EnterpriseCheckpointCursor::new(fixture.enterprise.clone());
    manifest_cursor.observe(&first).unwrap();
    assert!(manifest_cursor.observe(&manifest_rollback).is_err());
    let mut time_cursor = EnterpriseCheckpointCursor::new(fixture.enterprise);
    time_cursor.observe(&first).unwrap();
    assert!(time_cursor.observe(&time_rollback).is_err());
}

#[test]
fn cursor_rejects_tampered_persisted_high_water_metadata() {
    let fixture = fixture();
    let checkpoint = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches);
    let verified = fixture.chain.verify_ledger_checkpoint(checkpoint).unwrap();
    let mut cursor = EnterpriseCheckpointCursor::new(fixture.enterprise);
    cursor.observe(&verified).unwrap();
    let mut encoded = serde_json::to_value(&cursor).unwrap();
    encoded["highest_issued_at_ms"] = serde_json::json!(999);
    let mut tampered: EnterpriseCheckpointCursor = serde_json::from_value(encoded).unwrap();

    assert!(tampered.observe(&verified).is_err());
}
