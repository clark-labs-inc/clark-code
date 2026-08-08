use sha2::{Digest, Sha256};

use super::model::{InteractionReceipt, PlanReceipt, Scenario, TraceAuthority};

pub fn plan(scenario: &Scenario, delegated: bool, authority: TraceAuthority) -> PlanReceipt {
    PlanReceipt {
        graph_id: digest(&format!("{}:{delegated}", scenario.id)),
        authority,
        task_ids: scenario.tasks.iter().map(|task| task.id.clone()).collect(),
        resource_ids: scenario
            .resources
            .iter()
            .map(|resource| resource.id.clone())
            .collect(),
        validated_at_ms: 30,
        delegated,
    }
}

pub fn simple_interaction() -> InteractionReceipt {
    InteractionReceipt {
        default_flow: true,
        setup_actions: 2,
        completion_actions: 1,
        model_choice_required: false,
        agent_configuration_required: false,
        version_control_knowledge_required: false,
        advanced_details_collapsed: true,
        plain_language_progress: true,
        exposed_internal_terms: Vec::new(),
    }
}

pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
