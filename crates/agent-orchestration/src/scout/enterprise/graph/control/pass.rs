use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::super::super::contract::{
    canonical_digest, CoverageStatus, DiscoveryPassSealObservation, EnterpriseEdgeId,
    EnterpriseEntityId, EnterpriseEvent, EnterpriseFact, EnterpriseId, FrontierState,
};
use super::super::materialize::{
    apply_endpoint_classification, materialize_edges, materialize_entities,
};
use super::super::model::{MaterializedCharter, MaterializedEdge, MaterializedEntity};

pub(super) struct PassProjection {
    pub requirement_root: String,
    pub scope_root: String,
    pub topology_root: String,
    pub member_entity_ids: BTreeSet<EnterpriseEntityId>,
    pub member_edge_ids: BTreeSet<EnterpriseEdgeId>,
    pub entity_scopes:
        BTreeMap<EnterpriseEntityId, BTreeSet<super::super::super::contract::CoverageCellId>>,
    pub edge_scopes:
        BTreeMap<EnterpriseEdgeId, BTreeSet<super::super::super::contract::CoverageCellId>>,
    pub blockers: Vec<String>,
}

pub(super) fn project_pass(
    enterprise_id: &EnterpriseId,
    events: &[&EnterpriseEvent],
    charter: &MaterializedCharter,
    seal: &DiscoveryPassSealObservation,
) -> Result<PassProjection, String> {
    let requirement_root = canonical_digest(&(
        "scout-enterprise-requirement-root-v1",
        enterprise_id,
        &charter.charter_id,
        &charter.required_coverage,
    ))?;
    let mut blockers = Vec::new();
    let mut scopes = Vec::new();
    let mut member_entity_ids = BTreeSet::new();
    let mut member_edge_ids = BTreeSet::new();
    let mut entity_scopes = BTreeMap::<EnterpriseEntityId, BTreeSet<_>>::new();
    let mut edge_scopes = BTreeMap::<EnterpriseEdgeId, BTreeSet<_>>::new();
    for key in &charter.required_coverage {
        let cell_id = key.id(enterprise_id)?;
        let coverage_values = events
            .iter()
            .filter(|event| {
                event.provenance.discovery_epoch_sequence == seal.discovery_epoch_sequence
                    && event.provenance.discovery_epoch == seal.discovery_epoch
            })
            .filter_map(|event| match &event.fact {
                EnterpriseFact::CoverageObserved(value) if &value.key == key => Some((
                    value.status,
                    value.next_cursor.clone(),
                    value.enumerated_count,
                    value.enumerated_edge_count,
                )),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let coverage = if coverage_values.len() == 1 {
            coverage_values.into_iter().next()
        } else {
            blockers.push(format!(
                "coverage cell {cell_id} has zero or conflicting pass observations"
            ));
            None
        };
        let task_events = events
            .iter()
            .filter(|event| {
                event.provenance.discovery_epoch_sequence == seal.discovery_epoch_sequence
                    && event.provenance.discovery_epoch == seal.discovery_epoch
            })
            .filter(|event| {
                matches!(
                    &event.fact,
                    EnterpriseFact::FrontierObserved(value) if &value.key.coverage == key
                )
            })
            .copied()
            .collect::<Vec<_>>();
        let tasks = pass_frontier(task_events, &cell_id.to_string(), &mut blockers);
        let mut scope_entities = BTreeSet::new();
        let mut scope_edges = BTreeSet::new();
        for task in tasks.values() {
            scope_entities.extend(task.entities.iter().cloned());
            scope_edges.extend(task.edges.iter().cloned());
        }
        let roots = tasks
            .iter()
            .filter(|(_, task)| task.cursor.is_none() && task.parent_task_id.is_none())
            .map(|(task_id, _)| task_id.clone())
            .collect::<Vec<_>>();
        if roots.len() != 1 {
            blockers.push(format!(
                "coverage cell {cell_id} must have exactly one root frontier task"
            ));
        }
        for (task_id, task) in &tasks {
            match (&task.cursor, &task.parent_task_id) {
                (None, Some(_)) => blockers.push(format!(
                    "coverage cell {cell_id} root task {task_id} has a parent"
                )),
                (Some(_), None) => blockers.push(format!(
                    "coverage cell {cell_id} page task {task_id} has no parent"
                )),
                (Some(cursor), Some(parent_task_id)) => {
                    let valid_parent = tasks.get(parent_task_id).is_some_and(|parent| {
                        matches!(
                            &parent.state,
                            FrontierState::PageComplete {
                                next_cursor: Some(next),
                                ..
                            } if next == cursor
                        )
                    });
                    if !valid_parent {
                        blockers.push(format!(
                            "coverage cell {cell_id} page task {task_id} has an invalid parent link"
                        ));
                    }
                }
                (None, None) => {}
            }
            match &task.state {
                FrontierState::Pending | FrontierState::Leased { .. } => blockers.push(format!(
                    "coverage cell {cell_id} retains unfinished frontier work"
                )),
                FrontierState::PageComplete {
                    next_cursor: Some(next),
                    ..
                } if !tasks.iter().any(|(candidate_id, candidate)| {
                    candidate.cursor.as_ref() == Some(next)
                        && candidate.parent_task_id.as_ref() == Some(task_id)
                        && candidate_id != task_id
                }) =>
                {
                    blockers.push(format!(
                        "coverage cell {cell_id} has an unmaterialized next-page handle"
                    ))
                }
                FrontierState::Terminal { status, .. } if status.blocks_complete() => blockers
                    .push(format!(
                        "coverage cell {cell_id} terminates with {status:?}"
                    )),
                _ => {}
            }
        }
        if let Some(root) = roots.first() {
            let mut reachable = BTreeSet::from([root.clone()]);
            loop {
                let before = reachable.len();
                for (task_id, task) in &tasks {
                    if task
                        .parent_task_id
                        .as_ref()
                        .is_some_and(|parent| reachable.contains(parent))
                    {
                        reachable.insert(task_id.clone());
                    }
                }
                if reachable.len() == before {
                    break;
                }
            }
            if reachable.len() != tasks.len() {
                blockers.push(format!(
                    "coverage cell {cell_id} contains disconnected or cyclic frontier pages"
                ));
            }
        }
        if let Some((status, next_cursor, entity_count, edge_count)) = coverage {
            if status.blocks_complete() || next_cursor.is_some() {
                blockers.push(format!("coverage cell {cell_id} is not complete"));
            }
            for task in tasks.values() {
                if let FrontierState::Terminal {
                    status: terminal_status,
                    ..
                } = task.state
                {
                    if terminal_status != status {
                        blockers.push(format!(
                            "coverage cell {cell_id} terminal status disagrees with coverage"
                        ));
                    }
                }
            }
            if status == CoverageStatus::Empty
                && (!scope_entities.is_empty() || !scope_edges.is_empty())
            {
                blockers.push(format!(
                    "coverage cell {cell_id} is empty but contains authoritative membership"
                ));
            }
            if entity_count != scope_entities.len() as u64 {
                blockers.push(format!(
                    "coverage cell {cell_id} entity count does not match frontier membership"
                ));
            }
            if edge_count != scope_edges.len() as u64 {
                blockers.push(format!(
                    "coverage cell {cell_id} edge count does not match frontier membership"
                ));
            }
            scopes.push(ScopeProjection {
                cell_id: cell_id.clone(),
                status,
                entity_ids: scope_entities.clone(),
                edge_ids: scope_edges.clone(),
            });
            for entity_id in &scope_entities {
                entity_scopes
                    .entry(entity_id.clone())
                    .or_default()
                    .insert(cell_id.clone());
            }
            for edge_id in &scope_edges {
                edge_scopes
                    .entry(edge_id.clone())
                    .or_default()
                    .insert(cell_id.clone());
            }
        }
        member_entity_ids.extend(scope_entities);
        member_edge_ids.extend(scope_edges);
    }
    scopes.sort_by(|left, right| left.cell_id.cmp(&right.cell_id));
    let scope_root = canonical_digest(&("scout-enterprise-scope-root-v1", &scopes))?;

    let as_of = events
        .iter()
        .filter(|event| event.provenance.discovery_epoch_sequence <= seal.discovery_epoch_sequence)
        .copied()
        .collect::<Vec<_>>();
    let entities = materialize_entities(&as_of);
    let mut edges = materialize_edges(&as_of);
    apply_endpoint_classification(&entities, &mut edges);
    for entity_id in &member_entity_ids {
        if !entities.contains_key(entity_id) {
            blockers.push(format!(
                "authoritative membership references missing entity {entity_id}"
            ));
        }
    }
    for edge_id in &member_edge_ids {
        if !edges.contains_key(edge_id) {
            blockers.push(format!(
                "authoritative membership references missing edge {edge_id}"
            ));
        }
    }
    let topology_root = topology_root(
        enterprise_id,
        &entities,
        &edges,
        &member_entity_ids,
        &member_edge_ids,
    )?;
    Ok(PassProjection {
        requirement_root,
        scope_root,
        topology_root,
        member_entity_ids,
        member_edge_ids,
        entity_scopes,
        edge_scopes,
        blockers,
    })
}

#[derive(Serialize)]
struct ScopeProjection {
    cell_id: super::super::super::contract::CoverageCellId,
    status: CoverageStatus,
    entity_ids: BTreeSet<EnterpriseEntityId>,
    edge_ids: BTreeSet<EnterpriseEdgeId>,
}

struct FrontierProjection {
    cursor: Option<String>,
    parent_task_id: Option<String>,
    state: FrontierState,
    entities: BTreeSet<EnterpriseEntityId>,
    edges: BTreeSet<EnterpriseEdgeId>,
}

fn pass_frontier(
    events: Vec<&EnterpriseEvent>,
    cell_id: &str,
    blockers: &mut Vec<String>,
) -> BTreeMap<String, FrontierProjection> {
    let mut grouped = BTreeMap::<String, Vec<&EnterpriseEvent>>::new();
    for event in events {
        let EnterpriseFact::FrontierObserved(value) = &event.fact else {
            continue;
        };
        grouped
            .entry(value.task_id.to_string())
            .or_default()
            .push(event);
    }
    grouped
        .into_iter()
        .filter_map(|(task_id, events)| {
            let transition = events
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::FrontierObserved(value) => Some(value.transition_sequence),
                    _ => None,
                })
                .max()?;
            let latest = events
                .into_iter()
                .filter(|event| {
                    matches!(
                        &event.fact,
                        EnterpriseFact::FrontierObserved(value)
                            if value.transition_sequence == transition
                    )
                })
                .collect::<Vec<_>>();
            let semantics = latest
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::FrontierObserved(value) => Some((
                        value.state.clone(),
                        value.parent_task_id.clone(),
                        value.discovered_entity_ids.clone(),
                        value.discovered_edge_ids.clone(),
                    )),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            if semantics.len() != 1 {
                blockers.push(format!(
                    "coverage cell {cell_id} has a conflicting frontier transition"
                ));
                return None;
            }
            let first = latest.iter().find_map(|event| match &event.fact {
                EnterpriseFact::FrontierObserved(value) => Some(value),
                _ => None,
            })?;
            let mut entities = BTreeSet::new();
            let mut edges = BTreeSet::new();
            for event in latest {
                if let EnterpriseFact::FrontierObserved(value) = &event.fact {
                    entities.extend(value.discovered_entity_ids.iter().cloned());
                    edges.extend(value.discovered_edge_ids.iter().cloned());
                }
            }
            Some((
                task_id,
                FrontierProjection {
                    cursor: first.key.cursor.clone(),
                    parent_task_id: first.parent_task_id.as_ref().map(ToString::to_string),
                    state: semantics
                        .into_iter()
                        .next()
                        .expect("one frontier semantic")
                        .0,
                    entities,
                    edges,
                },
            ))
        })
        .collect()
}

fn topology_root(
    enterprise_id: &EnterpriseId,
    entities: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    edges: &BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    member_entity_ids: &BTreeSet<EnterpriseEntityId>,
    member_edge_ids: &BTreeSet<EnterpriseEdgeId>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct EntitySemantic<'a> {
        entity_id: &'a EnterpriseEntityId,
        kind: super::super::super::contract::EnterpriseEntityKind,
        authority: &'a super::super::super::contract::AuthorityRef,
        labels: &'a BTreeSet<String>,
        environments: &'a BTreeSet<String>,
        critical: bool,
        #[serde(skip_serializing_if = "classification_is_default")]
        classification: super::super::super::contract::EnterpriseClassification,
    }
    #[derive(Serialize)]
    struct EdgeSemantic<'a> {
        edge_id: &'a EnterpriseEdgeId,
        from: &'a EnterpriseEntityId,
        to: &'a EnterpriseEntityId,
        kind: super::super::super::contract::EnterpriseEdgeKind,
        qualifier: &'a Option<String>,
        #[serde(skip_serializing_if = "classification_is_default")]
        classification: super::super::super::contract::EnterpriseClassification,
    }
    let entity_projection = member_entity_ids
        .iter()
        .filter_map(|id| entities.get(id))
        .map(|entity| EntitySemantic {
            entity_id: &entity.entity_id,
            kind: entity.kind,
            authority: &entity.authority,
            labels: &entity.labels,
            environments: &entity.environments,
            critical: entity.critical,
            classification: entity.classification,
        })
        .collect::<Vec<_>>();
    let edge_projection = member_edge_ids
        .iter()
        .filter_map(|id| edges.get(id))
        .map(|edge| EdgeSemantic {
            edge_id: &edge.edge_id,
            from: &edge.from,
            to: &edge.to,
            kind: edge.kind,
            qualifier: &edge.qualifier,
            classification: edge.classification,
        })
        .collect::<Vec<_>>();
    canonical_digest(&(
        "scout-enterprise-topology-root-v1",
        enterprise_id,
        entity_projection,
        edge_projection,
    ))
}

fn classification_is_default(
    value: &super::super::super::contract::EnterpriseClassification,
) -> bool {
    *value == super::super::super::contract::EnterpriseClassification::Internal
}

pub(super) fn pass_id(
    enterprise_id: &EnterpriseId,
    seal: &DiscoveryPassSealObservation,
    projection: &PassProjection,
) -> Result<String, String> {
    Ok(format!(
        "pass:{}",
        canonical_digest(&(
            "scout-enterprise-pass-v1",
            enterprise_id,
            &seal.charter_id,
            &seal.discovery_epoch,
            seal.discovery_epoch_sequence,
            &seal.previous_pass_id,
            &projection.requirement_root,
            &projection.scope_root,
            &projection.topology_root,
        ))?
    ))
}
