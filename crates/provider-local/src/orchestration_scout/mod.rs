use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use scout_adapter_protocol::TargetIdentity;

use crate::tools::ToolExecutor;

mod adapter;
mod capabilities;
mod capsule;
mod enterprise_backend;
mod repository_census;

pub(super) struct ScoutToolState {
    target: Mutex<Option<TargetIdentity>>,
    repositories: Mutex<HashMap<String, std::path::PathBuf>>,
    /// Target-service adapter calls share private state. Serialize a model
    /// tool batch so duplicate census/fetch calls cannot race that state.
    adapter_gate: tokio::sync::Mutex<()>,
}

pub(super) fn tools(
    capsule_policy: Option<crate::orchestration::ScoutCapsulePolicyConfig>,
    cartography_config: crate::orchestration::OrchestrationToolsConfig,
) -> Vec<Arc<dyn ToolExecutor>> {
    let state = Arc::new(ScoutToolState {
        target: Mutex::new(None),
        repositories: Mutex::new(HashMap::new()),
        adapter_gate: tokio::sync::Mutex::new(()),
    });
    let cartography = Arc::new(enterprise_backend::CartographyBackendState::new(
        cartography_config,
    ));
    let mut tools: Vec<Arc<dyn ToolExecutor>> = vec![
        Arc::new(capabilities::ScoutCapabilitiesTool),
        Arc::new(repository_census::ScoutRepositoryCensusTool {
            state: state.clone(),
            cartography: cartography.clone(),
        }),
        Arc::new(adapter::ScoutAdapterTool {
            state: state.clone(),
            cartography: Some(cartography.clone()),
        }),
        Arc::new(enterprise_backend::ScoutEnterpriseBackendTool {
            state: cartography.clone(),
        }),
        Arc::new(enterprise_backend::ScoutEnterpriseBackendQueryTool { state: cartography }),
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
        target: Mutex::new(None),
        repositories: Mutex::new(HashMap::new()),
        adapter_gate: tokio::sync::Mutex::new(()),
    });
    vec![
        Arc::new(adapter::ScoutAdapterTool {
            state: state.clone(),
            cartography: None,
        }),
        Arc::new(capsule::ScoutCapsuleTool { state, policy }),
    ]
}
