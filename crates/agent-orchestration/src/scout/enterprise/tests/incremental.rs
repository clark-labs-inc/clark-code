use std::collections::{BTreeMap, BTreeSet};

use super::*;

#[derive(Default)]
struct ProjectedRows {
    entities: BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    edges: BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    coverage: BTreeMap<CoverageCellId, MaterializedCoverage>,
    frontier: BTreeMap<FrontierTaskId, MaterializedFrontier>,
    simulation: BTreeMap<EnterpriseEntityId, MaterializedSimulationContract>,
}

impl ProjectedRows {
    fn from_snapshot(snapshot: &EnterpriseSnapshot) -> Self {
        Self {
            entities: snapshot.entities.clone(),
            edges: snapshot.edges.clone(),
            coverage: snapshot.coverage.clone(),
            frontier: snapshot.frontier.clone(),
            simulation: snapshot.simulation_contracts.clone(),
        }
    }

    fn apply(&mut self, update: EnterpriseAffectedProjection) {
        if let Some(snapshot) = update.replacement_snapshot {
            *self = Self::from_snapshot(&snapshot);
            return;
        }
        apply_rows(&mut self.entities, update.entities);
        apply_rows(&mut self.edges, update.edges);
        apply_rows(&mut self.coverage, update.coverage);
        apply_rows(&mut self.frontier, update.frontier);
        apply_rows(&mut self.simulation, update.simulation_contracts);
    }

    fn assert_matches(&self, snapshot: &EnterpriseSnapshot) {
        assert_eq!(self.entities, snapshot.entities);
        assert_eq!(self.edges, snapshot.edges);
        assert_eq!(self.coverage, snapshot.coverage);
        assert_eq!(self.frontier, snapshot.frontier);
        assert_eq!(self.simulation, snapshot.simulation_contracts);
    }
}

fn apply_rows<K: Ord, V>(rows: &mut BTreeMap<K, V>, changes: BTreeMap<K, Option<V>>) {
    for (key, value) in changes {
        if let Some(value) = value {
            rows.insert(key, value);
        } else {
            rows.remove(&key);
        }
    }
}

#[test]
fn one_batch_rebuilds_only_its_six_affected_rows() {
    let enterprise = enterprise();
    let (first, ids) = topology_batch(&enterprise, "machine-a", "initial", 'a');
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    graph.apply_batch(first).unwrap();
    let baseline = graph.snapshot().unwrap();
    let mut projected = ProjectedRows::from_snapshot(&baseline);
    let mut cursor = graph.projection_cursor().unwrap();

    let (second, second_ids) = topology_batch(&enterprise, "machine-b", "updated", 'b');
    assert_eq!(ids, second_ids);
    let update = graph.apply_batch_affected(&mut cursor, second).unwrap();

    assert!(!update.requires_full_rebuild());
    assert!(update.global_metadata_changed);
    assert_eq!(update.affected_row_count(), 6);
    assert_eq!(update.entities.len(), 2);
    assert_eq!(update.edges.len(), 1);
    assert_eq!(update.coverage.len(), 1);
    assert_eq!(update.frontier.len(), 1);
    assert_eq!(update.simulation_contracts.len(), 1);
    assert_eq!(update.work.inserted_events, 6);
    assert_eq!(update.work.candidate_events_examined, 12);
    assert_eq!(update.work.records_rebuilt, 6);

    projected.apply(update);
    let snapshot = graph.snapshot().unwrap();
    projected.assert_matches(&snapshot);
    assert!(snapshot.entities[&ids.0].labels.contains("updated"));
}

#[test]
fn affected_projection_converges_across_batch_order_and_duplicate_delivery() {
    let enterprise = enterprise();
    let (left_batch, _) = topology_batch(&enterprise, "machine-a", "left", 'a');
    let (right_batch, _) = topology_batch(&enterprise, "machine-b", "right", 'b');

    let (left_rows, left_snapshot) = projected_replay(
        enterprise.clone(),
        [left_batch.clone(), right_batch.clone(), left_batch.clone()],
    );
    let (right_rows, right_snapshot) =
        projected_replay(enterprise, [right_batch, left_batch.clone(), left_batch]);

    left_rows.assert_matches(&left_snapshot);
    right_rows.assert_matches(&right_snapshot);
    assert_eq!(left_snapshot, right_snapshot);
}

#[test]
fn stale_cursor_is_rejected_before_the_next_batch_is_inserted() {
    let enterprise = enterprise();
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    let mut cursor = graph.projection_cursor().unwrap();
    let first = EnterpriseBatch::new(
        enterprise.clone(),
        [entity("machine-a", 1, "service:first", "first")],
    )
    .unwrap();
    graph.apply_batch(first).unwrap();
    let before = graph.event_count();
    let second = EnterpriseBatch::new(
        enterprise,
        [entity("machine-b", 1, "service:second", "second")],
    )
    .unwrap();

    assert_eq!(
        graph.apply_batch_affected(&mut cursor, second).unwrap_err(),
        "enterprise projection cursor is stale"
    );
    assert_eq!(graph.event_count(), before);
}

#[test]
fn duplicate_only_subset_batch_advances_cursor_without_rebuilding_rows() {
    let enterprise = enterprise();
    let first = entity("machine-a", 1, "service:first", "first");
    let second = entity("machine-a", 2, "service:second", "second");
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    let mut cursor = graph.projection_cursor().unwrap();
    graph
        .apply_batch_affected(
            &mut cursor,
            EnterpriseBatch::new(enterprise.clone(), [first.clone(), second]).unwrap(),
        )
        .unwrap();

    let subset = graph
        .apply_batch_affected(
            &mut cursor,
            EnterpriseBatch::new(enterprise.clone(), [first]).unwrap(),
        )
        .unwrap();
    assert_eq!(subset.merge.inserted, 0);
    assert!(!subset.global_metadata_changed);
    assert_eq!(subset.affected_row_count(), 0);
    assert_eq!(cursor.batch_count(), 2);

    let third = EnterpriseBatch::new(
        enterprise,
        [entity("machine-b", 1, "service:third", "third")],
    )
    .unwrap();
    assert!(graph.apply_batch_affected(&mut cursor, third).is_ok());
}

#[test]
fn correction_retraction_uses_exact_full_rebuild_and_removes_the_target_projection_row() {
    let enterprise = enterprise();
    let observed = entity("machine-a", 1, "service:retired", "retired");
    let target_id = observed.event_id.clone();
    let entity_id = match &observed.fact {
        EnterpriseFact::EntityObserved(value) => value.entity_id.clone(),
        _ => unreachable!(),
    };
    let mut graph = EnterpriseGraph::new(enterprise.clone());
    graph
        .apply_batch(EnterpriseBatch::new(enterprise.clone(), [observed]).unwrap())
        .unwrap();
    let mut cursor = graph.projection_cursor().unwrap();
    let retraction = EnterpriseEvent::new(
        enterprise.clone(),
        provenance("coordinator", 2, 1),
        EnterpriseFact::ObservationRetracted {
            target_event_id: target_id,
            reason: "resource was authoritatively removed".into(),
            evidence_digests: evidence('c'),
        },
    )
    .unwrap();

    let update = graph
        .apply_batch_affected(
            &mut cursor,
            EnterpriseBatch::new(enterprise, [retraction]).unwrap(),
        )
        .unwrap();

    assert!(update.requires_full_rebuild());
    assert!(update.work.full_rebuild);
    assert!(!update
        .replacement_snapshot
        .as_ref()
        .unwrap()
        .entities
        .contains_key(&entity_id));
}

#[test]
fn sealed_graph_defers_new_epoch_topology_until_the_next_control_transition() {
    let enterprise = enterprise();
    let mut graph = EnterpriseGraph::from_batches(
        enterprise.clone(),
        super::scale::enterprise_batches(&enterprise),
    )
    .unwrap();
    assert!(graph.snapshot().unwrap().fixed_point);
    let mut cursor = graph.projection_cursor().unwrap();
    assert!(cursor.current_pass_id().is_some());

    let observation = GraphEntityObservation::new(
        &enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:new", "service:new").unwrap(),
        BTreeSet::from(["new-unsealed-service".into()]),
        evidence('d'),
    )
    .unwrap();
    let entity_id = observation.entity_id.clone();
    let event = EnterpriseEvent::new(
        enterprise.clone(),
        provenance("machine-new", 3, 1),
        EnterpriseFact::EntityObserved(observation),
    )
    .unwrap();
    let update = graph
        .apply_batch_affected(
            &mut cursor,
            EnterpriseBatch::new(enterprise.clone(), [event]).unwrap(),
        )
        .unwrap();

    assert!(!update.requires_full_rebuild());
    assert!(update.global_metadata_changed);
    assert_eq!(update.affected_row_count(), 0);
    assert!(!graph.snapshot().unwrap().entities.contains_key(&entity_id));

    let older = entity("machine-late", 1, "service:late-old-epoch", "late");
    let fallback = graph
        .apply_batch_affected(
            &mut cursor,
            EnterpriseBatch::new(enterprise, [older]).unwrap(),
        )
        .unwrap();
    assert!(fallback.requires_full_rebuild());
}

fn projected_replay(
    enterprise: EnterpriseId,
    batches: impl IntoIterator<Item = EnterpriseBatch>,
) -> (ProjectedRows, EnterpriseSnapshot) {
    let mut graph = EnterpriseGraph::new(enterprise);
    let mut cursor = graph.projection_cursor().unwrap();
    let mut rows = ProjectedRows::default();
    for batch in batches {
        rows.apply(graph.apply_batch_affected(&mut cursor, batch).unwrap());
    }
    let snapshot = graph.snapshot().unwrap();
    (rows, snapshot)
}

fn topology_batch(
    enterprise: &EnterpriseId,
    machine: &str,
    label: &str,
    evidence_byte: char,
) -> (EnterpriseBatch, (EnterpriseEntityId, EnterpriseEntityId)) {
    let service = GraphEntityObservation::new(
        enterprise,
        EnterpriseEntityKind::Service,
        AuthorityRef::new("aws", "account:prod", "service:checkout").unwrap(),
        BTreeSet::from([label.into()]),
        evidence(evidence_byte),
    )
    .unwrap();
    let repository = GraphEntityObservation::new(
        enterprise,
        EnterpriseEntityKind::Repository,
        AuthorityRef::new("github", "global", "repo:42").unwrap(),
        BTreeSet::from(["checkout-source".into()]),
        evidence(evidence_byte),
    )
    .unwrap();
    let service_id = service.entity_id.clone();
    let repository_id = repository.entity_id.clone();
    let edge = GraphEdgeObservation::new(
        enterprise,
        repository_id.clone(),
        service_id.clone(),
        EnterpriseEdgeKind::SourceFor,
        None,
        evidence(evidence_byte),
    )
    .unwrap();
    let coverage_key = CoverageKey::new(
        "aws",
        "auth-read-only",
        "account:prod",
        "us-east-1",
        "service",
    )
    .unwrap();
    let coverage = CoverageObservation::new(
        enterprise,
        coverage_key.clone(),
        CoverageStatus::Supported,
        None,
        1,
        evidence(evidence_byte),
    )
    .unwrap();
    let frontier = FrontierObservation::new(
        enterprise,
        FrontierKey::new(coverage_key, None).unwrap(),
        FrontierState::Terminal {
            status: CoverageStatus::Supported,
            reason: "enumeration complete".into(),
        },
        evidence(evidence_byte),
    )
    .unwrap();
    let simulation = SimulationContractObservation {
        runtime_id: service_id.clone(),
        inputs: true,
        outputs: true,
        state_effects: true,
        timeouts: true,
        retries: true,
        idempotency: true,
        failure_behavior: true,
        observability: true,
        recovery: true,
        evidence_digests: evidence(evidence_byte),
    };
    let facts = [
        EnterpriseFact::EntityObserved(service),
        EnterpriseFact::EntityObserved(repository),
        EnterpriseFact::EdgeObserved(edge),
        EnterpriseFact::CoverageObserved(coverage),
        EnterpriseFact::FrontierObserved(frontier),
        EnterpriseFact::SimulationContractObserved(simulation),
    ];
    let events = facts
        .into_iter()
        .enumerate()
        .map(|(index, fact)| {
            EnterpriseEvent::new(
                enterprise.clone(),
                provenance(machine, 1, index as u64 + 1),
                fact,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    (
        EnterpriseBatch::new(enterprise.clone(), events).unwrap(),
        (service_id, repository_id),
    )
}
