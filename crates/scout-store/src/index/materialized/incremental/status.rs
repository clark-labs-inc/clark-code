use std::collections::BTreeMap;

use agent_orchestration::{EnterpriseClassification, EnterpriseConflict};

use super::super::state::ProjectionState;
use crate::model::IndexedStatus;

pub(super) fn adjust_visible_count<T>(
    previous_count: usize,
    old: &BTreeMap<T, impl Classified>,
    new: &BTreeMap<T, impl Classified>,
) -> Result<usize, String>
where
    T: Ord,
{
    let removed = old.values().filter(|value| value.is_internal()).count();
    let added = new.values().filter(|value| value.is_internal()).count();
    previous_count
        .checked_sub(removed)
        .and_then(|count| count.checked_add(added))
        .ok_or_else(|| "Scout visible projection count overflowed its prior state".to_string())
}

pub(super) struct StatusCounts {
    pub batches: usize,
    pub events: usize,
    pub entities: usize,
    pub edges: usize,
    pub simulation_contracts: usize,
}

pub(super) fn build(
    enterprise_id: &agent_orchestration::EnterpriseId,
    state: &ProjectionState,
    counts: StatusCounts,
    conflict_preview: Vec<EnterpriseConflict>,
) -> IndexedStatus {
    let mut base_completion_blockers = state.control_blockers.clone();
    if counts.entities == 0 {
        base_completion_blockers.push("visible enterprise graph contains no entities".into());
    }
    if state.visible_conflict_count != 0 {
        base_completion_blockers.push(format!(
            "visible enterprise graph contains {} unresolved conflicts",
            state.visible_conflict_count
        ));
    }
    IndexedStatus {
        enterprise_id: enterprise_id.clone(),
        max_classification: EnterpriseClassification::Internal,
        event_root: String::new(),
        graph_digest: String::new(),
        event_set_root_v1: None,
        projection_map_root_v2: None,
        enterprise_snapshot_root_v2: None,
        batches: counts.batches,
        events: counts.events,
        entities: counts.entities,
        edges: counts.edges,
        coverage_cells: state.coverage_cell_count,
        frontier_tasks: state.frontier_task_count,
        simulation_contracts: counts.simulation_contracts,
        charter: state.charter.clone(),
        discovery_passes: state.discovery_passes.len(),
        current_pass_id: state.current_pass_id.clone(),
        current_pass_sealed_at_ms: state
            .current_pass_id
            .as_ref()
            .and_then(|pass_id| state.discovery_passes.get(pass_id))
            .map(|pass| pass.sealed_at_ms),
        fixed_point: state.fixed_point,
        base_completion_blockers,
        conflict_count: state.visible_conflict_count,
        conflicts: conflict_preview,
    }
}

pub(super) trait Classified {
    fn classification(&self) -> EnterpriseClassification;

    fn is_internal(&self) -> bool {
        EnterpriseClassification::Internal.permits(self.classification())
    }
}

impl Classified for agent_orchestration::MaterializedEntity {
    fn classification(&self) -> EnterpriseClassification {
        self.classification
    }
}

impl Classified for agent_orchestration::MaterializedEdge {
    fn classification(&self) -> EnterpriseClassification {
        self.classification
    }
}
