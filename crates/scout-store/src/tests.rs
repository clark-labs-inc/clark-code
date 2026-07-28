use std::collections::BTreeSet;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact,
    EnterpriseGrantScope, EnterpriseId, EnterpriseLedgerCheckpoint, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseTrustChain, EnterpriseTrustManifest, GraphEntityObservation,
};

use crate::{
    dispatch, CheckpointExchangeBundle, EntityQuery, IngestOutcome, ObservedCheckpointStatus,
    ScoutStoreRequest, ScoutStoreResponse, StoredCheckpointBundle, SERVICE_NAME,
};

mod auxiliary_acceptance;
mod commitment_regressions;
mod hot_path_acceptance;
mod integrity_regressions;

pub(super) struct Fixture {
    pub(super) enterprise: EnterpriseId,
    pub(super) root: tempfile::TempDir,
    coordinator: EnterpriseSigningKey,
    manifest: EnterpriseTrustManifest,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let enterprise = EnterpriseId::new("ingest-enterprise").unwrap();
        let coordinator = EnterpriseSigningKey::from_seed([0x19; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise.clone(),
            "trust:00000000-0000-4000-8000-000000000019".into(),
            100,
            100_000,
            &coordinator,
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("trust")).unwrap();
        std::fs::create_dir_all(root.path().join("private")).unwrap();
        let chain = EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest.clone()],
        };
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
        let mut bootstrap = [0_u8; 40];
        bootstrap[..32].copy_from_slice(&[0x19; 32]);
        bootstrap[32..].copy_from_slice(&100_u64.to_le_bytes());
        std::fs::write(
            root.path().join("private/local-signing-bootstrap"),
            bootstrap,
        )
        .unwrap();
        std::fs::write(
            root.path().join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 3,
                "enterprise_id": enterprise,
                "anchor_manifest_id": chain.anchor_manifest_id,
                "local_signer_id": coordinator.signer_id(),
                "mode": "coordinator"
            }))
            .unwrap(),
        )
        .unwrap();
        Self {
            enterprise,
            root,
            coordinator,
            manifest,
        }
    }

    pub(super) fn envelope(&self, machine: &str, sequence: u64) -> EnterpriseSignedBatch {
        self.sign_batch(
            batch(&self.enterprise, machine, sequence),
            machine,
            sequence,
        )
    }

    fn sign_batch(
        &self,
        batch: EnterpriseBatch,
        machine: &str,
        sequence: u64,
    ) -> EnterpriseSignedBatch {
        let grant = EnterpriseSignerGrant::issue(
            &self.manifest,
            self.coordinator.signer_id(),
            self.coordinator.public_key_hex(),
            BTreeSet::from([
                EnterpriseSignerRole::Collector,
                EnterpriseSignerRole::Coordinator,
            ]),
            EnterpriseGrantScope {
                machine_id: machine.into(),
                run_id: format!("run-{machine}"),
                adapter_instance_id: "fixture-adapter".into(),
                auth_context_id: "fixture-auth".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: sequence,
                last_source_sequence: sequence,
            },
            100,
            100_000,
            &[&self.coordinator],
        )
        .unwrap();
        EnterpriseSignedBatch::sign(
            batch,
            &self.manifest,
            grant,
            1_000 + sequence,
            &self.coordinator,
        )
        .unwrap()
    }

    pub(super) fn ingest(
        &self,
        envelope: EnterpriseSignedBatch,
    ) -> Result<ScoutStoreResponse, String> {
        call(
            self.root.path(),
            ScoutStoreRequest::Ingest {
                enterprise_id: self.enterprise.clone(),
                envelope: Box::new(envelope),
            },
        )
    }

    fn issue_checkpoint(&self, now_ms: u64) -> crate::AuthenticatedCheckpointStatus {
        let response = call(
            self.root.path(),
            ScoutStoreRequest::IssueCheckpoint {
                enterprise_id: self.enterprise.clone(),
                now_ms,
            },
        )
        .unwrap();
        let ScoutStoreResponse::CheckpointIssued { status, .. } = response else {
            panic!("wrong checkpoint issue response");
        };
        status
    }

    fn export_checkpoint(&self, sequence: u64) -> CheckpointExchangeBundle {
        let response = call(
            self.root.path(),
            ScoutStoreRequest::ExportCheckpoint {
                enterprise_id: self.enterprise.clone(),
                sequence,
            },
        )
        .unwrap();
        let ScoutStoreResponse::CheckpointExported { exchange } = response else {
            panic!("wrong checkpoint export response");
        };
        *exchange
    }

    fn observe_checkpoint(
        &self,
        exchange: CheckpointExchangeBundle,
    ) -> Result<(ObservedCheckpointStatus, bool), String> {
        let response = call(
            self.root.path(),
            ScoutStoreRequest::ObserveCheckpoint {
                enterprise_id: self.enterprise.clone(),
                exchange: Box::new(exchange),
            },
        )?;
        let ScoutStoreResponse::CheckpointObserved { status, idempotent } = response else {
            return Err("wrong checkpoint observation response".into());
        };
        Ok((status, idempotent))
    }

    fn make_replica(&self) {
        std::fs::write(
            self.root.path().join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 3,
                "enterprise_id": self.enterprise,
                "anchor_manifest_id": self.manifest.manifest_id,
                "local_signer_id": self.coordinator.signer_id(),
                "mode": "replica"
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

fn batch(enterprise: &EnterpriseId, machine: &str, sequence: u64) -> EnterpriseBatch {
    batch_for_native(
        enterprise,
        machine,
        sequence,
        format!("resource:{machine}:{sequence}"),
    )
}

fn batch_for_native(
    enterprise: &EnterpriseId,
    machine: &str,
    sequence: u64,
    native_id: String,
) -> EnterpriseBatch {
    let observation = GraphEntityObservation::new(
        enterprise,
        EnterpriseEntityKind::CloudResource,
        AuthorityRef::new("fixture", "tenant:fixture", native_id).unwrap(),
        BTreeSet::from([format!("resource-{sequence}")]),
        BTreeSet::from(["a".repeat(64)]),
    )
    .unwrap();
    let event = EnterpriseEvent::new(
        enterprise.clone(),
        EnterpriseProvenance {
            machine_id: machine.into(),
            run_id: format!("run-{machine}"),
            adapter_instance_id: "fixture-adapter".into(),
            auth_context_id: "fixture-auth".into(),
            discovery_epoch: "epoch-1".into(),
            discovery_epoch_sequence: 1,
            source_sequence: sequence,
            observed_at_ms: 500 + sequence,
            source_fingerprint: "f".repeat(64),
        },
        EnterpriseFact::EntityObserved(observation),
    )
    .unwrap();
    EnterpriseBatch::new(enterprise.clone(), [event]).unwrap()
}

fn call(root: &std::path::Path, request: ScoutStoreRequest) -> Result<ScoutStoreResponse, String> {
    let response = dispatch(SERVICE_NAME, root, &serde_json::to_vec(&request).unwrap())?;
    serde_json::from_slice(&response).map_err(|error| error.to_string())
}

fn status(fixture: &Fixture) -> (crate::IndexedStatus, crate::IndexReceipt) {
    let response = call(
        fixture.root.path(),
        ScoutStoreRequest::Status {
            enterprise_id: fixture.enterprise.clone(),
        },
    )
    .unwrap();
    let ScoutStoreResponse::Status { status, receipt } = response else {
        panic!("wrong status response");
    };
    (*status, receipt)
}

fn observed_directory(root: &std::path::Path, coordinator_id: &str) -> std::path::PathBuf {
    root.join("private/observed-checkpoints")
        .join(coordinator_id.strip_prefix("signer:").unwrap())
}

fn alternate_exchange(
    source: &Fixture,
    original: &CheckpointExchangeBundle,
    issued_at_ms: u64,
) -> CheckpointExchangeBundle {
    let batches = crate::index::ledger_authority::open(source.root.path(), &source.enterprise)
        .unwrap()
        .authority
        .read_all_envelopes()
        .unwrap()
        .envelopes
        .into_iter()
        .map(|generation| generation.envelope.batch)
        .collect::<Vec<_>>();
    let ledger_commitment = original
        .bundle
        .checkpoint
        .ledger_commitment
        .clone()
        .expect("new checkpoints must bind the typed ledger commitment");
    let checkpoint = EnterpriseLedgerCheckpoint::issue_v2(
        &source.manifest,
        original.bundle.checkpoint.sequence,
        original.bundle.checkpoint.previous_checkpoint_id.clone(),
        issued_at_ms,
        ledger_commitment,
        original.bundle.checkpoint.snapshot_commitment_v2.clone(),
        &[&source.coordinator],
    )
    .unwrap();
    CheckpointExchangeBundle {
        coordinator_id: source.coordinator.signer_id(),
        anchor_manifest_id: source.manifest.manifest_id.clone(),
        bundle: StoredCheckpointBundle {
            checkpoint,
            added_batch_ids: batches.into_iter().map(|batch| batch.batch_id).collect(),
        },
    }
}

#[test]
fn ingest_is_durable_indexed_and_retry_safe() {
    let fixture = Fixture::new();
    let envelope = fixture.envelope("machine-a", 1);
    let first = fixture.ingest(envelope.clone()).unwrap();
    let ScoutStoreResponse::Ingested { outcome, receipt } = first else {
        panic!("wrong ingest response");
    };
    assert_eq!(outcome, IngestOutcome::Inserted);
    assert!(receipt.rebuilt);
    assert_eq!(receipt.derived_batches_read, 1);
    assert_eq!(receipt.ledger_authority_work.envelope_rows_read, 1);
    assert_eq!(receipt.projection_rows_written, 2);
    assert_eq!(receipt.projection_rows_deleted, 0);
    assert!(receipt.supplemental_rows_written > 0);
    assert!(receipt.supplemental_rows_written < 100);
    assert!(receipt.supplemental_rows_deleted < 100);
    assert!(receipt
        .event_set_root_v1
        .as_deref()
        .is_some_and(|root| root.starts_with("scout-event-set-v1:")));
    assert!(receipt
        .projection_map_root_v2
        .as_deref()
        .is_some_and(|root| root.starts_with("scout-projection-map-v2:")));
    assert!(receipt
        .enterprise_snapshot_root_v2
        .as_deref()
        .is_some_and(|root| root.starts_with("scout-enterprise-snapshot-v2:")));
    let encoded_receipt = serde_json::to_value(&receipt).unwrap();
    assert!(encoded_receipt.get("projection_map_root_v1").is_none());
    assert!(encoded_receipt.get("enterprise_snapshot_root_v1").is_none());

    let retry = fixture.ingest(envelope).unwrap();
    let ScoutStoreResponse::Ingested { outcome, receipt } = retry else {
        panic!("wrong retry response");
    };
    assert_eq!(outcome, IngestOutcome::AlreadyPresent);
    assert!(!receipt.rebuilt);
    assert_eq!(receipt.projection_rows_written, 0);
    assert_eq!(receipt.projection_rows_deleted, 0);
}

#[test]
fn neighborhood_query_reads_the_provider_namespace_projection_column() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let entities = call(
        fixture.root.path(),
        ScoutStoreRequest::Entities {
            enterprise_id: fixture.enterprise.clone(),
            query: EntityQuery {
                limit: 10,
                ..EntityQuery::default()
            },
        },
    )
    .unwrap();
    let ScoutStoreResponse::Entities { page, .. } = entities else {
        panic!("wrong entity query response");
    };
    let seed = page.entities[0].entity_id.clone();

    let neighborhood = call(
        fixture.root.path(),
        ScoutStoreRequest::Neighborhood {
            enterprise_id: fixture.enterprise.clone(),
            seed,
            depth: 1,
            limit: 10,
        },
    )
    .unwrap();
    let ScoutStoreResponse::Neighborhood { page, .. } = neighborhood else {
        panic!("wrong neighborhood query response");
    };
    assert_eq!(page.entities.len(), 1);
    assert!(!page.truncated);
}

#[test]
fn ordinary_append_reads_no_prior_envelopes_and_writes_only_affected_rows() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let second = fixture.ingest(fixture.envelope("machine-b", 1)).unwrap();
    let ScoutStoreResponse::Ingested { outcome, receipt } = second else {
        panic!("wrong second ingest response");
    };
    assert_eq!(outcome, IngestOutcome::Inserted);
    assert!(!receipt.rebuilt);
    assert_eq!(receipt.derived_batches_read, 0);
    assert_eq!(receipt.events_replayed, 0);
    assert_eq!(receipt.affected_projection_rows, 1);
    assert!(!receipt.full_projection_fallback);
    assert_eq!(receipt.projection_rows_written, 2);
    assert_eq!(receipt.projection_rows_deleted, 0);
}

#[test]
fn warm_status_reads_one_ledger_head_without_envelopes() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();

    for _ in 0..3 {
        let (indexed, receipt) = status(&fixture);
        assert_eq!((indexed.batches, indexed.events), (1, 1));
        assert_eq!(
            receipt.ledger_authority_work,
            crate::ledger_authority::LedgerAuthorityWork {
                head_rows_read: 1,
                ..crate::ledger_authority::LedgerAuthorityWork::default()
            }
        );
        assert_eq!(receipt.derived_batches_read, 0);
    }
}

#[test]
fn one_generation_projection_lag_replays_one_authoritative_successor() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let index_path = fixture.root.path().join("index-v4.sqlite3");
    let generation_one_index = std::fs::read(&index_path).unwrap();

    fixture.ingest(fixture.envelope("machine-b", 1)).unwrap();
    std::fs::write(&index_path, generation_one_index).unwrap();

    let (indexed, receipt) = status(&fixture);
    assert_eq!(
        (indexed.batches, indexed.events, indexed.entities),
        (2, 2, 2)
    );
    assert!(!receipt.rebuilt);
    assert_eq!(receipt.derived_batches_read, 0);
    assert_eq!(receipt.ledger_authority_work.envelope_rows_read, 1);
    assert!(receipt.events_replayed <= 1);
}

#[test]
fn incremental_append_survives_restart_and_matches_a_forced_cold_rebuild() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let incremental = fixture.ingest(fixture.envelope("machine-b", 1)).unwrap();
    let ScoutStoreResponse::Ingested {
        receipt: incremental_receipt,
        ..
    } = incremental
    else {
        panic!("wrong incremental ingest response");
    };
    let (incremental_status, _warm_receipt) = status(&fixture);

    let connection =
        rusqlite::Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE meta SET value = 'force-cold-rebuild' WHERE key = 'projection_version'",
            [],
        )
        .unwrap();
    drop(connection);

    let rebuilt = call(
        fixture.root.path(),
        ScoutStoreRequest::Rebuild {
            enterprise_id: fixture.enterprise.clone(),
        },
    )
    .unwrap();
    let ScoutStoreResponse::Rebuilt(cold_receipt) = rebuilt else {
        panic!("wrong rebuild response");
    };
    let (cold_status, _) = status(&fixture);
    assert!(cold_receipt.rebuilt);
    assert_eq!(cold_receipt.derived_batches_read, 2);
    assert_eq!(cold_receipt.ledger_authority_work.envelope_rows_read, 2);
    assert_eq!(cold_status, incremental_status);
    assert_eq!(cold_receipt.event_root, incremental_receipt.event_root);
    assert_eq!(cold_receipt.graph_digest, incremental_receipt.graph_digest);
    assert_eq!(
        cold_receipt.event_set_root_v1,
        incremental_receipt.event_set_root_v1
    );
    assert_eq!(
        cold_receipt.projection_map_root_v2,
        incremental_receipt.projection_map_root_v2
    );
    assert_eq!(
        cold_receipt.enterprise_snapshot_root_v2,
        incremental_receipt.enterprise_snapshot_root_v2
    );
}

#[test]
fn authenticated_commitment_entry_tamper_falls_back_to_cold_rebuild() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let connection =
        rusqlite::Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE commitment_entries SET value_digest = X'00'
             WHERE lane = 'projection-map-v2'",
            [],
        )
        .unwrap();
    drop(connection);

    let response = fixture.ingest(fixture.envelope("machine-b", 1)).unwrap();
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong commitment recovery response");
    };
    assert!(receipt.rebuilt);
    assert_eq!(receipt.derived_batches_read, 2);
    assert!(receipt.supplemental_rows_written > 0);
    assert!(receipt.event_set_root_v1.is_some());
    assert!(receipt.projection_map_root_v2.is_some());
    assert_eq!(status(&fixture).0.entities, 2);
}

#[test]
fn missing_compact_commitment_entry_is_hidden_on_read_and_rebuilt_before_append() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let connection =
        rusqlite::Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    let key = connection
        .query_row(
            "SELECT lane, partition_id, object_id FROM commitment_entries LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "DELETE FROM commitment_entries
                 WHERE lane = ?1 AND partition_id = ?2 AND object_id = ?3",
                rusqlite::params![key.0, key.1, key.2],
            )
            .unwrap(),
        1
    );
    drop(connection);

    let (indexed_status, receipt) = status(&fixture);
    assert!(!receipt.rebuilt);
    assert_eq!(indexed_status.entities, 1);
    assert!(receipt.event_set_root_v1.is_none());
    assert!(receipt.projection_map_root_v2.is_none());
    assert!(receipt.enterprise_snapshot_root_v2.is_none());

    let response = fixture.ingest(fixture.envelope("machine-b", 1)).unwrap();
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong recovery ingest response");
    };
    assert!(receipt.rebuilt);
    assert_eq!(receipt.derived_batches_read, 2);
    assert_eq!(status(&fixture).0.entities, 2);
}

#[test]
fn repeated_key_append_replays_only_that_key_and_matches_a_cold_rebuild() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    fixture.ingest(fixture.envelope("machine-z", 1)).unwrap();
    let repeated = fixture.sign_batch(
        batch_for_native(
            &fixture.enterprise,
            "machine-b",
            1,
            "resource:machine-a:1".into(),
        ),
        "machine-b",
        1,
    );
    let response = fixture.ingest(repeated).unwrap();
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong repeated-key ingest response");
    };
    assert!(!receipt.rebuilt);
    assert_eq!(receipt.derived_batches_read, 0);
    assert_eq!(receipt.events_replayed, 1);
    assert_eq!(receipt.affected_projection_rows, 1);
    let (incremental_status, _) = status(&fixture);

    let connection =
        rusqlite::Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE meta SET value = 'force-cold-rebuild' WHERE key = 'projection_version'",
            [],
        )
        .unwrap();
    drop(connection);
    let (cold_status, cold_receipt) = status(&fixture);
    assert!(cold_receipt.rebuilt);
    assert_eq!(cold_receipt.derived_batches_read, 3);
    assert_eq!(cold_status, incremental_status);
}

#[test]
fn authenticated_event_cache_tamper_falls_back_when_the_row_is_consumed() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let connection =
        rusqlite::Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE cached_events SET event_json = '{\"forged\":true}'",
            [],
        )
        .unwrap();
    drop(connection);

    let second = fixture.sign_batch(
        batch_for_native(
            &fixture.enterprise,
            "machine-b",
            1,
            "resource:machine-a:1".into(),
        ),
        "machine-b",
        1,
    );
    let response = fixture.ingest(second).unwrap();
    let ScoutStoreResponse::Ingested { outcome, receipt } = response else {
        panic!("wrong recovery ingest response");
    };
    assert_eq!(outcome, IngestOutcome::Inserted);
    assert!(receipt.rebuilt);
    assert!(receipt.full_projection_fallback);
    assert_eq!(receipt.derived_batches_read, 2);
    assert_eq!(status(&fixture).0.entities, 1);

    let third = fixture.sign_batch(
        batch_for_native(
            &fixture.enterprise,
            "machine-c",
            1,
            "resource:machine-a:1".into(),
        ),
        "machine-c",
        1,
    );
    let ScoutStoreResponse::Ingested { receipt, .. } = fixture.ingest(third).unwrap() else {
        panic!("wrong post-recovery ingest response");
    };
    assert!(!receipt.rebuilt);
    assert_eq!(receipt.events_replayed, 2);
}

#[test]
fn authenticated_projection_row_tamper_falls_back_on_the_next_append() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let connection =
        rusqlite::Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE entities SET materialized_json = '{\"forged\":true}'",
            [],
        )
        .unwrap();
    drop(connection);

    let response = fixture.ingest(fixture.envelope("machine-b", 1)).unwrap();
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong projection recovery response");
    };
    assert!(receipt.rebuilt);
    assert_eq!(receipt.derived_batches_read, 2);
    assert_eq!(status(&fixture).0.entities, 2);
}

#[test]
fn retraction_uses_immutable_full_projection_fallback_and_remains_exact() {
    let fixture = Fixture::new();
    let first = fixture.envelope("machine-a", 1);
    let target_event_id = first.batch.events[0].event_id.clone();
    fixture.ingest(first).unwrap();
    let retraction = EnterpriseEvent::new(
        fixture.enterprise.clone(),
        EnterpriseProvenance {
            machine_id: "machine-b".into(),
            run_id: "run-machine-b".into(),
            adapter_instance_id: "fixture-adapter".into(),
            auth_context_id: "fixture-auth".into(),
            discovery_epoch: "epoch-1".into(),
            discovery_epoch_sequence: 1,
            source_sequence: 1,
            observed_at_ms: 900,
            source_fingerprint: "c".repeat(64),
        },
        EnterpriseFact::ObservationRetracted {
            target_event_id,
            reason: "authoritative resource deletion".into(),
            evidence_digests: BTreeSet::from(["b".repeat(64)]),
        },
    )
    .unwrap();
    let batch = EnterpriseBatch::new(fixture.enterprise.clone(), [retraction]).unwrap();
    let envelope = fixture.sign_batch(batch, "machine-b", 1);

    let response = fixture.ingest(envelope).unwrap();
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong retraction ingest response");
    };
    assert!(receipt.rebuilt);
    assert!(receipt.full_projection_fallback);
    assert_eq!(receipt.derived_batches_read, 2);
    let (status, _warm_receipt) = status(&fixture);
    assert_eq!(status.events, 2);
    assert_eq!(status.entities, 0);
    assert_eq!(status.event_root, receipt.event_root);
    assert_eq!(status.graph_digest, receipt.graph_digest);
}

#[test]
fn corrupt_projection_falls_back_to_authenticated_ledger_rebuild() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    std::fs::write(fixture.root.path().join("index-v4.sqlite3"), b"not sqlite").unwrap();

    let response = fixture.ingest(fixture.envelope("machine-b", 1)).unwrap();
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong corruption recovery response");
    };
    assert!(receipt.rebuilt);
    assert_eq!(receipt.derived_batches_read, 2);
    assert_eq!(status(&fixture).0.entities, 2);
}

#[test]
fn concurrent_ingests_share_one_target_lock_and_converge() {
    let fixture = std::sync::Arc::new(Fixture::new());
    let left = fixture.envelope("machine-a", 1);
    let right = fixture.envelope("machine-b", 1);
    let left_fixture = fixture.clone();
    let right_fixture = fixture.clone();
    let left = std::thread::spawn(move || left_fixture.ingest(left));
    let right = std::thread::spawn(move || right_fixture.ingest(right));
    assert!(left.join().unwrap().is_ok());
    assert!(right.join().unwrap().is_ok());

    let status = call(
        fixture.root.path(),
        ScoutStoreRequest::Status {
            enterprise_id: fixture.enterprise.clone(),
        },
    )
    .unwrap();
    let ScoutStoreResponse::Status { status, receipt: _ } = status else {
        panic!("wrong status response");
    };
    assert_eq!(status.batches, 2);
    assert_eq!(status.entities, 2);
}

#[test]
fn target_ingest_rejects_a_self_reported_replacement_trust_anchor() {
    let fixture = Fixture::new();
    let rogue = EnterpriseSigningKey::from_seed([0x88; 32]);
    let rogue_root = EnterpriseTrustManifest::initial(
        fixture.enterprise.clone(),
        "trust:00000000-0000-4000-8000-000000000088".into(),
        100,
        100_000,
        &rogue,
    )
    .unwrap();
    let rogue_chain = EnterpriseTrustChain {
        anchor_manifest_id: rogue_root.manifest_id.clone(),
        manifests: vec![rogue_root],
    };
    std::fs::write(
        fixture.root.path().join("trust/chain.json"),
        serde_json::to_vec(&rogue_chain).unwrap(),
    )
    .unwrap();

    let error = fixture
        .ingest(fixture.envelope("machine-a", 1))
        .unwrap_err();
    assert!(error.contains("private anchor pin"), "{error}");
}

#[test]
fn warm_query_rejects_a_valid_sqlite_row_without_its_private_mac() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("machine-a", 1)).unwrap();
    let connection =
        rusqlite::Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE entities SET materialized_json = ?1",
            [r#"{"forged":true}"#],
        )
        .unwrap();
    drop(connection);

    let error = call(
        fixture.root.path(),
        ScoutStoreRequest::Entities {
            enterprise_id: fixture.enterprise.clone(),
            query: EntityQuery {
                limit: 10,
                ..EntityQuery::default()
            },
        },
    )
    .unwrap_err();
    assert!(error.contains("authentication failed"), "{error}");
}

#[test]
fn replica_observes_exported_checkpoint_and_duplicate_is_idempotent() {
    let source = Fixture::new();
    let envelope = source.envelope("machine-a", 1);
    let ingested = source.ingest(envelope.clone()).unwrap();
    let ScoutStoreResponse::Ingested {
        receipt: ingest_receipt,
        ..
    } = ingested
    else {
        panic!("wrong source ingest response");
    };
    let issued = source.issue_checkpoint(1_000);
    let commitment = issued
        .snapshot_commitment_v2
        .as_ref()
        .expect("coordinator checkpoint must bind the v2 materialized snapshot");
    assert_eq!(commitment.graph_digest, ingest_receipt.graph_digest);
    assert_eq!(
        Some(&commitment.event_set_root_v1),
        ingest_receipt.event_set_root_v1.as_ref()
    );
    assert_eq!(
        Some(&commitment.projection_map_root_v2),
        ingest_receipt.projection_map_root_v2.as_ref()
    );
    assert_eq!(
        Some(&commitment.enterprise_snapshot_root_v2),
        ingest_receipt.enterprise_snapshot_root_v2.as_ref()
    );
    assert!(issued.checkpoint_covers_current_projection);
    let retry = call(
        source.root.path(),
        ScoutStoreRequest::IssueCheckpoint {
            enterprise_id: source.enterprise.clone(),
            now_ms: 1_001,
        },
    )
    .unwrap();
    let ScoutStoreResponse::CheckpointIssued {
        status: duplicate,
        idempotent: true,
    } = retry
    else {
        panic!("unchanged signed snapshot checkpoint was not idempotent");
    };
    assert_eq!(duplicate, issued);
    let exchange = source.export_checkpoint(issued.sequence);

    let replica = Fixture::new();
    replica.make_replica();
    replica.ingest(envelope).unwrap();
    let (observed, idempotent) = replica.observe_checkpoint(exchange.clone()).unwrap();
    assert!(!idempotent);
    assert_eq!(observed.checkpoint_id, issued.checkpoint_id);
    assert_eq!(observed.sequence, 1);
    assert_eq!(observed.coordinator_id, source.coordinator.signer_id());
    assert_eq!(
        observed.snapshot_commitment_v2,
        issued.snapshot_commitment_v2
    );

    let (duplicate, idempotent) = replica.observe_checkpoint(exchange).unwrap();
    assert!(idempotent);
    assert_eq!(duplicate, observed);
    let directory = observed_directory(replica.root.path(), &observed.coordinator_id);
    assert!(directory.join("00000000000000000001.json").is_file());
    assert!(directory.join("cursor.json").is_file());
    assert!(!directory.join("checkpoints").exists());
}

#[test]
fn observation_rejects_anchor_gap_fork_and_regression() {
    let source = Fixture::new();
    let first_envelope = source.envelope("machine-a", 1);
    source.ingest(first_envelope.clone()).unwrap();
    let first = source.issue_checkpoint(1_000);
    let first_exchange = source.export_checkpoint(first.sequence);
    assert_eq!(first_exchange.bundle.added_batch_ids.len(), 1);
    let second_envelope = source.envelope("machine-a", 2);
    source.ingest(second_envelope.clone()).unwrap();
    let second = source.issue_checkpoint(1_100);
    let second_exchange = source.export_checkpoint(second.sequence);
    assert_eq!(second_exchange.bundle.added_batch_ids.len(), 1);
    assert!(first_exchange
        .bundle
        .added_batch_ids
        .is_disjoint(&second_exchange.bundle.added_batch_ids));

    let replica = Fixture::new();
    replica.make_replica();
    replica.ingest(first_envelope).unwrap();
    replica.ingest(second_envelope).unwrap();

    let mut wrong_anchor = first_exchange.clone();
    wrong_anchor.anchor_manifest_id = format!("trust-manifest:{}", "0".repeat(64));
    let error = replica.observe_checkpoint(wrong_anchor).unwrap_err();
    assert!(error.contains("target-private trust anchor"), "{error}");

    let error = replica
        .observe_checkpoint(second_exchange.clone())
        .unwrap_err();
    assert!(error.contains("sequence one"), "{error}");

    replica.observe_checkpoint(first_exchange.clone()).unwrap();
    let mut repeated_delta = second_exchange.clone();
    repeated_delta.bundle.added_batch_ids = first_exchange.bundle.added_batch_ids.clone();
    let error = replica.observe_checkpoint(repeated_delta).unwrap_err();
    assert!(error.contains("repeats an existing batch"), "{error}");

    let fork = alternate_exchange(&source, &first_exchange, 1_001);
    let error = replica.observe_checkpoint(fork).unwrap_err();
    assert!(error.contains("conflicting ledger checkpoints"), "{error}");

    replica
        .observe_checkpoint(second_exchange)
        .expect("continuous successor should advance");
    let error = replica.observe_checkpoint(first_exchange).unwrap_err();
    assert!(error.contains("rollback"), "{error}");
}

#[test]
fn observed_cursor_recovers_create_only_checkpoint_before_next_observation() {
    let source = Fixture::new();
    let first_envelope = source.envelope("machine-a", 1);
    source.ingest(first_envelope.clone()).unwrap();
    source.issue_checkpoint(1_000);
    let first_exchange = source.export_checkpoint(1);
    let second_envelope = source.envelope("machine-a", 2);
    source.ingest(second_envelope.clone()).unwrap();
    source.issue_checkpoint(1_100);
    let second_exchange = source.export_checkpoint(2);

    let replica = Fixture::new();
    replica.make_replica();
    replica.ingest(first_envelope).unwrap();
    replica.ingest(second_envelope).unwrap();
    replica.observe_checkpoint(first_exchange.clone()).unwrap();
    let directory = observed_directory(replica.root.path(), &first_exchange.coordinator_id);
    std::fs::remove_file(directory.join("cursor.json")).unwrap();

    let (observed, idempotent) = replica.observe_checkpoint(second_exchange).unwrap();
    assert!(!idempotent);
    assert_eq!(observed.sequence, 2);
    let cursor = std::fs::read_to_string(directory.join("cursor.json")).unwrap();
    assert!(cursor.contains("\"highest_sequence\":2"), "{cursor}");
}
