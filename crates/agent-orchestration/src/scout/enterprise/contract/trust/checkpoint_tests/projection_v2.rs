use super::*;
use crate::scout::enterprise::contract::trust::checkpoint::{
    EnterpriseLedgerCommitment, EnterpriseSnapshotCommitmentV2,
};
use crate::scout::enterprise::contract::trust::crypto::AuthTranscript;

fn snapshot_v2(enterprise_id: &EnterpriseId) -> EnterpriseSnapshotCommitmentV2 {
    EnterpriseSnapshotCommitmentV2::new(
        enterprise_id,
        "7".repeat(64),
        format!("scout-event-set-v1:12:2:{}", "6".repeat(64)),
        format!("scout-projection-map-v2:12:3:{}", "8".repeat(64)),
    )
    .unwrap()
}

#[test]
fn signed_legacy_v1_snapshot_checkpoint_keeps_its_exact_content_and_signature() {
    #[derive(serde::Serialize)]
    struct LegacyV1CheckpointContent<'a> {
        schema_version: u16,
        enterprise_id: &'a EnterpriseId,
        manifest_id: &'a str,
        sequence: u64,
        previous_checkpoint_id: &'a Option<String>,
        issued_at_ms: u64,
        batch_root: &'a str,
        event_root: &'a str,
        batch_count: u64,
        event_count: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        ledger_commitment: Option<&'a EnterpriseLedgerCommitment>,
        snapshot_commitment: Option<&'a EnterpriseSnapshotCommitment>,
    }

    let fixture = fixture();
    let checkpoint =
        issue_checkpoint_with_commitment(&fixture, snapshot_commitment(&fixture.enterprise));
    let legacy_id = format!(
        "ledger-checkpoint:{}",
        crate::scout::enterprise::contract::canonical_digest(&LegacyV1CheckpointContent {
            schema_version: checkpoint.schema_version,
            enterprise_id: &checkpoint.enterprise_id,
            manifest_id: &checkpoint.manifest_id,
            sequence: checkpoint.sequence,
            previous_checkpoint_id: &checkpoint.previous_checkpoint_id,
            issued_at_ms: checkpoint.issued_at_ms,
            batch_root: &checkpoint.batch_root,
            event_root: &checkpoint.event_root,
            batch_count: checkpoint.batch_count,
            event_count: checkpoint.event_count,
            ledger_commitment: checkpoint.ledger_commitment.as_ref(),
            snapshot_commitment: checkpoint.snapshot_commitment.as_ref(),
        })
        .unwrap()
    );
    assert_eq!(checkpoint.checkpoint_id, legacy_id);
    let signer_id = fixture.coordinator.signer_id();
    assert_eq!(
        checkpoint.approvals[&signer_id],
        fixture.coordinator.sign(&AuthTranscript {
            kind: "ledger_checkpoint",
            enterprise_id: fixture.enterprise.as_str(),
            payload_id: &legacy_id,
            manifest_id: &fixture.manifest.manifest_id,
            grant_id: "",
            signer_id: &signer_id,
        })
    );
    let encoded = serde_json::to_value(&checkpoint).unwrap();
    assert!(encoded.get("snapshot_commitment").is_some());
    assert!(encoded.get("snapshot_commitment_v2").is_none());
    let decoded: EnterpriseLedgerCheckpoint = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, checkpoint);
    fixture.chain.verify_ledger_checkpoint(decoded).unwrap();
}

#[test]
fn snapshot_root_versions_are_mutually_rejecting() {
    let enterprise = EnterpriseId::new("acme").unwrap();
    let event_root = format!("scout-event-set-v1:8:2:{}", "2".repeat(64));
    let projection_v1 = format!("scout-projection-map-v1:8:3:{}", "3".repeat(64));
    let projection_v2 = format!("scout-projection-map-v2:8:3:{}", "4".repeat(64));

    assert!(EnterpriseSnapshotCommitment::new(
        &enterprise,
        "1".repeat(64),
        event_root.clone(),
        projection_v2.clone(),
    )
    .is_err());
    assert!(EnterpriseSnapshotCommitmentV2::new(
        &enterprise,
        "1".repeat(64),
        event_root.clone(),
        projection_v1,
    )
    .is_err());
    EnterpriseSnapshotCommitmentV2::new(&enterprise, "1".repeat(64), event_root, projection_v2)
        .unwrap();
}

#[test]
fn new_projection_encoding_validates_and_signs_only_as_v2() {
    let fixture = fixture();
    let ledger =
        EnterpriseLedgerCommitment::from_batches(&fixture.enterprise, 9, &fixture.batches).unwrap();
    let snapshot = snapshot_v2(&fixture.enterprise);
    assert!(snapshot
        .enterprise_snapshot_root_v2
        .starts_with("scout-enterprise-snapshot-v2:"));
    let checkpoint = EnterpriseLedgerCheckpoint::issue_v2(
        &fixture.manifest,
        1,
        None,
        1_000,
        ledger,
        Some(snapshot.clone()),
        &[&fixture.coordinator],
    )
    .unwrap();
    assert!(checkpoint.snapshot_commitment.is_none());
    assert_eq!(checkpoint.snapshot_commitment_v2, Some(snapshot.clone()));
    let verified = fixture
        .chain
        .verify_ledger_checkpoint(checkpoint.clone())
        .unwrap();
    assert_eq!(verified.snapshot_commitment_v2(), Some(&snapshot));

    let encoded = serde_json::to_value(&checkpoint).unwrap();
    assert!(encoded.get("snapshot_commitment").is_none());
    assert!(encoded.get("snapshot_commitment_v2").is_some());
}

#[test]
fn v2_snapshot_tamper_and_enterprise_transplant_fail() {
    let fixture = fixture();
    let ledger =
        EnterpriseLedgerCommitment::from_batches(&fixture.enterprise, 9, &fixture.batches).unwrap();
    let snapshot = snapshot_v2(&fixture.enterprise);
    let checkpoint = EnterpriseLedgerCheckpoint::issue_v2(
        &fixture.manifest,
        1,
        None,
        1_000,
        ledger,
        Some(snapshot.clone()),
        &[&fixture.coordinator],
    )
    .unwrap();

    let mut tampered = checkpoint.clone();
    tampered
        .snapshot_commitment_v2
        .as_mut()
        .unwrap()
        .graph_digest
        .replace_range(0..2, "aa");
    assert!(fixture.chain.verify_ledger_checkpoint(tampered).is_err());

    let other = EnterpriseId::new("other").unwrap();
    assert!(snapshot.validate(&other).is_err());
    let mut transplanted = checkpoint;
    transplanted.enterprise_id = other;
    transplanted.checkpoint_id = transplanted.content_id().unwrap();
    assert!(fixture
        .chain
        .verify_ledger_checkpoint(transplanted)
        .is_err());
}
