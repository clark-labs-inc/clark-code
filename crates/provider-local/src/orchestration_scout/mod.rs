use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_orchestration::ScoutLedger;

use crate::tools::ToolExecutor;

mod capabilities;
mod ledger;
mod measure;
mod probe;
mod scope;

pub(super) struct ScoutToolState {
    censuses: Mutex<HashMap<String, capabilities::CapabilityReport>>,
    ledgers: Mutex<HashMap<String, ScoutLedger>>,
    max_parallel_agents: u16,
}

pub(super) fn tools(max_parallel_agents: usize) -> Vec<Arc<dyn ToolExecutor>> {
    let state = Arc::new(ScoutToolState {
        censuses: Mutex::new(HashMap::new()),
        ledgers: Mutex::new(HashMap::new()),
        max_parallel_agents: u16::try_from(max_parallel_agents).unwrap_or(4).clamp(1, 32),
    });
    vec![
        Arc::new(capabilities::ScoutCapabilitiesTool {
            state: state.clone(),
        }),
        Arc::new(ledger::ScoutLedgerTool {
            state: state.clone(),
        }),
        Arc::new(probe::ScoutProbeTool {
            state: state.clone(),
        }),
        Arc::new(measure::ScoutMeasureTool { state }),
    ]
}
