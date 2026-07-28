use std::collections::BTreeSet;

use agent_orchestration::{
    EnterpriseClassification, EnterpriseConflict, EnterpriseFact, EnterpriseId, EnterpriseSnapshot,
};

use crate::model::IndexedStatus;

pub(super) fn status_from_snapshot(
    enterprise_id: &EnterpriseId,
    batches: usize,
    snapshot: &EnterpriseSnapshot,
) -> IndexedStatus {
    let max_classification = EnterpriseClassification::Internal;
    let entity_is_visible = |entity_id: &agent_orchestration::EnterpriseEntityId| {
        snapshot
            .entities
            .get(entity_id)
            .is_some_and(|entity| max_classification.permits(entity.classification))
    };
    let visible_entity_count = snapshot
        .entities
        .values()
        .filter(|entity| max_classification.permits(entity.classification))
        .count();
    let visible_edge_count = snapshot
        .edges
        .values()
        .filter(|edge| max_classification.permits(edge.classification))
        .count();
    let visible_conflicts = snapshot
        .conflicts
        .iter()
        .filter(|conflict| match conflict {
            // These carry opaque event or missing-entity identifiers whose
            // classification cannot be established from a derived projection.
            EnterpriseConflict::SourceEquivocation { .. }
            | EnterpriseConflict::OrphanRetraction { .. }
            | EnterpriseConflict::RetractionOfRetraction { .. }
            | EnterpriseConflict::DanglingEdge { .. } => false,
            EnterpriseConflict::SimulationContractDisagreement { runtime_id, .. } => {
                entity_is_visible(runtime_id)
            }
            EnterpriseConflict::CoverageDisagreement { .. }
            | EnterpriseConflict::FrontierDisagreement { .. }
            | EnterpriseConflict::CharterDisagreement { .. }
            | EnterpriseConflict::DiscoveryPassInvalid { .. }
            | EnterpriseConflict::DiscoveryPassFork { .. }
            | EnterpriseConflict::DiscoveryPassNonMonotonic { .. } => true,
        })
        .cloned()
        .collect::<Vec<_>>();
    let visible_simulation_contracts = snapshot
        .simulation_contracts
        .keys()
        .filter(|runtime_id| entity_is_visible(runtime_id))
        .count();
    let mut base_completion_blockers = snapshot.control_blockers.clone();
    if visible_entity_count == 0 {
        base_completion_blockers.push("visible enterprise graph contains no entities".into());
    }
    if !visible_conflicts.is_empty() {
        base_completion_blockers.push(format!(
            "visible enterprise graph contains {} unresolved conflicts",
            visible_conflicts.len()
        ));
    }
    IndexedStatus {
        enterprise_id: enterprise_id.clone(),
        max_classification,
        event_root: snapshot.event_root.clone(),
        graph_digest: snapshot.graph_digest.clone(),
        event_set_root_v1: None,
        projection_map_root_v2: None,
        enterprise_snapshot_root_v2: None,
        batches,
        events: snapshot.event_count,
        entities: visible_entity_count,
        edges: visible_edge_count,
        coverage_cells: snapshot.coverage.len(),
        frontier_tasks: snapshot.frontier.len(),
        simulation_contracts: visible_simulation_contracts,
        charter: snapshot.charter.clone(),
        discovery_passes: snapshot.discovery_passes.len(),
        current_pass_id: snapshot.current_pass_id.clone(),
        current_pass_sealed_at_ms: snapshot
            .current_pass_id
            .as_ref()
            .and_then(|pass_id| snapshot.discovery_passes.get(pass_id))
            .map(|pass| pass.sealed_at_ms),
        fixed_point: snapshot.fixed_point,
        base_completion_blockers,
        conflict_count: visible_conflicts.len(),
        conflicts: visible_conflicts.into_iter().take(64).collect(),
    }
}

pub(super) fn retracted_event_ids<'a>(
    events: impl IntoIterator<Item = &'a agent_orchestration::EnterpriseEvent>,
) -> BTreeSet<String> {
    let events = events
        .into_iter()
        .map(|event| (event.event_id.as_str(), event))
        .collect::<std::collections::BTreeMap<_, _>>();
    events
        .values()
        .filter_map(|event| {
            let EnterpriseFact::ObservationRetracted {
                target_event_id, ..
            } = &event.fact
            else {
                return None;
            };
            events.get(target_event_id.as_str()).and_then(|target| {
                (!matches!(target.fact, EnterpriseFact::ObservationRetracted { .. }))
                    .then(|| target_event_id.to_string())
            })
        })
        .collect()
}
