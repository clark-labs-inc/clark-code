use std::collections::BTreeSet;

use agent_orchestration::{
    CoverageKey, CoverageObservation, CoverageStatus, EnterpriseBatch, EnterpriseEvent,
    EnterpriseFact, EnterpriseProvenance,
};
use rusqlite::Connection;

use super::{call, status, Fixture};
use crate::{ScoutStoreRequest, ScoutStoreResponse};

#[test]
fn repeated_coverage_append_reads_one_authenticated_auxiliary_row_and_matches_cold() {
    let fixture = Fixture::new();
    let key = CoverageKey::new(
        "fixture",
        "fixture-auth",
        "tenant:fixture",
        "global",
        "service",
    )
    .unwrap();
    let first = coverage_envelope(&fixture, "coverage-a", 1, &key, 'a');
    fixture.ingest(first).unwrap();

    let second = coverage_envelope(&fixture, "coverage-b", 1, &key, 'b');
    let response = fixture.ingest(second).unwrap();
    let ScoutStoreResponse::Ingested {
        receipt: hot_receipt,
        ..
    } = response
    else {
        panic!("wrong coverage ingest response");
    };
    assert!(!hot_receipt.rebuilt);
    assert_eq!(hot_receipt.events_replayed, 1);
    assert_eq!(hot_receipt.event_ids_scanned, 0);
    assert_eq!(hot_receipt.entity_rows_read, 0);
    assert_eq!(hot_receipt.edge_rows_read, 0);
    assert_eq!(hot_receipt.history_rows_read, 0);
    assert_eq!(hot_receipt.auxiliary_rows_read, 1);
    assert_eq!(hot_receipt.affected_projection_rows, 1);
    assert!(!hot_receipt.full_projection_fallback);
    let hot_status = status(&fixture).0;
    assert_eq!(hot_status.coverage_cells, 1);

    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE meta SET value = 'force-auxiliary-cold'
             WHERE key = 'projection_version'",
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
        panic!("wrong auxiliary rebuild response");
    };
    assert_eq!(status(&fixture).0, hot_status);
    assert_eq!(cold_receipt.event_root, hot_receipt.event_root);
    assert_eq!(cold_receipt.graph_digest, hot_receipt.graph_digest);
    assert_eq!(
        cold_receipt.event_set_root_v1,
        hot_receipt.event_set_root_v1
    );
    assert_eq!(
        cold_receipt.projection_map_root_v2,
        hot_receipt.projection_map_root_v2
    );
    assert_eq!(
        cold_receipt.enterprise_snapshot_root_v2,
        hot_receipt.enterprise_snapshot_root_v2
    );
}

fn coverage_envelope(
    fixture: &Fixture,
    machine: &str,
    sequence: u64,
    key: &CoverageKey,
    evidence: char,
) -> agent_orchestration::EnterpriseSignedBatch {
    let observation = CoverageObservation::new(
        &fixture.enterprise,
        key.clone(),
        CoverageStatus::Supported,
        None,
        1,
        BTreeSet::from([evidence.to_string().repeat(64)]),
    )
    .unwrap();
    let event = EnterpriseEvent::new(
        fixture.enterprise.clone(),
        EnterpriseProvenance {
            machine_id: machine.into(),
            run_id: format!("run-{machine}"),
            adapter_instance_id: "fixture-adapter".into(),
            auth_context_id: "fixture-auth".into(),
            discovery_epoch: "epoch-1".into(),
            discovery_epoch_sequence: 1,
            source_sequence: sequence,
            observed_at_ms: 2_000 + sequence,
            source_fingerprint: "f".repeat(64),
        },
        EnterpriseFact::CoverageObserved(observation),
    )
    .unwrap();
    fixture.sign_batch(
        EnterpriseBatch::new(fixture.enterprise.clone(), [event]).unwrap(),
        machine,
        sequence,
    )
}
