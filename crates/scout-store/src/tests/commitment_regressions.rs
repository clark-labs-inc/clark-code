use std::collections::BTreeSet;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseClassification, EnterpriseEdgeKind,
    EnterpriseEntityKind, EnterpriseFact, EnterpriseGrantScope, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, GraphEdgeObservation,
    GraphEntityObservation, MaterializedEdge,
};
use rusqlite::Connection;

use super::{call, status, Fixture};
use crate::{
    EdgeQuery, EntityQuery, IndexReceipt, IndexedStatus, ScoutStoreRequest, ScoutStoreResponse,
};

#[test]
fn same_batch_restricted_endpoint_reclassifies_edge_and_hot_equals_cold() {
    let fixture = Fixture::new();
    fixture.ingest(fixture.envelope("seed", 1)).unwrap();

    let restricted = entity(
        &fixture,
        "service:restricted",
        EnterpriseClassification::Restricted,
    );
    let internal = entity(
        &fixture,
        "service:internal",
        EnterpriseClassification::Internal,
    );
    let edge = edge(&fixture, &restricted, &internal, EnterpriseEdgeKind::Calls);
    let response = fixture
        .ingest(signed_facts(
            &fixture,
            "same-batch",
            10,
            vec![
                EnterpriseFact::EntityObserved(restricted),
                EnterpriseFact::EntityObserved(internal),
                EnterpriseFact::EdgeObserved(edge),
            ],
        ))
        .unwrap();
    let ScoutStoreResponse::Ingested {
        receipt: hot_receipt,
        ..
    } = response
    else {
        panic!("wrong same-batch ingest response");
    };
    assert!(!hot_receipt.rebuilt, "same-batch case did not use hot path");
    let (hot_status, _) = status(&fixture);
    let hot_edges = stored_edges(&fixture);
    let hot_visible = visible_edges(&fixture).0;

    let (cold_status, cold_receipt) = force_cold(&fixture);
    let cold_edges = stored_edges(&fixture);

    assert_eq!(hot_edges, cold_edges, "hot and cold edge rows diverged");
    assert_eq!(hot_status, cold_status, "hot and cold status diverged");
    assert_roots_equal(&hot_receipt, &cold_receipt);
    assert_eq!(hot_edges.len(), 1);
    assert_eq!(
        hot_edges[0].classification,
        EnterpriseClassification::Restricted
    );
    assert!(
        hot_visible.is_empty(),
        "Internal query exposed an edge incident to a Restricted endpoint"
    );
}

#[test]
fn endpoint_upgrade_reclassifies_multi_edge_fanout_and_hot_equals_cold() {
    let fixture = Fixture::new();
    let upgraded = entity(
        &fixture,
        "service:upgraded",
        EnterpriseClassification::Internal,
    );
    let peer_a = entity(
        &fixture,
        "service:peer-a",
        EnterpriseClassification::Internal,
    );
    let peer_b = entity(
        &fixture,
        "service:peer-b",
        EnterpriseClassification::Internal,
    );
    let initial = signed_facts(
        &fixture,
        "initial-graph",
        10,
        vec![
            EnterpriseFact::EntityObserved(upgraded.clone()),
            EnterpriseFact::EntityObserved(peer_a.clone()),
            EnterpriseFact::EntityObserved(peer_b.clone()),
            EnterpriseFact::EdgeObserved(edge(
                &fixture,
                &upgraded,
                &peer_a,
                EnterpriseEdgeKind::Calls,
            )),
            EnterpriseFact::EdgeObserved(edge(
                &fixture,
                &upgraded,
                &peer_b,
                EnterpriseEdgeKind::Reads,
            )),
            EnterpriseFact::EdgeObserved(edge(
                &fixture,
                &peer_b,
                &upgraded,
                EnterpriseEdgeKind::Writes,
            )),
        ],
    );
    let ScoutStoreResponse::Ingested { receipt, .. } = fixture.ingest(initial).unwrap() else {
        panic!("wrong initial graph response");
    };
    assert!(receipt.rebuilt);
    assert_eq!(visible_edges(&fixture).0.len(), 3);

    let upgraded = entity(
        &fixture,
        "service:upgraded",
        EnterpriseClassification::Restricted,
    );
    let response = fixture
        .ingest(signed_facts(
            &fixture,
            "classification-upgrade",
            30,
            vec![EnterpriseFact::EntityObserved(upgraded)],
        ))
        .unwrap();
    let ScoutStoreResponse::Ingested {
        receipt: hot_receipt,
        ..
    } = response
    else {
        panic!("wrong classification upgrade response");
    };
    assert!(
        !hot_receipt.rebuilt,
        "classification upgrade did not use hot path"
    );
    let (hot_status, _) = status(&fixture);
    let hot_edges = stored_edges(&fixture);
    let hot_visible = visible_edges(&fixture).0;

    let (cold_status, cold_receipt) = force_cold(&fixture);
    let cold_edges = stored_edges(&fixture);

    assert_eq!(hot_edges, cold_edges, "hot and cold fanout rows diverged");
    assert_eq!(hot_status, cold_status, "hot and cold status diverged");
    assert_roots_equal(&hot_receipt, &cold_receipt);
    assert_eq!(hot_edges.len(), 3);
    assert!(hot_edges
        .iter()
        .all(|edge| edge.classification == EnterpriseClassification::Restricted));
    assert!(
        hot_visible.is_empty(),
        "Internal query exposed incident edges after endpoint upgrade"
    );
}

#[test]
fn ordinary_append_after_restricted_rows_stays_incremental() {
    let fixture = Fixture::new();
    let restricted = entity(
        &fixture,
        "service:restricted",
        EnterpriseClassification::Restricted,
    );
    let internal = entity(
        &fixture,
        "service:internal",
        EnterpriseClassification::Internal,
    );
    let initial = signed_facts(
        &fixture,
        "classified-initial",
        10,
        vec![
            EnterpriseFact::EntityObserved(restricted.clone()),
            EnterpriseFact::EntityObserved(internal.clone()),
            EnterpriseFact::EdgeObserved(edge(
                &fixture,
                &restricted,
                &internal,
                EnterpriseEdgeKind::Calls,
            )),
        ],
    );
    let ScoutStoreResponse::Ingested { receipt, .. } = fixture.ingest(initial).unwrap() else {
        panic!("wrong classified initial response");
    };
    assert!(receipt.rebuilt);
    assert_eq!(
        stored_edges(&fixture)[0].classification,
        EnterpriseClassification::Restricted
    );

    let ordinary = entity(
        &fixture,
        "service:ordinary",
        EnterpriseClassification::Internal,
    );
    let response = fixture
        .ingest(signed_facts(
            &fixture,
            "ordinary-append",
            20,
            vec![EnterpriseFact::EntityObserved(ordinary)],
        ))
        .unwrap();
    let ScoutStoreResponse::Ingested { receipt, .. } = response else {
        panic!("wrong ordinary append response");
    };
    assert!(!receipt.rebuilt, "Restricted rows forced a cold rebuild");
    assert!(!receipt.full_projection_fallback);
    assert_eq!(receipt.derived_batches_read, 0);
}

#[test]
fn query_and_status_receipts_hide_supplemental_roots() {
    let fixture = Fixture::new();
    let ScoutStoreResponse::Ingested {
        receipt: ingest_receipt,
        ..
    } = fixture.ingest(fixture.envelope("machine-a", 1)).unwrap()
    else {
        panic!("wrong ingest response");
    };
    assert!(ingest_receipt.event_set_root_v1.is_some());
    assert!(ingest_receipt.projection_map_root_v2.is_some());
    assert!(ingest_receipt.enterprise_snapshot_root_v2.is_some());

    let (_, status_receipt) = status(&fixture);
    assert_supplemental_roots_hidden(&status_receipt);

    let response = call(
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
    let ScoutStoreResponse::Entities { receipt, .. } = response else {
        panic!("wrong entity query response");
    };
    assert_supplemental_roots_hidden(&receipt);
}

fn signed_facts(
    fixture: &Fixture,
    machine: &str,
    first_sequence: u64,
    facts: Vec<EnterpriseFact>,
) -> EnterpriseSignedBatch {
    let events = facts
        .into_iter()
        .enumerate()
        .map(|(offset, fact)| {
            let sequence = first_sequence + u64::try_from(offset).unwrap();
            agent_orchestration::EnterpriseEvent::new(
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
                fact,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let last_sequence = first_sequence + u64::try_from(events.len() - 1).unwrap();
    let batch = EnterpriseBatch::new(fixture.enterprise.clone(), events).unwrap();
    let grant = EnterpriseSignerGrant::issue(
        &fixture.manifest,
        fixture.coordinator.signer_id(),
        fixture.coordinator.public_key_hex(),
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
            first_source_sequence: first_sequence,
            last_source_sequence: last_sequence,
        },
        100,
        100_000,
        &[&fixture.coordinator],
    )
    .unwrap();
    EnterpriseSignedBatch::sign(
        batch,
        &fixture.manifest,
        grant,
        10_000,
        &fixture.coordinator,
    )
    .unwrap()
}

fn entity(
    fixture: &Fixture,
    native_id: &str,
    classification: EnterpriseClassification,
) -> GraphEntityObservation {
    let mut entity = GraphEntityObservation::new(
        &fixture.enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("fixture", "tenant:fixture", native_id).unwrap(),
        BTreeSet::from([native_id.replace(':', "-")]),
        BTreeSet::from(["a".repeat(64)]),
    )
    .unwrap();
    entity.classification = classification;
    entity
}

fn edge(
    fixture: &Fixture,
    from: &GraphEntityObservation,
    to: &GraphEntityObservation,
    kind: EnterpriseEdgeKind,
) -> GraphEdgeObservation {
    GraphEdgeObservation::new(
        &fixture.enterprise,
        from.entity_id.clone(),
        to.entity_id.clone(),
        kind,
        None,
        BTreeSet::from(["b".repeat(64)]),
    )
    .unwrap()
}

fn stored_edges(fixture: &Fixture) -> Vec<MaterializedEdge> {
    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    let mut statement = connection
        .prepare("SELECT materialized_json FROM edges ORDER BY edge_id")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect()
}

fn visible_edges(fixture: &Fixture) -> (Vec<MaterializedEdge>, IndexReceipt) {
    let response = call(
        fixture.root.path(),
        ScoutStoreRequest::Edges {
            enterprise_id: fixture.enterprise.clone(),
            query: EdgeQuery {
                limit: 100,
                ..EdgeQuery::default()
            },
        },
    )
    .unwrap();
    let ScoutStoreResponse::Edges { page, receipt } = response else {
        panic!("wrong edge query response");
    };
    (page.edges, receipt)
}

fn force_cold(fixture: &Fixture) -> (IndexedStatus, IndexReceipt) {
    let connection = Connection::open(fixture.root.path().join("index-v4.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE meta SET value = 'force-cold-rebuild' WHERE key = 'projection_version'",
            [],
        )
        .unwrap();
    drop(connection);
    let response = call(
        fixture.root.path(),
        ScoutStoreRequest::Rebuild {
            enterprise_id: fixture.enterprise.clone(),
        },
    )
    .unwrap();
    let ScoutStoreResponse::Rebuilt(receipt) = response else {
        panic!("wrong forced rebuild response");
    };
    let (status, _) = status(fixture);
    (status, receipt)
}

fn assert_roots_equal(hot: &IndexReceipt, cold: &IndexReceipt) {
    assert_eq!(hot.event_root, cold.event_root);
    assert_eq!(hot.graph_digest, cold.graph_digest);
    assert_eq!(hot.event_set_root_v1, cold.event_set_root_v1);
    assert_eq!(hot.projection_map_root_v2, cold.projection_map_root_v2);
    assert_eq!(
        hot.enterprise_snapshot_root_v2,
        cold.enterprise_snapshot_root_v2
    );
}

fn assert_supplemental_roots_hidden(receipt: &IndexReceipt) {
    assert!(receipt.event_set_root_v1.is_none());
    assert!(receipt.projection_map_root_v2.is_none());
    assert!(receipt.enterprise_snapshot_root_v2.is_none());
    let json = serde_json::to_value(receipt).unwrap();
    assert!(json.get("event_set_root_v1").is_none());
    assert!(json.get("projection_map_root_v1").is_none());
    assert!(json.get("enterprise_snapshot_root_v1").is_none());
    assert!(json.get("projection_map_root_v2").is_none());
    assert!(json.get("enterprise_snapshot_root_v2").is_none());
}
