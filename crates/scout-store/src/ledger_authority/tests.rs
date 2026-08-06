use std::collections::BTreeSet;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact,
    EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance, EnterpriseSignedBatch,
    EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey, EnterpriseTrustChain,
    EnterpriseTrustManifest, GraphEntityObservation, VerifiedEnterpriseBatch,
};
use rusqlite::{params, Connection, TransactionBehavior};
use sha2::{Digest as _, Sha256};

use super::append::AppendFailpoint;
use super::*;

struct Fixture {
    root: tempfile::TempDir,
    enterprise_id: EnterpriseId,
    signer: EnterpriseSigningKey,
    manifest: EnterpriseTrustManifest,
    chain: EnterpriseTrustChain,
    trust_chain_digest: String,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = tempfile::tempdir().expect("temporary ledger");
        let enterprise_id = EnterpriseId::new(format!("enterprise:{name}")).expect("enterprise");
        let signer = EnterpriseSigningKey::from_seed([name.len() as u8 + 1; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise_id.clone(),
            format!("trust:{name}"),
            100,
            100_000,
            &signer,
        )
        .expect("manifest");
        let chain = EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest.clone()],
        };
        let trust_chain_digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&chain).expect("chain bytes"))
        );
        Self {
            root,
            enterprise_id,
            signer,
            manifest,
            chain,
            trust_chain_digest,
        }
    }
    fn authority(&self) -> LedgerAuthority {
        LedgerAuthority::open(
            self.root.path(),
            self.enterprise_id.clone(),
            self.trust_chain_digest.clone(),
        )
        .expect("ledger authority")
    }
    fn event(&self, sequence: u64) -> EnterpriseEvent {
        let digest = format!("{:064x}", sequence);
        let provenance = EnterpriseProvenance {
            machine_id: "fixture-machine".into(),
            run_id: "fixture-run".into(),
            adapter_instance_id: "fixture-adapter".into(),
            auth_context_id: "fixture-auth".into(),
            discovery_epoch: "epoch-1".into(),
            discovery_epoch_sequence: 1,
            source_sequence: sequence,
            observed_at_ms: 1_000 + sequence,
            source_fingerprint: digest.clone(),
        };
        let observation = GraphEntityObservation::new(
            &self.enterprise_id,
            EnterpriseEntityKind::Service,
            AuthorityRef::new("fixture", "services", format!("service-{sequence}"))
                .expect("authority"),
            BTreeSet::from([format!("service-{sequence}")]),
            BTreeSet::from([digest]),
        )
        .expect("observation");
        EnterpriseEvent::new(
            self.enterprise_id.clone(),
            provenance,
            EnterpriseFact::EntityObserved(observation),
        )
        .expect("event")
    }

    fn signed(&self, events: Vec<EnterpriseEvent>, signed_at_ms: u64) -> EnterpriseSignedBatch {
        let first = events
            .iter()
            .map(|event| event.provenance.source_sequence)
            .min()
            .expect("events");
        let last = events
            .iter()
            .map(|event| event.provenance.source_sequence)
            .max()
            .expect("events");
        let grant = EnterpriseSignerGrant::issue(
            &self.manifest,
            self.signer.signer_id(),
            self.signer.public_key_hex(),
            BTreeSet::from([EnterpriseSignerRole::Collector]),
            EnterpriseGrantScope {
                machine_id: "fixture-machine".into(),
                run_id: "fixture-run".into(),
                adapter_instance_id: "fixture-adapter".into(),
                auth_context_id: "fixture-auth".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: first,
                last_source_sequence: last,
            },
            100,
            100_000,
            &[&self.signer],
        )
        .expect("grant");
        EnterpriseSignedBatch::sign(
            EnterpriseBatch::new(self.enterprise_id.clone(), events).expect("batch"),
            &self.manifest,
            grant,
            signed_at_ms,
            &self.signer,
        )
        .expect("signed batch")
    }

    fn verified(&self, envelope: EnterpriseSignedBatch) -> VerifiedEnterpriseBatch {
        self.chain
            .verify_signed_batch(envelope)
            .expect("verified batch")
    }

    fn connection(&self) -> Connection {
        Connection::open(self.root.path().join(LEDGER_DATABASE_NAME)).expect("database")
    }
}

#[test]
fn warm_head_is_one_authenticated_row_with_delete_full_sqlite() {
    let fixture = Fixture::new("warm");
    let authority = fixture.authority();
    let read = authority.read_head().expect("warm head");
    assert_eq!(read.head.generation, 0);
    assert_eq!(read.work.head_rows_read, 1);
    assert_eq!(
        read.work,
        LedgerAuthorityWork {
            head_rows_read: 1,
            ..LedgerAuthorityWork::default()
        }
    );
    assert!(!fixture.root.path().join("batches").exists());

    let connection = fixture.connection();
    let journal: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode");
    let synchronous: u8 = connection
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("sync mode");
    assert_eq!(journal, "delete");
    assert_eq!(synchronous, 2);
}

#[test]
fn append_is_atomic_exactly_idempotent_and_exports_generation_order() {
    let fixture = Fixture::new("append");
    let authority = fixture.authority();
    let first_event = fixture.event(1);
    let first = fixture.verified(fixture.signed(vec![first_event.clone()], 1_001));
    let inserted = authority.append_verified(&first).expect("first append");
    assert_eq!(inserted.outcome, LedgerAppendOutcome::Inserted);
    assert_eq!(
        (inserted.head.generation, inserted.head.batch_count),
        (1, 1)
    );
    assert_eq!(inserted.head.event_count, 1);
    assert_eq!(inserted.head.batch_accumulator.partition_bits, 12);
    assert_eq!(inserted.head.event_accumulator.partition_bits, 12);
    assert_eq!(inserted.work.batch_rows_written, 1);
    assert_eq!(inserted.work.event_rows_written, 1);

    let duplicate = authority
        .append_verified(&first)
        .expect("idempotent append");
    assert_eq!(duplicate.outcome, LedgerAppendOutcome::AlreadyPresent);
    assert_eq!(duplicate.head, inserted.head);
    assert_eq!(duplicate.work.head_rows_read, 1);
    assert_eq!(duplicate.work.batch_lookups, 1);
    assert_eq!(duplicate.work.event_lookups, 0);
    assert_eq!(duplicate.work.accumulator_node_lookups, 0);
    assert_eq!(duplicate.work.head_rows_written, 0);

    let second = fixture.verified(fixture.signed(vec![first_event, fixture.event(2)], 1_002));
    let second_receipt = authority.append_verified(&second).expect("second append");
    assert_eq!(
        (
            second_receipt.head.batch_count,
            second_receipt.head.event_count
        ),
        (2, 2)
    );
    assert_eq!(second_receipt.work.event_lookups, 2);
    assert_eq!(second_receipt.work.event_rows_written, 1);

    let range = authority.read_all_envelopes().expect("all envelopes");
    assert_eq!(
        range
            .envelopes
            .iter()
            .map(|item| item.generation)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(range.work.head_rows_read, 1);
    assert_eq!(range.work.envelope_rows_read, 2);
    assert!(range.work.envelope_bytes_read > 0);
    let commitment = second_receipt
        .head
        .ledger_commitment()
        .expect("ledger commitment");
    assert_eq!(commitment.generation, 2);
    assert_eq!(
        commitment.event_set_root_v1,
        accumulator::root_id("scout-event-set-v1", &second_receipt.head.event_accumulator)
    );
}

#[test]
fn same_batch_content_with_different_signed_bytes_is_not_idempotent() {
    let fixture = Fixture::new("exact");
    let authority = fixture.authority();
    let event = fixture.event(1);
    let first = fixture.verified(fixture.signed(vec![event.clone()], 1_001));
    let differently_signed = fixture.verified(fixture.signed(vec![event], 1_002));
    assert_eq!(first.batch().batch_id, differently_signed.batch().batch_id);
    authority.append_verified(&first).expect("first append");
    let error = authority
        .append_verified(&differently_signed)
        .expect_err("different envelope bytes must fail");
    assert!(error.contains("different signed bytes"), "{error}");
}

#[test]
fn authority_rejects_cross_enterprise_open_and_append() {
    let first = Fixture::new("first");
    let authority = first.authority();
    let second = Fixture::new("second");
    let cross = second.verified(second.signed(vec![second.event(1)], 1_001));
    let error = authority
        .append_verified(&cross)
        .expect_err("cross-enterprise append");
    assert!(error.contains("another enterprise"), "{error}");
    let error = LedgerAuthority::open(
        first.root.path(),
        second.enterprise_id.clone(),
        second.trust_chain_digest.clone(),
    )
    .err()
    .expect("cross-enterprise open");
    assert!(
        error.contains("another authority") || error.contains("another enterprise"),
        "{error}"
    );
}

#[test]
fn failed_transactions_leave_no_partial_payload_or_head() {
    for (name, failpoint) in [
        ("payload-crash", AppendFailpoint::AfterPayloadRows),
        ("head-crash", AppendFailpoint::AfterHeadWrite),
    ] {
        let fixture = Fixture::new(name);
        let authority = fixture.authority();
        let batch = fixture.verified(fixture.signed(vec![fixture.event(1)], 1_001));
        super::append::append(&authority, &batch, failpoint).expect_err("injected crash");

        let connection = database::open_connection(fixture.root.path()).expect("reopen database");
        let mut work = LedgerAuthorityWork::default();
        let head = database::read_head(
            &connection,
            &authority.auth_key,
            &fixture.enterprise_id,
            &fixture.trust_chain_digest,
            &mut work,
        )
        .expect("rolled-back head");
        assert_eq!(head.generation, 0);
        assert!(database::read_batch(
            &connection,
            &authority.auth_key,
            &fixture.enterprise_id,
            batch.batch().batch_id.as_str(),
            &mut work,
        )
        .expect("batch lookup")
        .is_none());
    }
}

#[test]
fn committed_unsealed_successor_is_recovered_and_resealed() {
    let fixture = Fixture::new("successor-recovery");
    let authority = fixture.authority();
    let batch = fixture.verified(fixture.signed(vec![fixture.event(1)], 1_001));
    let error = super::append::append(&authority, &batch, AppendFailpoint::AfterCommitBeforeSeal)
        .expect_err("injected post-commit crash");
    assert!(error.contains("before seal"), "{error}");

    let recovered = fixture.authority();
    let head = recovered.read_head().expect("recovered head").head;
    assert_eq!(
        (head.generation, head.batch_count, head.event_count),
        (1, 1, 1)
    );
    assert_eq!(
        recovered
            .read_envelope(&batch.batch().batch_id)
            .expect("recovered envelope")
            .envelope,
        Some(batch.envelope().clone())
    );
}

#[test]
fn recovery_rejects_wrong_predecessor_and_forked_successors() {
    for fork in [false, true] {
        let fixture = Fixture::new(if fork { "forked" } else { "wrong-parent" });
        let authority = fixture.authority();
        let batch = fixture.verified(fixture.signed(vec![fixture.event(1)], 1_001));
        super::append::append(&authority, &batch, AppendFailpoint::AfterCommitBeforeSeal)
            .expect_err("post-commit crash");

        let mut connection =
            database::open_connection(fixture.root.path()).expect("open unsealed database");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("tamper transaction");
        let mut work = LedgerAuthorityWork::default();
        let current = database::read_head(
            &transaction,
            &authority.auth_key,
            &fixture.enterprise_id,
            &fixture.trust_chain_digest,
            &mut work,
        )
        .expect("successor head");
        let alternate = database::make_head(database::LedgerHeadFields {
            enterprise_id: current.enterprise_id.clone(),
            generation: current.generation,
            previous_head_id: Some(format!(
                "ledger-head:{}",
                if fork { "1" } else { "2" }.repeat(64)
            )),
            trust_chain_digest: current.trust_chain_digest.clone(),
            batch_count: current.batch_count,
            event_count: current.event_count,
            batch_accumulator: current.batch_accumulator.clone(),
            event_accumulator: current.event_accumulator.clone(),
        })
        .expect("alternate head");
        if fork {
            database::insert_head_history(&transaction, &authority.auth_key, &alternate)
                .expect("fork history");
        } else {
            transaction
                .execute(
                    "DELETE FROM ledger_head_history WHERE generation = ?1",
                    [current.generation],
                )
                .expect("delete original history");
            database::write_head(&transaction, &authority.auth_key, &alternate)
                .expect("replace current head");
        }
        transaction.commit().expect("commit tamper");

        let error = LedgerAuthority::open(
            fixture.root.path(),
            fixture.enterprise_id.clone(),
            fixture.trust_chain_digest.clone(),
        )
        .err()
        .expect("recovery must reject");
        if fork {
            assert!(error.contains("forked successor"), "{error}");
        } else {
            assert!(error.contains("does not extend"), "{error}");
        }
    }
}

#[test]
fn recovery_rejects_a_generation_gap() {
    let fixture = Fixture::new("generation-gap");
    let authority = fixture.authority();
    let seal_path = fixture
        .root
        .path()
        .join("private/ledger-authority-storage-seal.json");
    let generation_zero_seal = std::fs::read(&seal_path).expect("initial seal");
    let first = fixture.verified(fixture.signed(vec![fixture.event(1)], 1_001));
    authority
        .append_verified(&first)
        .expect("sealed first append");
    let second = fixture.verified(fixture.signed(vec![fixture.event(2)], 1_002));
    super::append::append(&authority, &second, AppendFailpoint::AfterCommitBeforeSeal)
        .expect_err("unsealed second append");
    std::fs::write(&seal_path, generation_zero_seal).expect("restore old authenticated seal");

    let error = LedgerAuthority::open(
        fixture.root.path(),
        fixture.enterprise_id.clone(),
        fixture.trust_chain_digest.clone(),
    )
    .err()
    .expect("generation gap must fail");
    assert!(error.contains("exactly one committed successor"), "{error}");
}

#[test]
fn file_seal_blocks_warm_roots_after_untouched_row_tampering() {
    let fixture = Fixture::new("seal");
    let authority = fixture.authority();
    let batch = fixture.verified(fixture.signed(vec![fixture.event(1)], 1_001));
    authority.append_verified(&batch).expect("append");
    fixture
        .connection()
        .execute(
            "UPDATE ledger_batches SET envelope_json = zeroblob(length(envelope_json))",
            [],
        )
        .expect("tamper batch row");
    let error = authority.read_head().expect_err("storage seal must fail");
    assert!(error.contains("changed outside"), "{error}");
    let reopen_error = LedgerAuthority::open(
        fixture.root.path(),
        fixture.enterprise_id.clone(),
        fixture.trust_chain_digest.clone(),
    )
    .err()
    .expect("same-generation mutation must not be resealed");
    assert!(
        reopen_error.contains("exactly one committed successor"),
        "{reopen_error}"
    );
}

#[test]
fn authenticated_event_identity_collision_is_rejected() {
    let fixture = Fixture::new("collision");
    let authority = fixture.authority();
    let shared = fixture.event(1);
    let first = fixture.verified(fixture.signed(vec![shared.clone()], 1_001));
    let first_receipt = authority.append_verified(&first).expect("first append");

    let forged_json = br#"{"forged":"event"}"#.to_vec();
    let digest = database::sha256_hex(&forged_json);
    let event_id = shared.event_id.as_str();
    let first_batch_id = first.batch().batch_id.as_str();
    let mac = database::auth_mac(
        &authority.auth_key,
        "ledger-event-v1",
        &(
            &fixture.enterprise_id,
            event_id,
            &digest,
            first_batch_id,
            &forged_json,
        ),
    )
    .expect("forged row mac");
    fixture
        .connection()
        .execute(
            "UPDATE ledger_events
             SET event_sha256 = ?2, event_json = ?3, mac = ?4
             WHERE event_id = ?1",
            params![event_id, digest, forged_json, mac],
        )
        .expect("forge event row");
    seal::write(
        fixture.root.path(),
        &authority.auth_key,
        &first_receipt.head,
    )
    .expect("reseal forged fixture");

    let second = fixture.verified(fixture.signed(vec![shared, fixture.event(2)], 1_002));
    let error = authority
        .append_verified(&second)
        .expect_err("event identity collision");
    assert!(error.contains("event-id collision"), "{error}");
}
