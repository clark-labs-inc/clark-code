use std::collections::BTreeSet;

use agent_orchestration::{
    CoverageKey, DiscoveryCharterObservation, EnterpriseConflict, EnterpriseEdgeKind,
    EnterpriseEntityKind, EnterpriseFact, EnterpriseGraph, GraphEdgeObservation,
};

use super::{status, Fixture};
use crate::ScoutStoreResponse;

mod support;
use support::*;

#[test]
fn ordinary_entity_append_is_bounded_and_matches_forced_cold() {
    let fixture = Fixture::new();
    let entities = (0..6)
        .map(|index| {
            entity(
                &fixture,
                EnterpriseEntityKind::Service,
                &format!("service:seed-{index}"),
            )
        })
        .collect::<Vec<_>>();
    let mut facts = entities
        .iter()
        .cloned()
        .map(EnterpriseFact::EntityObserved)
        .collect::<Vec<_>>();
    for (from, to) in [(0, 1), (2, 3), (4, 5)] {
        facts.push(EnterpriseFact::EdgeObserved(
            GraphEdgeObservation::new(
                &fixture.enterprise,
                entities[from].entity_id.clone(),
                entities[to].entity_id.clone(),
                EnterpriseEdgeKind::Calls,
                None,
                evidence('e'),
            )
            .unwrap(),
        ));
    }
    fixture
        .ingest(signed_facts(&fixture, "seed", 1, 1, facts))
        .unwrap();

    let appended = entity(
        &fixture,
        EnterpriseEntityKind::Service,
        "service:ordinary-append",
    );
    let response = fixture
        .ingest(signed_facts(
            &fixture,
            "ordinary",
            1,
            1,
            vec![EnterpriseFact::EntityObserved(appended)],
        ))
        .unwrap();
    let ScoutStoreResponse::Ingested {
        receipt: hot_receipt,
        ..
    } = response
    else {
        panic!("wrong ordinary append response");
    };
    assert!(!hot_receipt.rebuilt, "ordinary append missed the hot path");

    let (hot_status, _) = status(&fixture);
    let hot_entities = entity_page(&fixture);
    let hot_edges = edge_page(&fixture);
    let (cold_status, cold_receipt) = force_cold(&fixture);
    let cold_entities = entity_page(&fixture);
    let cold_edges = edge_page(&fixture);

    assert_eq!(hot_status, cold_status);
    assert_eq!(hot_entities, cold_entities);
    assert_eq!(hot_edges, cold_edges);
    assert_roots_equal(&hot_receipt, &cold_receipt);
    assert_eq!(hot_receipt.event_ids_scanned, 0);
    assert_eq!(hot_receipt.history_rows_read, 0);
    let bounded_rows = hot_receipt
        .affected_projection_rows
        .saturating_add(hot_receipt.incident_edges_reclassified);
    assert!(
        hot_receipt.entity_rows_read <= bounded_rows,
        "entity reads {} exceeded affected-plus-incident bound {bounded_rows}",
        hot_receipt.entity_rows_read
    );
    assert!(
        hot_receipt.edge_rows_read <= bounded_rows,
        "edge reads {} exceeded affected-plus-incident bound {bounded_rows}",
        hot_receipt.edge_rows_read
    );
}

#[test]
fn dangling_conflicts_converge_as_missing_endpoints_arrive() {
    let fixture = Fixture::new();
    let from = entity(
        &fixture,
        EnterpriseEntityKind::Service,
        "service:missing-from",
    );
    let to = entity(
        &fixture,
        EnterpriseEntityKind::Service,
        "service:missing-to",
    );
    let edge = GraphEdgeObservation::new(
        &fixture.enterprise,
        from.entity_id.clone(),
        to.entity_id.clone(),
        EnterpriseEdgeKind::Calls,
        None,
        evidence('d'),
    )
    .unwrap();
    let both_missing = BTreeSet::from([
        EnterpriseConflict::DanglingEdge {
            edge_id: edge.edge_id.clone(),
            missing_entity_id: from.entity_id.clone(),
        },
        EnterpriseConflict::DanglingEdge {
            edge_id: edge.edge_id.clone(),
            missing_entity_id: to.entity_id.clone(),
        },
    ]);
    fixture
        .ingest(signed_facts(
            &fixture,
            "edge",
            1,
            1,
            vec![EnterpriseFact::EdgeObserved(edge.clone())],
        ))
        .unwrap();
    assert_dangling_hot_equals_cold(&fixture, &both_missing);

    let response = fixture
        .ingest(signed_facts(
            &fixture,
            "from",
            1,
            1,
            vec![EnterpriseFact::EntityObserved(from)],
        ))
        .unwrap();
    assert_hot(response);
    let one_missing = BTreeSet::from([EnterpriseConflict::DanglingEdge {
        edge_id: edge.edge_id.clone(),
        missing_entity_id: to.entity_id.clone(),
    }]);
    assert_dangling_hot_equals_cold(&fixture, &one_missing);

    let response = fixture
        .ingest(signed_facts(
            &fixture,
            "to",
            1,
            1,
            vec![EnterpriseFact::EntityObserved(to)],
        ))
        .unwrap();
    assert_hot(response);
    assert_dangling_hot_equals_cold(&fixture, &BTreeSet::new());
}

#[test]
fn post_seal_topology_changes_event_identity_but_not_projection_root() {
    let fixture = Fixture::new();
    let runtime = entity(
        &fixture,
        EnterpriseEntityKind::Service,
        "service:critical-runtime",
    );
    let journey = entity(&fixture, EnterpriseEntityKind::Journey, "journey:critical");
    let coverage_key = CoverageKey::new(
        "fixture",
        "fixture-auth",
        "tenant:fixture",
        "global",
        "service",
    )
    .unwrap();
    let members = BTreeSet::from([runtime.entity_id.clone(), journey.entity_id.clone()]);
    let charter_id = "charter:00000000-0000-4000-8000-000000000091";
    let charter = DiscoveryCharterObservation {
        charter_id: charter_id.into(),
        revision: 1,
        max_age_ms: 86_400_000,
        supersedes: None,
        required_coverage: BTreeSet::from([coverage_key.clone()]),
        critical_journey_ids: BTreeSet::from([journey.entity_id.clone()]),
        critical_runtime_ids: BTreeSet::from([runtime.entity_id.clone()]),
        evidence_digests: evidence('b'),
    };
    let mut initial_facts = vec![
        EnterpriseFact::EntityObserved(runtime),
        EnterpriseFact::EntityObserved(journey),
        EnterpriseFact::DiscoveryCharterObserved(charter),
    ];
    initial_facts.extend(coverage_pass(&fixture, &coverage_key, &members, '1'));
    let initial = signed_facts(&fixture, "discovery", 1, 1, initial_facts);
    let mut graph =
        EnterpriseGraph::from_batches(fixture.enterprise.clone(), [initial.batch.clone()]).unwrap();
    fixture.ingest(initial).unwrap();
    let seal_one = graph
        .draft_discovery_pass_seal(charter_id, "epoch-1", 1, None, evidence('c'))
        .unwrap();
    let seal_one_id = seal_one.pass_id.clone();
    let seal_one_batch = signed_facts(
        &fixture,
        "coordinator",
        1,
        1,
        vec![EnterpriseFact::DiscoveryPassSealed(seal_one)],
    );
    graph.apply_batch(seal_one_batch.batch.clone()).unwrap();
    fixture.ingest(seal_one_batch).unwrap();

    let epoch_two = signed_facts(
        &fixture,
        "discovery",
        2,
        1,
        coverage_pass(&fixture, &coverage_key, &members, '2'),
    );
    graph.apply_batch(epoch_two.batch.clone()).unwrap();
    fixture.ingest(epoch_two).unwrap();
    let seal_two = graph
        .draft_discovery_pass_seal(charter_id, "epoch-2", 2, Some(seal_one_id), evidence('d'))
        .unwrap();
    let response = fixture
        .ingest(signed_facts(
            &fixture,
            "coordinator",
            2,
            1,
            vec![EnterpriseFact::DiscoveryPassSealed(seal_two)],
        ))
        .unwrap();
    let ScoutStoreResponse::Ingested {
        receipt: sealed_receipt,
        ..
    } = response
    else {
        panic!("wrong seal response");
    };
    assert!(status(&fixture).0.current_pass_id.is_some());
    let sealed_entities = entity_page(&fixture);

    let ignored = entity(
        &fixture,
        EnterpriseEntityKind::Service,
        "service:post-seal-ignored",
    );
    let response = fixture
        .ingest(signed_facts(
            &fixture,
            "late-topology",
            3,
            1,
            vec![EnterpriseFact::EntityObserved(ignored)],
        ))
        .unwrap();
    let ScoutStoreResponse::Ingested {
        receipt: post_receipt,
        ..
    } = response
    else {
        panic!("wrong post-seal response");
    };
    assert!(
        !post_receipt.rebuilt,
        "newer-epoch topology missed hot path"
    );
    assert_ne!(
        post_receipt.event_set_root_v1,
        sealed_receipt.event_set_root_v1
    );
    assert_ne!(post_receipt.event_root, sealed_receipt.event_root);
    assert_ne!(post_receipt.graph_digest, sealed_receipt.graph_digest);
    assert_eq!(
        post_receipt.projection_map_root_v2,
        sealed_receipt.projection_map_root_v2
    );
    assert_eq!(post_receipt.affected_projection_rows, 0);
    assert_eq!(entity_page(&fixture), sealed_entities);
}
