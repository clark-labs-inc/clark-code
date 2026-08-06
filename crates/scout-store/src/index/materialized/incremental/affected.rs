use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    EnterpriseClassification, EnterpriseConflict, EnterpriseEdgeId, EnterpriseEntityId,
    EnterpriseProjectionSlice, MaterializedEdge, MaterializedEntity,
};

pub(super) fn classify_candidate_edges(
    edges: &mut BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    changed_entities: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    old_entities: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
) {
    for edge in edges.values_mut() {
        for endpoint in [&edge.from, &edge.to] {
            let entity = changed_entities
                .get(endpoint)
                .or_else(|| old_entities.get(endpoint));
            if let Some(entity) = entity {
                edge.classification = edge.classification.join(entity.classification);
            }
        }
    }
}

pub(super) fn changed_edges(
    old: &BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    candidates: &BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
) -> BTreeMap<EnterpriseEdgeId, MaterializedEdge> {
    candidates
        .iter()
        .filter(|(edge_id, edge)| old.get(*edge_id) != Some(*edge))
        .map(|(edge_id, edge)| (edge_id.clone(), edge.clone()))
        .collect()
}

pub(super) fn dangling_edge_conflicts(
    candidate_edges: &BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    changed_entities: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    old_entities: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
) -> BTreeSet<EnterpriseConflict> {
    let mut conflicts = BTreeSet::new();
    for edge in candidate_edges.values() {
        for endpoint in [&edge.from, &edge.to] {
            if !changed_entities.contains_key(endpoint) && !old_entities.contains_key(endpoint) {
                conflicts.insert(EnterpriseConflict::DanglingEdge {
                    edge_id: edge.edge_id.clone(),
                    missing_entity_id: endpoint.clone(),
                });
            }
        }
    }
    conflicts
}

pub(super) fn updated_simulation_count(
    previous_count: usize,
    update: &EnterpriseProjectionSlice,
    old_entities: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    existing_simulations: &BTreeSet<EnterpriseEntityId>,
) -> Result<usize, String> {
    let relevant_ids = update
        .entities
        .keys()
        .chain(update.simulation_contracts.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut count = previous_count;
    for runtime_id in relevant_ids {
        let old_visible = existing_simulations.contains(&runtime_id)
            && old_entities.get(&runtime_id).is_some_and(internal_entity);
        let new_visible = (existing_simulations.contains(&runtime_id)
            || update.simulation_contracts.contains_key(&runtime_id))
            && update
                .entities
                .get(&runtime_id)
                .or_else(|| old_entities.get(&runtime_id))
                .is_some_and(internal_entity);
        count = count
            .checked_sub(usize::from(old_visible))
            .and_then(|value| value.checked_add(usize::from(new_visible)))
            .ok_or_else(|| {
                "Scout visible simulation count overflowed its prior state".to_string()
            })?;
    }
    Ok(count)
}

fn internal_entity(entity: &MaterializedEntity) -> bool {
    EnterpriseClassification::Internal.permits(entity.classification)
}
