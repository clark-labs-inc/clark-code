use std::collections::BTreeSet;

use super::*;

mod contracts;
mod incremental;
mod scale;

fn enterprise() -> EnterpriseId {
    EnterpriseId::new("acme").unwrap()
}

fn evidence(byte: char) -> BTreeSet<String> {
    BTreeSet::from([byte.to_string().repeat(64)])
}

fn provenance(machine: &str, epoch: u64, sequence: u64) -> EnterpriseProvenance {
    EnterpriseProvenance {
        machine_id: machine.into(),
        run_id: format!("run-{machine}-{epoch}"),
        adapter_instance_id: format!("adapter-{machine}"),
        auth_context_id: "auth-read-only".into(),
        discovery_epoch: format!("epoch-{epoch}"),
        discovery_epoch_sequence: epoch,
        source_sequence: sequence,
        observed_at_ms: epoch * 1_000_000 + sequence,
        source_fingerprint: "f".repeat(64),
    }
}

fn entity(machine: &str, sequence: u64, native_id: &str, label: &str) -> EnterpriseEvent {
    let enterprise = enterprise();
    let observation = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:prod", native_id).unwrap(),
        BTreeSet::from([label.into()]),
        evidence('a'),
    )
    .unwrap();
    EnterpriseEvent::new(
        enterprise,
        provenance(machine, 1, sequence),
        EnterpriseFact::EntityObserved(observation),
    )
    .unwrap()
}

#[test]
fn provider_native_identity_is_stable_and_namespaced() {
    let enterprise = enterprise();
    let left = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:a", "service:checkout").unwrap(),
        BTreeSet::from(["checkout".into()]),
        evidence('a'),
    )
    .unwrap();
    let replay = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:a", "service:checkout").unwrap(),
        BTreeSet::from(["renamed-checkout".into()]),
        evidence('b'),
    )
    .unwrap();
    let other_account = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:b", "service:checkout").unwrap(),
        BTreeSet::from(["checkout".into()]),
        evidence('c'),
    )
    .unwrap();

    assert_eq!(left.entity_id, replay.entity_id);
    assert_ne!(left.entity_id, other_account.entity_id);
}

#[test]
fn batch_union_is_commutative_associative_and_idempotent() {
    let enterprise = enterprise();
    let event_a = entity("mac-a", 1, "service:checkout", "checkout");
    let event_b = entity("mac-b", 1, "service:checkout", "payments-entry");
    let event_c = entity("mac-c", 1, "service:fulfillment", "fulfillment");
    let batch_a = EnterpriseBatch::new(enterprise.clone(), [event_a]).unwrap();
    let batch_b = EnterpriseBatch::new(enterprise.clone(), [event_b, event_c]).unwrap();

    let mut left = EnterpriseGraph::new(enterprise.clone());
    left.apply_batch(batch_a.clone()).unwrap();
    left.apply_batch(batch_b.clone()).unwrap();
    let duplicate = left.apply_batch(batch_a.clone()).unwrap();
    assert_eq!(duplicate.inserted, 0);

    let mut right = EnterpriseGraph::new(enterprise);
    right.apply_batch(batch_b).unwrap();
    right.apply_batch(batch_a).unwrap();

    let left_snapshot = left.snapshot().unwrap();
    let right_snapshot = right.snapshot().unwrap();
    assert_eq!(left_snapshot.graph_digest, right_snapshot.graph_digest);
    assert_eq!(left_snapshot, right_snapshot);
    assert_eq!(left_snapshot.entities.len(), 2);
    let checkout = left_snapshot
        .entities
        .values()
        .find(|entity| entity.authority.native_id == "service:checkout")
        .unwrap();
    assert_eq!(checkout.supporting_events.len(), 2);
    assert!(checkout.labels.contains("checkout"));
    assert!(checkout.labels.contains("payments-entry"));
}

#[test]
fn actor_sequence_equivocation_is_preserved_as_a_conflict() {
    let enterprise = enterprise();
    let first = entity("mac-a", 1, "service:a", "a");
    let second = entity("mac-a", 1, "service:b", "b");
    let batch = EnterpriseBatch::new(enterprise.clone(), [first, second]).unwrap();
    let mut graph = EnterpriseGraph::new(enterprise);
    graph.apply_batch(batch).unwrap();

    let snapshot = graph.snapshot().unwrap();
    assert!(snapshot
        .conflicts
        .iter()
        .any(|conflict| matches!(conflict, EnterpriseConflict::SourceEquivocation { .. })));
    assert!(!snapshot.completion().complete);
}

#[test]
fn equal_sequences_in_independent_auth_contexts_are_not_equivocation() {
    let enterprise = enterprise();
    let make = |auth_context: &str, authority_scope: &str, native_id: &str| {
        let observation = GraphEntityObservation::new(
            &enterprise,
            EnterpriseEntityKind::Service,
            AuthorityRef::new("aws", authority_scope, native_id).unwrap(),
            BTreeSet::from([native_id.into()]),
            evidence('a'),
        )
        .unwrap();
        let mut provenance = provenance("mac-a", 1, 1);
        provenance.run_id = "shared-run".into();
        provenance.adapter_instance_id = "shared-adapter".into();
        provenance.auth_context_id = auth_context.into();
        EnterpriseEvent::new(
            enterprise.clone(),
            provenance,
            EnterpriseFact::EntityObserved(observation),
        )
        .unwrap()
    };
    let batch = EnterpriseBatch::new(
        enterprise.clone(),
        [
            make("auth-a", "account:a", "service:a"),
            make("auth-b", "account:b", "service:b"),
        ],
    )
    .unwrap();
    let mut graph = EnterpriseGraph::new(enterprise);
    graph.apply_batch(batch).unwrap();

    assert!(!graph
        .snapshot()
        .unwrap()
        .conflicts
        .iter()
        .any(|conflict| { matches!(conflict, EnterpriseConflict::SourceEquivocation { .. }) }));
}

#[test]
fn newest_scan_epoch_wins_regardless_of_arrival_order() {
    let enterprise = enterprise();
    let key = CoverageKey::new("aws", "auth", "account:a", "us-east-1", "service").unwrap();
    let denied = CoverageObservation::new(
        &enterprise,
        key.clone(),
        CoverageStatus::Denied,
        None,
        0,
        evidence('a'),
    )
    .unwrap();
    let supported = CoverageObservation::new(
        &enterprise,
        key,
        CoverageStatus::Supported,
        None,
        12,
        evidence('b'),
    )
    .unwrap();
    let old_event = EnterpriseEvent::new(
        enterprise.clone(),
        provenance("mac-a", 1, 1),
        EnterpriseFact::CoverageObserved(denied),
    )
    .unwrap();
    let new_event = EnterpriseEvent::new(
        enterprise.clone(),
        provenance("mac-b", 2, 1),
        EnterpriseFact::CoverageObserved(supported),
    )
    .unwrap();
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    graph
        .apply_batch(EnterpriseBatch::new(enterprise, [new_event, old_event]).unwrap())
        .unwrap();

    let coverage = graph
        .snapshot()
        .unwrap()
        .coverage
        .into_values()
        .next()
        .unwrap();
    assert_eq!(coverage.status, Some(CoverageStatus::Supported));
    assert_eq!(coverage.enumerated_count, Some(12));
    assert!(!coverage.conflicted);
}

#[test]
fn newest_entity_and_edge_epoch_define_the_current_materialized_view() {
    let enterprise = enterprise();
    let authority = AuthorityRef::new("aws", "account:prod", "service:checkout").unwrap();
    let mut old_entity = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        authority.clone(),
        BTreeSet::from(["old-name".into()]),
        evidence('a'),
    )
    .unwrap();
    old_entity.environments.insert("staging".into());
    old_entity.critical = true;
    let mut new_entity = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        authority,
        BTreeSet::from(["checkout".into()]),
        evidence('b'),
    )
    .unwrap();
    new_entity.environments.insert("production".into());
    let service_id = new_entity.entity_id.clone();

    let repository = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Repository,
        AuthorityRef::new("github", "org:acme", "repo:checkout").unwrap(),
        BTreeSet::from(["checkout-repo".into()]),
        evidence('c'),
    )
    .unwrap();
    let repository_id = repository.entity_id.clone();
    let old_edge = GraphEdgeObservation::new(
        &enterprise,
        repository_id.clone(),
        service_id.clone(),
        EnterpriseEdgeKind::SourceFor,
        None,
        evidence('d'),
    )
    .unwrap();
    let new_edge = GraphEdgeObservation::new(
        &enterprise,
        repository_id,
        service_id.clone(),
        EnterpriseEdgeKind::SourceFor,
        None,
        evidence('e'),
    )
    .unwrap();
    let events = [
        EnterpriseEvent::new(
            enterprise.clone(),
            provenance("mac-a", 1, 1),
            EnterpriseFact::EntityObserved(old_entity),
        )
        .unwrap(),
        EnterpriseEvent::new(
            enterprise.clone(),
            provenance("mac-a", 2, 1),
            EnterpriseFact::EntityObserved(new_entity),
        )
        .unwrap(),
        EnterpriseEvent::new(
            enterprise.clone(),
            provenance("mac-a", 2, 2),
            EnterpriseFact::EntityObserved(repository),
        )
        .unwrap(),
        EnterpriseEvent::new(
            enterprise.clone(),
            provenance("mac-a", 1, 3),
            EnterpriseFact::EdgeObserved(old_edge),
        )
        .unwrap(),
        EnterpriseEvent::new(
            enterprise.clone(),
            provenance("mac-a", 2, 3),
            EnterpriseFact::EdgeObserved(new_edge),
        )
        .unwrap(),
    ];
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    graph
        .apply_batch(EnterpriseBatch::new(enterprise, events).unwrap())
        .unwrap();
    let snapshot = graph.snapshot().unwrap();
    let current = snapshot.entities.get(&service_id).unwrap();
    assert_eq!(current.discovery_epoch_sequence, 2);
    assert_eq!(current.labels, BTreeSet::from(["checkout".into()]));
    assert_eq!(current.environments, BTreeSet::from(["production".into()]));
    assert!(!current.critical);
    assert_eq!(current.evidence_digests, evidence('b'));
    let edge = snapshot.edges.values().next().unwrap();
    assert_eq!(edge.discovery_epoch_sequence, 2);
    assert_eq!(edge.evidence_digests, evidence('e'));
}

#[test]
fn explicit_retraction_removes_an_observation_without_erasing_history() {
    let enterprise = enterprise();
    let observed = entity("mac-a", 1, "service:a", "a");
    let retracted_id = observed.event_id.clone();
    let retraction = EnterpriseEvent::new(
        enterprise.clone(),
        provenance("mac-a", 1, 2),
        EnterpriseFact::ObservationRetracted {
            target_event_id: retracted_id,
            reason: "provider confirmed deletion in a complete scan".into(),
            evidence_digests: evidence('d'),
        },
    )
    .unwrap();
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    graph
        .apply_batch(EnterpriseBatch::new(enterprise, [observed, retraction]).unwrap())
        .unwrap();

    let snapshot = graph.snapshot().unwrap();
    assert!(snapshot.entities.is_empty());
    assert_eq!(snapshot.event_count, 2);
    assert_eq!(snapshot.retracted_event_count, 1);
}

#[test]
fn bounded_queries_and_neighborhoods_retrieve_graph_slices() {
    let enterprise = enterprise();
    let service_event = entity("mac-a", 1, "service:a", "checkout");
    let service_id = match &service_event.fact {
        EnterpriseFact::EntityObserved(entity) => entity.entity_id.clone(),
        _ => unreachable!(),
    };
    let database = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Database,
        AuthorityRef::new("aws", "account:prod", "db:orders").unwrap(),
        BTreeSet::from(["orders".into()]),
        evidence('b'),
    )
    .unwrap();
    let database_id = database.entity_id.clone();
    let database_event = EnterpriseEvent::new(
        enterprise.clone(),
        provenance("mac-a", 1, 2),
        EnterpriseFact::EntityObserved(database),
    )
    .unwrap();
    let edge = GraphEdgeObservation::new(
        &enterprise,
        service_id.clone(),
        database_id,
        EnterpriseEdgeKind::Writes,
        None,
        evidence('c'),
    )
    .unwrap();
    let edge_event = EnterpriseEvent::new(
        enterprise.clone(),
        provenance("mac-a", 1, 3),
        EnterpriseFact::EdgeObserved(edge),
    )
    .unwrap();
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    graph
        .apply_batch(
            EnterpriseBatch::new(enterprise, [service_event, database_event, edge_event]).unwrap(),
        )
        .unwrap();

    let first_page = graph
        .query_entities(&EnterpriseQuery {
            limit: 1,
            ..EnterpriseQuery::default()
        })
        .unwrap();
    assert_eq!(first_page.len(), 1);
    let second_page = graph
        .query_entities(&EnterpriseQuery {
            after_entity_id: Some(first_page[0].entity_id.clone()),
            limit: 1,
            ..EnterpriseQuery::default()
        })
        .unwrap();
    assert_eq!(second_page.len(), 1);
    assert_ne!(first_page[0].entity_id, second_page[0].entity_id);

    let result = graph
        .query_entities(&EnterpriseQuery {
            kind: Some(EnterpriseEntityKind::Service),
            label_contains: Some("check".into()),
            limit: 10,
            ..EnterpriseQuery::default()
        })
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(graph.neighborhood(&service_id, 1, 10).unwrap().len(), 2);
}
