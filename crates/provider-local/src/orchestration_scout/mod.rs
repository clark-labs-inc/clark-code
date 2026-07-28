use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_orchestration::ScoutLedger;
use scout_adapter_protocol::TargetIdentity;

use crate::tools::ToolExecutor;

mod adapter;
mod capabilities;
mod capsule;
mod enterprise_backend;
mod ledger;
mod measure;
mod probe;
mod scope;

pub(super) struct ScoutToolState {
    censuses: Mutex<HashMap<String, capabilities::CapabilityReport>>,
    ledgers: Mutex<HashMap<String, ScoutLedger>>,
    target: Mutex<Option<TargetIdentity>>,
    max_parallel_agents: u16,
}

pub(super) fn tools(
    max_parallel_agents: usize,
    capsule_policy: Option<crate::orchestration::ScoutCapsulePolicyConfig>,
    cartography_config: crate::orchestration::OrchestrationToolsConfig,
) -> Vec<Arc<dyn ToolExecutor>> {
    let state = Arc::new(ScoutToolState {
        censuses: Mutex::new(HashMap::new()),
        ledgers: Mutex::new(HashMap::new()),
        target: Mutex::new(None),
        max_parallel_agents: u16::try_from(max_parallel_agents).unwrap_or(4).clamp(1, 32),
    });
    let cartography = Arc::new(enterprise_backend::CartographyBackendState::new(
        cartography_config,
    ));
    let mut tools: Vec<Arc<dyn ToolExecutor>> = vec![
        Arc::new(capabilities::ScoutCapabilitiesTool {
            state: state.clone(),
        }),
        Arc::new(adapter::ScoutAdapterTool {
            state: state.clone(),
            cartography: Some(cartography.clone()),
        }),
        Arc::new(ledger::ScoutLedgerTool {
            state: state.clone(),
        }),
        Arc::new(enterprise_backend::ScoutEnterpriseBackendTool {
            state: cartography.clone(),
        }),
        Arc::new(enterprise_backend::ScoutEnterpriseBackendQueryTool { state: cartography }),
        Arc::new(probe::ScoutProbeTool {
            state: state.clone(),
        }),
        Arc::new(measure::ScoutMeasureTool {
            state: state.clone(),
        }),
    ];
    if let Some(policy) = capsule_policy {
        tools.push(Arc::new(capsule::ScoutCapsuleTool { state, policy }));
    }
    tools
}

pub(super) fn capsule_tools(
    policy: crate::orchestration::ScoutCapsulePolicyConfig,
) -> Vec<Arc<dyn ToolExecutor>> {
    let state = Arc::new(ScoutToolState {
        censuses: Mutex::new(HashMap::new()),
        ledgers: Mutex::new(HashMap::new()),
        target: Mutex::new(None),
        max_parallel_agents: 1,
    });
    vec![
        Arc::new(adapter::ScoutAdapterTool {
            state: state.clone(),
            cartography: None,
        }),
        Arc::new(capsule::ScoutCapsuleTool { state, policy }),
    ]
}
