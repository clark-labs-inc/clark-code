use std::collections::BTreeMap;

use agent_orchestration::{EnterpriseSnapshot, MaterializedCharter, MaterializedDiscoveryPass};
use serde::Serialize;

use crate::index::materialized::state::ProjectionState;

#[derive(Serialize)]
pub(super) struct ProjectionControl<'a> {
    schema: &'static str,
    retracted_event_count: usize,
    charter: &'a Option<MaterializedCharter>,
    discovery_passes: &'a BTreeMap<String, MaterializedDiscoveryPass>,
    current_pass_id: &'a Option<String>,
    fixed_point: bool,
    control_blockers: &'a [String],
}

impl<'a> From<&'a EnterpriseSnapshot> for ProjectionControl<'a> {
    fn from(snapshot: &'a EnterpriseSnapshot) -> Self {
        Self {
            schema: "scout-projection-control-v2",
            retracted_event_count: snapshot.retracted_event_count,
            charter: &snapshot.charter,
            discovery_passes: &snapshot.discovery_passes,
            current_pass_id: &snapshot.current_pass_id,
            fixed_point: snapshot.fixed_point,
            control_blockers: &snapshot.control_blockers,
        }
    }
}

impl<'a> From<&'a ProjectionState> for ProjectionControl<'a> {
    fn from(state: &'a ProjectionState) -> Self {
        Self {
            schema: "scout-projection-control-v2",
            retracted_event_count: state.retracted_event_count,
            charter: &state.charter,
            discovery_passes: &state.discovery_passes,
            current_pass_id: &state.current_pass_id,
            fixed_point: state.fixed_point,
            control_blockers: &state.control_blockers,
        }
    }
}
