use std::collections::BTreeMap;

use agent_orchestration::{EnterpriseSnapshot, MaterializedCharter, MaterializedDiscoveryPass};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectionState {
    pub retracted_event_count: usize,
    pub entity_row_count: usize,
    pub edge_row_count: usize,
    pub coverage_cell_count: usize,
    pub frontier_task_count: usize,
    pub simulation_contract_count: usize,
    pub entity_history_row_count: usize,
    pub edge_history_row_count: usize,
    pub charter: Option<MaterializedCharter>,
    pub discovery_passes: BTreeMap<String, MaterializedDiscoveryPass>,
    pub current_pass_id: Option<String>,
    pub fixed_point: bool,
    pub control_blockers: Vec<String>,
    pub conflict_count: usize,
    pub visible_conflict_count: usize,
}

impl ProjectionState {
    pub(super) fn from_snapshot(snapshot: &EnterpriseSnapshot) -> Self {
        Self {
            retracted_event_count: snapshot.retracted_event_count,
            entity_row_count: snapshot.entities.len(),
            edge_row_count: snapshot.edges.len(),
            coverage_cell_count: snapshot.coverage.len(),
            frontier_task_count: snapshot.frontier.len(),
            simulation_contract_count: snapshot.simulation_contracts.len(),
            entity_history_row_count: snapshot.entity_history.values().map(Vec::len).sum(),
            edge_history_row_count: snapshot.edge_history.values().map(Vec::len).sum(),
            charter: snapshot.charter.clone(),
            discovery_passes: snapshot.discovery_passes.clone(),
            current_pass_id: snapshot.current_pass_id.clone(),
            fixed_point: snapshot.fixed_point,
            control_blockers: snapshot.control_blockers.clone(),
            conflict_count: snapshot.conflicts.len(),
            visible_conflict_count: super::conflicts::snapshot_visible_count(snapshot),
        }
    }

    pub(super) fn sealed_epoch_sequence(&self) -> Option<u64> {
        self.current_pass_id
            .as_ref()
            .and_then(|pass_id| self.discovery_passes.get(pass_id))
            .map(|pass| pass.discovery_epoch_sequence)
    }
}
