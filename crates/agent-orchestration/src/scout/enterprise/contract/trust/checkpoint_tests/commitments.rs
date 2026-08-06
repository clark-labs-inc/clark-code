use super::*;
use crate::scout::enterprise::contract::trust::checkpoint::EnterpriseLedgerCommitment;
use crate::scout::enterprise::contract::trust::crypto::AuthTranscript;

fn ledger_commitment(enterprise_id: &EnterpriseId) -> EnterpriseLedgerCommitment {
    EnterpriseLedgerCommitment::new(
        enterprise_id,
        7,
        format!("scout-batch-set-v1:12:2:{}", "4".repeat(64)),
        format!("scout-event-set-v1:12:2:{}", "5".repeat(64)),
        2,
        2,
    )
    .unwrap()
}

fn issue_checkpoint_with_ledger(
    fixture: &Fixture,
    commitment: EnterpriseLedgerCommitment,
) -> EnterpriseLedgerCheckpoint {
    EnterpriseLedgerCheckpoint::issue_v2(
        &fixture.manifest,
        1,
        None,
        1_000,
        commitment,
        None,
        &[&fixture.coordinator],
    )
    .unwrap()
}

#[test]
fn v2_issuance_is_summary_free_and_batches_reconstruct_typed_roots() {
    let fixture = fixture();
    let commitment =
        EnterpriseLedgerCommitment::from_batches(&fixture.enterprise, 11, &fixture.batches)
            .unwrap();
    assert!(commitment
        .batch_set_root_v1
        .starts_with("scout-batch-set-v1:12:2:"));
    assert!(commitment
        .event_set_root_v1
        .starts_with("scout-event-set-v1:12:2:"));
    let checkpoint = EnterpriseLedgerCheckpoint::issue_v2(
        &fixture.manifest,
        1,
        None,
        1_000,
        commitment.clone(),
        None,
        &[&fixture.coordinator],
    )
    .unwrap();
    assert_eq!(
        checkpoint.batch_root,
        commitment
            .compatibility_batch_root(&fixture.enterprise)
            .unwrap()
    );
    assert_eq!(
        checkpoint.event_root,
        commitment
            .compatibility_event_root(&fixture.enterprise)
            .unwrap()
    );
    let verified = fixture.chain.verify_ledger_checkpoint(checkpoint).unwrap();
    verified.verify_batches(&fixture.batches).unwrap();
    assert!(verified.verify_batches(&fixture.batches[..1]).is_err());

    let legacy_summary =
        EnterpriseLedgerSummary::from_batches(fixture.enterprise.clone(), &fixture.batches)
            .unwrap();
    assert!(EnterpriseLedgerCheckpoint::issue_with_commitments(
        &fixture.manifest,
        1,
        None,
        1_000,
        &legacy_summary,
        Some(commitment),
        None,
        &[&fixture.coordinator],
    )
    .is_err());
}

#[test]
fn ledger_commitment_changes_signed_identity_and_has_verified_helpers() {
    let fixture = fixture();
    let legacy = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches);
    let commitment = ledger_commitment(&fixture.enterprise);
    assert_eq!(
        commitment.enterprise_ledger_root_v2,
        format!(
            "scout-enterprise-ledger-v2:{}",
            crate::scout::enterprise::contract::canonical_digest(&(
                "scout-enterprise-ledger-v2",
                commitment.schema_version,
                fixture.enterprise.as_str(),
                commitment.generation,
                &commitment.batch_set_root_v1,
                &commitment.event_set_root_v1,
                commitment.batch_count,
                commitment.event_count,
            ))
            .unwrap()
        )
    );
    let committed = issue_checkpoint_with_ledger(&fixture, commitment.clone());
    assert_ne!(committed.checkpoint_id, legacy.checkpoint_id);
    assert_ne!(committed.approvals, legacy.approvals);

    let verified = fixture.chain.verify_ledger_checkpoint(committed).unwrap();
    assert_eq!(verified.ledger_commitment(), Some(&commitment));
    assert_eq!(verified.ledger_generation(), Some(7));
    assert_eq!(
        verified.enterprise_ledger_root_v2(),
        Some(commitment.enterprise_ledger_root_v2.as_str())
    );
    verified.verify_ledger_commitment(&commitment).unwrap();

    let different = EnterpriseLedgerCommitment::new(
        &fixture.enterprise,
        8,
        commitment.batch_set_root_v1,
        commitment.event_set_root_v1,
        2,
        2,
    )
    .unwrap();
    assert!(verified.verify_ledger_commitment(&different).is_err());
}

#[test]
fn ledger_commitment_rejects_noncanonical_roots_and_count_mismatches() {
    let enterprise = EnterpriseId::new("acme").unwrap();
    let valid_batch = format!("scout-batch-set-v1:12:2:{}", "4".repeat(64));
    let valid_event = format!("scout-event-set-v1:12:3:{}", "5".repeat(64));

    assert!(EnterpriseLedgerCommitment::new(
        &enterprise,
        0,
        valid_batch.clone(),
        valid_event.clone(),
        2,
        3,
    )
    .is_err());
    assert!(EnterpriseLedgerCommitment::new(
        &enterprise,
        1,
        valid_batch.clone(),
        valid_event.clone(),
        1,
        3,
    )
    .is_err());
    assert!(EnterpriseLedgerCommitment::new(
        &enterprise,
        1,
        valid_batch,
        format!("scout-event-set-v1:11:3:{}", "5".repeat(64)),
        2,
        3,
    )
    .is_err());
    assert!(EnterpriseLedgerCommitment::new(
        &enterprise,
        1,
        format!("scout-batch-set-v1:11:2:{}", "4".repeat(64)),
        format!("scout-event-set-v1:11:3:{}", "5".repeat(64)),
        2,
        3,
    )
    .is_err());
    assert!(EnterpriseLedgerCommitment::new(
        &enterprise,
        1,
        format!("scout-batch-set-v1:012:2:{}", "4".repeat(64)),
        valid_event,
        2,
        3,
    )
    .is_err());
}

#[test]
fn absent_commitments_preserve_legacy_content_serde_and_signature() {
    #[derive(serde::Serialize)]
    struct LegacyCheckpointContent<'a> {
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
    }

    let fixture = fixture();
    let checkpoint = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches);
    let legacy_id = format!(
        "ledger-checkpoint:{}",
        crate::scout::enterprise::contract::canonical_digest(&LegacyCheckpointContent {
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
    assert!(encoded.get("ledger_commitment").is_none());
    assert!(encoded.get("snapshot_commitment").is_none());
    let decoded: EnterpriseLedgerCheckpoint = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, checkpoint);
    fixture.chain.verify_ledger_checkpoint(decoded).unwrap();
}

#[test]
fn ledger_commitment_tampering_fails_root_id_and_signature_verification() {
    let fixture = fixture();
    let checkpoint = issue_checkpoint_with_ledger(&fixture, ledger_commitment(&fixture.enterprise));

    let mut component_tamper = checkpoint.clone();
    component_tamper
        .ledger_commitment
        .as_mut()
        .unwrap()
        .generation = 8;
    assert!(fixture
        .chain
        .verify_ledger_checkpoint(component_tamper)
        .is_err());

    let mut root_tamper = checkpoint.clone();
    root_tamper
        .ledger_commitment
        .as_mut()
        .unwrap()
        .enterprise_ledger_root_v2
        .replace_range(0..2, "aa");
    assert!(fixture.chain.verify_ledger_checkpoint(root_tamper).is_err());

    let original = checkpoint.ledger_commitment.as_ref().unwrap();
    let mut unsigned_replacement = checkpoint.clone();
    unsigned_replacement.ledger_commitment = Some(
        EnterpriseLedgerCommitment::new(
            &fixture.enterprise,
            8,
            original.batch_set_root_v1.clone(),
            original.event_set_root_v1.clone(),
            2,
            2,
        )
        .unwrap(),
    );
    unsigned_replacement.checkpoint_id = unsigned_replacement.content_id().unwrap();
    assert!(fixture
        .chain
        .verify_ledger_checkpoint(unsigned_replacement)
        .is_err());
}

#[test]
fn ledger_commitment_cannot_be_transplanted_between_enterprises() {
    let fixture = fixture();
    let commitment = ledger_commitment(&fixture.enterprise);
    let other_enterprise = EnterpriseId::new("other").unwrap();
    assert!(commitment.validate(&other_enterprise).is_err());

    let other_manifest = EnterpriseTrustManifest::initial(
        other_enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000020".into(),
        100,
        20_000,
        &fixture.coordinator,
    )
    .unwrap();
    let other_batches = [
        batch(&other_enterprise, "service:one", 1),
        batch(&other_enterprise, "service:two", 2),
    ];
    let other_summary =
        EnterpriseLedgerSummary::from_batches(other_enterprise.clone(), &other_batches).unwrap();
    assert!(EnterpriseLedgerCheckpoint::issue_with_commitments(
        &other_manifest,
        1,
        None,
        1_000,
        &other_summary,
        Some(commitment.clone()),
        None,
        &[&fixture.coordinator],
    )
    .is_err());

    let rebound = EnterpriseLedgerCommitment::new(
        &other_enterprise,
        commitment.generation,
        commitment.batch_set_root_v1,
        commitment.event_set_root_v1,
        commitment.batch_count,
        commitment.event_count,
    )
    .unwrap();
    assert_ne!(
        rebound.enterprise_ledger_root_v2,
        commitment.enterprise_ledger_root_v2
    );
}

#[test]
fn snapshot_commitment_changes_checkpoint_identity_and_signature() {
    let fixture = fixture();
    let legacy = issue_checkpoint(&fixture, 1, None, 1_000, &fixture.batches);
    let commitment = snapshot_commitment(&fixture.enterprise);
    let committed = issue_checkpoint_with_commitment(&fixture, commitment.clone());

    assert_ne!(committed.checkpoint_id, legacy.checkpoint_id);
    assert_ne!(committed.approvals, legacy.approvals);
    assert_eq!(committed.snapshot_commitment, Some(commitment));
    fixture.chain.verify_ledger_checkpoint(committed).unwrap();
}

#[test]
fn snapshot_commitment_rejects_noncanonical_or_mismatched_root_ids() {
    let enterprise = EnterpriseId::new("acme").unwrap();
    let graph_digest = "1".repeat(64);
    let valid_event = format!("scout-event-set-v1:12:2:{}", "2".repeat(64));
    let valid_projection = format!("scout-projection-map-v1:12:3:{}", "3".repeat(64));

    for invalid_event in [
        format!("wrong-event-set-v1:12:2:{}", "2".repeat(64)),
        format!("scout-event-set-v1:012:2:{}", "2".repeat(64)),
        format!("scout-event-set-v1:17:2:{}", "2".repeat(64)),
        format!("scout-event-set-v1:12:02:{}", "2".repeat(64)),
        format!("scout-event-set-v1:12:2:{}", "A".repeat(64)),
    ] {
        assert!(EnterpriseSnapshotCommitment::new(
            &enterprise,
            graph_digest.clone(),
            invalid_event,
            valid_projection.clone(),
        )
        .is_err());
    }
    let mismatched_projection = format!("scout-projection-map-v1:11:3:{}", "3".repeat(64));
    assert!(EnterpriseSnapshotCommitment::new(
        &enterprise,
        graph_digest,
        valid_event,
        mismatched_projection,
    )
    .is_err());
}

#[test]
fn snapshot_commitment_tampering_fails_shape_id_and_signature_verification() {
    let fixture = fixture();
    let checkpoint =
        issue_checkpoint_with_commitment(&fixture, snapshot_commitment(&fixture.enterprise));
    let mut component_tamper = checkpoint.clone();
    component_tamper
        .snapshot_commitment
        .as_mut()
        .unwrap()
        .graph_digest
        .replace_range(0..2, "aa");
    assert!(fixture
        .chain
        .verify_ledger_checkpoint(component_tamper)
        .is_err());

    let mut root_tamper = checkpoint.clone();
    root_tamper
        .snapshot_commitment
        .as_mut()
        .unwrap()
        .enterprise_snapshot_root_v1
        .replace_range(0..2, "bb");
    assert!(fixture.chain.verify_ledger_checkpoint(root_tamper).is_err());

    let mut unsigned_replacement = checkpoint;
    unsigned_replacement.snapshot_commitment = Some(
        EnterpriseSnapshotCommitment::new(
            &fixture.enterprise,
            "4".repeat(64),
            format!("scout-event-set-v1:12:4:{}", "5".repeat(64)),
            format!("scout-projection-map-v1:12:5:{}", "6".repeat(64)),
        )
        .unwrap(),
    );
    unsigned_replacement.checkpoint_id = unsigned_replacement.content_id().unwrap();
    assert!(fixture
        .chain
        .verify_ledger_checkpoint(unsigned_replacement)
        .is_err());
}

#[test]
fn snapshot_commitment_transplant_cannot_survive_outer_enterprise_signature() {
    let fixture = fixture();
    let checkpoint =
        issue_checkpoint_with_commitment(&fixture, snapshot_commitment(&fixture.enterprise));
    fixture
        .chain
        .verify_ledger_checkpoint(checkpoint.clone())
        .unwrap();

    let other_enterprise = EnterpriseId::new("other").unwrap();
    let other_manifest = EnterpriseTrustManifest::initial(
        other_enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000020".into(),
        100,
        20_000,
        &fixture.coordinator,
    )
    .unwrap();
    let other_chain = EnterpriseTrustChain {
        anchor_manifest_id: other_manifest.manifest_id.clone(),
        manifests: vec![other_manifest.clone()],
    };
    let mut transplanted = checkpoint.clone();
    transplanted.enterprise_id = other_enterprise.clone();
    transplanted.manifest_id = other_manifest.manifest_id.clone();
    transplanted.checkpoint_id = transplanted.content_id().unwrap();
    assert!(other_chain
        .verify_ledger_checkpoint(transplanted.clone())
        .is_err());

    let original = checkpoint.snapshot_commitment.as_ref().unwrap();
    transplanted.snapshot_commitment = Some(
        EnterpriseSnapshotCommitment::new(
            &other_enterprise,
            original.graph_digest.clone(),
            original.event_set_root_v1.clone(),
            original.projection_map_root_v1.clone(),
        )
        .unwrap(),
    );
    transplanted.checkpoint_id = transplanted.content_id().unwrap();
    assert!(other_chain.verify_ledger_checkpoint(transplanted).is_err());
}
