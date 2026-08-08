use agent_orchestration::{EnterpriseBatchId, EnterpriseId};
use scout_ingest_protocol::{CoordinatorSigningKey, IngestReceipt, ScoutTenantId};

fn receipt(signer: &CoordinatorSigningKey) -> IngestReceipt {
    IngestReceipt::issue(
        ScoutTenantId::new("organization:acme").unwrap(),
        EnterpriseId::new("acme").unwrap(),
        format!("trust-manifest:{}", "1".repeat(64)),
        EnterpriseBatchId::new(format!("batch:{}", "2".repeat(64))).unwrap(),
        "3".repeat(64),
        "5".repeat(64),
        7,
        7,
        1_000,
        Some(format!("central-ingestion:{}", "4".repeat(64))),
        signer,
    )
    .unwrap()
}

#[test]
fn receipt_is_deterministic_and_verifies_only_under_the_pinned_key() {
    let signer = CoordinatorSigningKey::from_seed([7; 32]);
    let first = receipt(&signer);
    let second = receipt(&signer);
    assert_eq!(first, second);
    first.verify(&signer.public_key_hex()).unwrap();

    let other = CoordinatorSigningKey::from_seed([8; 32]);
    assert!(first.verify(&other.public_key_hex()).is_err());
}

#[test]
fn receipt_tampering_fails_closed() {
    let signer = CoordinatorSigningKey::from_seed([7; 32]);
    let original = receipt(&signer);
    for mut tampered in [
        {
            let mut value = original.clone();
            value.sequence += 1;
            value
        },
        {
            let mut value = original.clone();
            value.issued_at_ms += 1;
            value
        },
        {
            let mut value = original.clone();
            value.envelope_sha256 = "5".repeat(64);
            value
        },
        {
            let mut value = original.clone();
            value.batch_accumulator_root = "6".repeat(64);
            value
        },
        {
            let mut value = original.clone();
            value.previous_receipt_id = None;
            value
        },
    ] {
        assert!(tampered.verify(&signer.public_key_hex()).is_err());
        tampered.receipt_id = original.receipt_id.clone();
        assert!(tampered.verify(&signer.public_key_hex()).is_err());
    }
}
