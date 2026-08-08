use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::super::contract::{
    canonical_digest, CoverageCellId, EnterpriseEdgeId, EnterpriseEntityId, EnterpriseEvent,
    EnterpriseEventId, EnterpriseFact, FrontierTaskId,
};
use super::control;
use super::model::{
    EnterpriseConflict, EnterpriseSnapshot, MaterializedCoverage, MaterializedEdge,
    MaterializedEntity, MaterializedFrontier, MaterializedSimulationContract,
};
use super::EnterpriseGraph;

pub(super) mod observations;

pub(super) fn snapshot(graph: &EnterpriseGraph) -> Result<EnterpriseSnapshot, String> {
    let events = graph.raw_events();
    let mut conflicts = source_conflicts(events);
    let retracted = retracted_events(events, &mut conflicts);
    let active = events
        .values()
        .filter(|event| !retracted.contains(&event.event_id))
        .collect::<Vec<_>>();

    let control = control::materialize(graph.enterprise_id(), &active, &mut conflicts)?;
    let sealed_epoch = control
        .current_pass_id
        .as_ref()
        .and_then(|pass_id| control.passes.get(pass_id))
        .map(|pass| pass.discovery_epoch_sequence);
    let sealed_events = active
        .iter()
        .copied()
        .filter(|event| {
            sealed_epoch.is_none_or(|epoch| event.provenance.discovery_epoch_sequence <= epoch)
        })
        .collect::<Vec<_>>();
    let mut entities = materialize_entities(&sealed_events);
    let mut edges = materialize_edges(&sealed_events);
    let mut entity_history = BTreeMap::new();
    let mut edge_history = BTreeMap::new();
    if control.current_pass_id.is_some() {
        entities.retain(|entity_id, _| control.member_entity_ids.contains(entity_id));
        edges.retain(|edge_id, _| control.member_edge_ids.contains(edge_id));
    }
    apply_endpoint_classification(&entities, &mut edges);
    if !control.qualified_topologies.is_empty() {
        let temporal = super::temporal::project(&active, &control.qualified_topologies);
        entities = temporal.entities;
        edges = temporal.edges;
        entity_history = temporal.entity_history;
        edge_history = temporal.edge_history;
    }
    record_dangling_edges(&entities, &edges, &mut conflicts);
    let coverage = observations::coverage(&active, &mut conflicts);
    let frontier = observations::frontier(&active, &mut conflicts);
    let simulation_contracts = observations::simulation(&sealed_events, &mut conflicts);
    let event_root = graph.event_root()?;
    let graph_digest = snapshot_digest(SnapshotDigest {
        enterprise_id: graph.enterprise_id(),
        event_root: &event_root,
        entities: &entities,
        edges: &edges,
        entity_history: &entity_history,
        edge_history: &edge_history,
        coverage: &coverage,
        frontier: &frontier,
        simulation_contracts: &simulation_contracts,
        charter: &control.charter,
        discovery_passes: &control.passes,
        current_pass_id: &control.current_pass_id,
        fixed_point: control.fixed_point,
        control_blockers: &control.blockers,
        conflicts: &conflicts,
    })?;

    Ok(EnterpriseSnapshot {
        enterprise_id: graph.enterprise_id().clone(),
        event_root,
        graph_digest,
        event_count: events.len(),
        retracted_event_count: retracted.len(),
        entities,
        edges,
        entity_history,
        edge_history,
        coverage,
        frontier,
        simulation_contracts,
        charter: control.charter,
        discovery_passes: control.passes,
        current_pass_id: control.current_pass_id,
        fixed_point: control.fixed_point,
        control_blockers: control.blockers,
        conflicts,
    })
}

fn source_conflicts(
    events: &BTreeMap<EnterpriseEventId, EnterpriseEvent>,
) -> BTreeSet<EnterpriseConflict> {
    let mut positions = BTreeMap::<String, BTreeSet<EnterpriseEventId>>::new();
    for event in events.values() {
        positions
            .entry(event.provenance.source_position())
            .or_default()
            .insert(event.event_id.clone());
    }
    positions
        .into_iter()
        .filter_map(|(source_position, event_ids)| {
            (event_ids.len() > 1).then_some(EnterpriseConflict::SourceEquivocation {
                source_position,
                event_ids,
            })
        })
        .collect()
}

pub(super) fn retracted_events(
    events: &BTreeMap<EnterpriseEventId, EnterpriseEvent>,
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> BTreeSet<EnterpriseEventId> {
    let mut retracted = BTreeSet::new();
    for event in events.values() {
        let EnterpriseFact::ObservationRetracted {
            target_event_id, ..
        } = &event.fact
        else {
            continue;
        };
        let Some(target) = events.get(target_event_id) else {
            conflicts.insert(EnterpriseConflict::OrphanRetraction {
                retraction_event_id: event.event_id.clone(),
                target_event_id: target_event_id.clone(),
            });
            continue;
        };
        if matches!(target.fact, EnterpriseFact::ObservationRetracted { .. }) {
            conflicts.insert(EnterpriseConflict::RetractionOfRetraction {
                retraction_event_id: event.event_id.clone(),
                target_event_id: target_event_id.clone(),
            });
            continue;
        }
        retracted.insert(target_event_id.clone());
    }
    retracted
}

pub(super) fn materialize_entities(
    events: &[&EnterpriseEvent],
) -> BTreeMap<EnterpriseEntityId, MaterializedEntity> {
    let mut grouped = BTreeMap::<EnterpriseEntityId, Vec<&EnterpriseEvent>>::new();
    for event in events {
        let EnterpriseFact::EntityObserved(observation) = &event.fact else {
            continue;
        };
        grouped
            .entry(observation.entity_id.clone())
            .or_default()
            .push(event);
    }
    grouped
        .into_iter()
        .map(|(entity_id, observations)| {
            let epoch = observations
                .iter()
                .map(|event| event.provenance.discovery_epoch_sequence)
                .max()
                .unwrap_or_default();
            let classification = observations
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::EntityObserved(value) => Some(value.classification),
                    _ => None,
                })
                .max()
                .unwrap_or_default();
            let latest = observations
                .into_iter()
                .filter(|event| event.provenance.discovery_epoch_sequence == epoch)
                .collect::<Vec<_>>();
            let first = latest
                .first()
                .and_then(|event| match &event.fact {
                    EnterpriseFact::EntityObserved(value) => Some(value),
                    _ => None,
                })
                .expect("entity group contains entity observations");
            let mut record = MaterializedEntity {
                entity_id: entity_id.clone(),
                kind: first.kind,
                authority: first.authority.clone(),
                labels: BTreeSet::new(),
                environments: BTreeSet::new(),
                critical: false,
                classification,
                discovery_epoch_sequence: epoch,
                evidence_digests: BTreeSet::new(),
                supporting_events: BTreeSet::new(),
                last_observed_at_ms: 0,
                valid_from_ms: None,
                valid_to_ms: None,
                qualified_pass_id: None,
                lifecycle: super::model::QualifiedLifecycle::Active,
            };
            for event in latest {
                let EnterpriseFact::EntityObserved(observation) = &event.fact else {
                    continue;
                };
                record.labels.extend(observation.labels.iter().cloned());
                record
                    .environments
                    .extend(observation.environments.iter().cloned());
                record.critical |= observation.critical;
                record
                    .evidence_digests
                    .extend(observation.evidence_digests.iter().cloned());
                record.supporting_events.insert(event.event_id.clone());
                record.last_observed_at_ms = record
                    .last_observed_at_ms
                    .max(event.provenance.observed_at_ms);
            }
            (entity_id, record)
        })
        .collect()
}

pub(super) fn materialize_edges(
    events: &[&EnterpriseEvent],
) -> BTreeMap<EnterpriseEdgeId, MaterializedEdge> {
    let mut grouped = BTreeMap::<EnterpriseEdgeId, Vec<&EnterpriseEvent>>::new();
    for event in events {
        let EnterpriseFact::EdgeObserved(observation) = &event.fact else {
            continue;
        };
        grouped
            .entry(observation.edge_id.clone())
            .or_default()
            .push(event);
    }
    grouped
        .into_iter()
        .map(|(edge_id, observations)| {
            let epoch = observations
                .iter()
                .map(|event| event.provenance.discovery_epoch_sequence)
                .max()
                .unwrap_or_default();
            let classification = observations
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::EdgeObserved(value) => Some(value.classification),
                    _ => None,
                })
                .max()
                .unwrap_or_default();
            let latest = observations
                .into_iter()
                .filter(|event| event.provenance.discovery_epoch_sequence == epoch)
                .collect::<Vec<_>>();
            let first = latest
                .first()
                .and_then(|event| match &event.fact {
                    EnterpriseFact::EdgeObserved(value) => Some(value),
                    _ => None,
                })
                .expect("edge group contains edge observations");
            let mut record = MaterializedEdge {
                edge_id: first.edge_id.clone(),
                from: first.from.clone(),
                to: first.to.clone(),
                kind: first.kind,
                qualifier: first.qualifier.clone(),
                classification,
                discovery_epoch_sequence: epoch,
                evidence_digests: BTreeSet::new(),
                supporting_events: BTreeSet::new(),
                last_observed_at_ms: 0,
                valid_from_ms: None,
                valid_to_ms: None,
                qualified_pass_id: None,
                lifecycle: super::model::QualifiedLifecycle::Active,
            };
            for event in latest {
                let EnterpriseFact::EdgeObserved(observation) = &event.fact else {
                    continue;
                };
                record
                    .evidence_digests
                    .extend(observation.evidence_digests.iter().cloned());
                record.supporting_events.insert(event.event_id.clone());
                record.last_observed_at_ms = record
                    .last_observed_at_ms
                    .max(event.provenance.observed_at_ms);
            }
            (edge_id, record)
        })
        .collect()
}

pub(super) fn apply_endpoint_classification(
    entities: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    edges: &mut BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
) {
    for edge in edges.values_mut() {
        for endpoint in [&edge.from, &edge.to] {
            if let Some(entity) = entities.get(endpoint) {
                edge.classification = edge.classification.join(entity.classification);
            }
        }
    }
}

fn record_dangling_edges(
    entities: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    edges: &BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) {
    for edge in edges.values() {
        for entity_id in [&edge.from, &edge.to] {
            if !entities.contains_key(entity_id) {
                conflicts.insert(EnterpriseConflict::DanglingEdge {
                    edge_id: edge.edge_id.clone(),
                    missing_entity_id: entity_id.clone(),
                });
            }
        }
    }
}

#[derive(Serialize)]
struct SnapshotDigest<'a> {
    enterprise_id: &'a super::super::contract::EnterpriseId,
    event_root: &'a str,
    entities: &'a BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    edges: &'a BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    entity_history: &'a BTreeMap<EnterpriseEntityId, Vec<MaterializedEntity>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    edge_history: &'a BTreeMap<EnterpriseEdgeId, Vec<MaterializedEdge>>,
    coverage: &'a BTreeMap<CoverageCellId, MaterializedCoverage>,
    frontier: &'a BTreeMap<FrontierTaskId, MaterializedFrontier>,
    simulation_contracts: &'a BTreeMap<EnterpriseEntityId, MaterializedSimulationContract>,
    charter: &'a Option<super::model::MaterializedCharter>,
    discovery_passes: &'a BTreeMap<String, super::model::MaterializedDiscoveryPass>,
    current_pass_id: &'a Option<String>,
    fixed_point: bool,
    control_blockers: &'a Vec<String>,
    conflicts: &'a BTreeSet<EnterpriseConflict>,
}

fn snapshot_digest(content: SnapshotDigest<'_>) -> Result<String, String> {
    canonical_digest(&content)
}

pub(super) fn snapshot_digest_from_snapshot(
    snapshot: &EnterpriseSnapshot,
) -> Result<String, String> {
    snapshot_digest(SnapshotDigest {
        enterprise_id: &snapshot.enterprise_id,
        event_root: &snapshot.event_root,
        entities: &snapshot.entities,
        edges: &snapshot.edges,
        entity_history: &snapshot.entity_history,
        edge_history: &snapshot.edge_history,
        coverage: &snapshot.coverage,
        frontier: &snapshot.frontier,
        simulation_contracts: &snapshot.simulation_contracts,
        charter: &snapshot.charter,
        discovery_passes: &snapshot.discovery_passes,
        current_pass_id: &snapshot.current_pass_id,
        fixed_point: snapshot.fixed_point,
        control_blockers: &snapshot.control_blockers,
        conflicts: &snapshot.conflicts,
    })
}
