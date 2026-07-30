use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::orchestration::OrchestrationConfig;

fn config() -> OrchestrationToolsConfig {
    OrchestrationToolsConfig {
        policy: OrchestrationConfig::default(),
        base_url: "https://api.clarkslabs.com/v1".into(),
        api_key: None,
        headers: HashMap::new(),
        root_model: "clark-code".into(),
        reasoning_effort: None,
        scout_capsules: None,
        scout_cartography: None,
        remote: None,
    }
}

#[test]
fn unconfigured_state_has_no_local_authority_or_tenant_arguments() {
    let state = Arc::new(CartographyBackendState::new(config()));
    assert_eq!(state.status()["configured"], false);
    assert_eq!(state.status()["local_enterprise_authority"], false);

    let writer = ScoutEnterpriseBackendTool {
        state: state.clone(),
    };
    let reader = ScoutEnterpriseBackendQueryTool { state };
    for schema in [writer.parameters(), reader.parameters()] {
        let wire = serde_json::to_string(&schema).unwrap();
        assert!(!wire.contains("organization_id"));
        assert!(!wire.contains("workspace_id"));
        assert!(!wire.contains("api_key"));
        assert!(!wire.contains("identity_root"));
        assert!(!wire.contains("enterprise_id"));
        assert!(!wire.contains("source_id"));
        assert!(!wire.contains("\"fence\""));
    }
}

#[test]
fn submission_cannot_override_the_stored_run_or_fence_binding() {
    let args: EnterpriseArgs = serde_json::from_value(json!({
        "action": "submit_adapter_receipt",
        "task_id": Uuid::new_v4(),
        "receipt_id": format!("receipt:{}", "a".repeat(64)),
    }))
    .unwrap();
    assert!(args.validate().is_ok());

    let overridden: EnterpriseArgs = serde_json::from_value(json!({
        "action": "submit_adapter_receipt",
        "run_id": Uuid::new_v4(),
        "task_id": Uuid::new_v4(),
        "receipt_id": format!("receipt:{}", "a".repeat(64)),
    }))
    .unwrap();
    assert!(overridden.validate().is_err());
}

#[test]
fn claim_result_exposes_the_exact_safe_task_to_the_model() {
    let task_id = Uuid::new_v4();
    let source_id = Uuid::new_v4();
    let outcome = claim_task_outcome(TaskClaimResponse {
        request_id: "task-claim:test".into(),
        task: Some(ClaimedTask {
            task_id,
            source_id,
            task_kind: "adapter_page".into(),
            scope: json!({
                "schema_version": 1,
                "adapter_id": "clark/github-organization@1",
            }),
            fence: 7,
            lease_expires_at: "2026-07-27T21:00:00Z".into(),
        }),
    });

    assert!(!outcome.is_error);
    assert!(outcome.content.contains(&task_id.to_string()));
    assert!(outcome.content.contains(&source_id.to_string()));
    assert!(outcome.content.contains("\"fence\":7"));
    assert!(outcome.content.contains("clark/github-organization@1"));
    assert_eq!(outcome.details["task"]["task_id"], task_id.to_string());
}

#[test]
fn empty_claim_result_remains_explicit_and_terminal() {
    let outcome = claim_task_outcome(TaskClaimResponse {
        request_id: "task-claim:empty".into(),
        task: None,
    });

    assert!(!outcome.is_error);
    assert_eq!(
        outcome.content,
        "Clark reports no claimable Scout task for this run."
    );
    assert!(outcome.details["task"].is_null());
}
