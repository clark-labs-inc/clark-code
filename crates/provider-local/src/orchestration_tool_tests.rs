use std::collections::HashMap;
use std::path::PathBuf;

use agent_orchestration::OrchestrationPurpose;
use exec_core::WalkEntry;
use serde_json::json;

use super::*;
use crate::orchestration::OrchestrationConfig;

fn tools_config() -> OrchestrationToolsConfig {
    OrchestrationToolsConfig {
        policy: OrchestrationConfig {
            enabled: true,
            ..Default::default()
        },
        base_url: "https://example.invalid/v1".to_string(),
        api_key: None,
        headers: HashMap::new(),
        root_model: "root-model".to_string(),
        reasoning_effort: None,
        scout_capsules: None,
        scout_cartography: None,
    }
}

#[test]
fn orchestration_tools_share_a_bounded_control_plane() {
    let tools = orchestration_tools(tools_config());
    assert_eq!(tools.len(), 9);
    assert_eq!(tools[0].name(), "delegate_read_only");
    assert_eq!(tools[1].name(), "resolve_delegation");
    assert_eq!(tools[2].name(), "delegate_coding_workstreams");
    assert_eq!(tools[3].name(), "resolve_coding_workstreams");
    assert_eq!(tools[4].name(), "scout_capabilities");
    assert_eq!(tools[5].name(), "scout_repository_census");
    assert_eq!(tools[6].name(), "scout_adapter");
    assert_eq!(tools[7].name(), "scout_enterprise");
    assert_eq!(tools[8].name(), "scout_enterprise_query");
    assert!(!tools[0].mutating());
    assert!(!tools[1].mutating());
    assert!(tools[2].mutating());
    assert!(tools[3].mutating());
    assert!(!tools[4].mutating());
    assert!(!tools[5].mutating());
    assert!(tools[6].mutating());
    assert!(tools[7].mutating());
    assert!(!tools[8].mutating());
}

#[test]
fn scout_model_policy_ignores_root_child_and_harness_configuration() {
    let mut config = tools_config();
    config.root_model = "user-root-model".to_string();
    config.reasoning_effort = Some("high".to_string());
    config.policy.subagent_model = Some("user-child-model".to_string());
    config.policy.read_only_harness = "user-acp-harness".to_string();

    let policy = delegation_model_policy(
        &config,
        Some(crate::tools::TurnModelOverride {
            model: "host-scout-model".into(),
            reasoning_effort: Some("max".into()),
        }),
    );

    assert_eq!(policy.root_model, "host-scout-model");
    assert_eq!(policy.child_model, "host-scout-model");
    assert_eq!(policy.harness, "local");
    assert_eq!(policy.reasoning_effort.as_deref(), Some("max"));
}

#[test]
fn scout_schemas_commit_to_action_and_identity_before_payload() {
    let tools = orchestration_tools(tools_config());
    let schema = |name: &str| {
        let tool = tools.iter().find(|tool| tool.name() == name).unwrap();
        serde_json::to_string(&tool.parameters()).unwrap()
    };
    let enterprise = schema("scout_enterprise");
    assert!(enterprise.find("\"action\"").unwrap() < enterprise.find("\"run_id\"").unwrap());
    assert!(enterprise.contains("submit_adapter_receipt"));
    assert!(enterprise.find("\"task_id\"").unwrap() < enterprise.find("\"receipt_id\"").unwrap());
    assert!(!enterprise.contains("\"organization_id\""));
    assert!(!enterprise.contains("\"workspace_id\""));
    assert!(!enterprise.contains("\"api_key\""));
    assert!(!enterprise.contains("\"identity_root\""));
    assert!(!enterprise.contains("\"source_id\""));
    assert!(!enterprise.contains("\"fence\""));

    let enterprise_query = schema("scout_enterprise_query");
    assert!(
        enterprise_query.find("\"action\"").unwrap()
            < enterprise_query.find("\"effective_at_ms\"").unwrap()
    );
    assert!(!enterprise_query.contains("\"organization_id\""));
    assert!(!enterprise_query.contains("\"workspace_id\""));

    let adapter = schema("scout_adapter");
    assert!(adapter.find("\"action\"").unwrap() < adapter.find("\"data\"").unwrap());
    let adapter_tool = tools
        .iter()
        .find(|tool| tool.name() == "scout_adapter")
        .unwrap();
    assert_eq!(
        adapter_tool.permission_class(),
        crate::tools::ToolPermissionClass::External
    );

    let repositories = schema("scout_repository_census");
    assert!(
        repositories.find("\"action\"").unwrap() < repositories.find("\"checkout_id\"").unwrap()
    );
    assert!(repositories.contains("\"collect\""));
}

#[test]
fn delegation_schema_conditions_decision_before_workstreams() {
    let wire = serde_json::to_string(&delegate_schema()).unwrap();
    let objective = wire.find("\"objective\"").unwrap();
    let purpose = wire.find("\"purpose\"").unwrap();
    let workstreams = wire.find("\"workstreams\"").unwrap();
    assert!(objective < purpose);
    assert!(purpose < workstreams);
    let id = wire.find("\"id\"").unwrap();
    let workstream_objective = wire[id..].find("\"objective\"").unwrap() + id;
    let scopes = wire.find("\"scopes\"").unwrap();
    let acceptance = wire.find("\"acceptance\"").unwrap();
    assert!(id < workstream_objective);
    assert!(workstream_objective < scopes);
    assert!(scopes < acceptance);
}

#[test]
fn delegation_schema_keeps_host_policy_out_of_model_arguments() {
    let wire = serde_json::to_string(&delegate_schema()).unwrap();
    assert!(!wire.contains("root_estimated_output_tokens"));
    assert!(!wire.contains("estimated_output_tokens"));
    assert!(!wire.contains("risk"));
    assert!(!wire.contains("harness"));

    let args: DelegateArgs = serde_json::from_value(json!({
        "objective": "review the security boundary",
        "purpose": "review",
        "workstreams": [{
            "id": "review",
            "objective": "inspect auth",
            "scopes": ["src/auth"],
            "acceptance": ["cite concrete evidence"]
        }]
    }))
    .unwrap();
    assert_eq!(args.purpose, OrchestrationPurpose::Review);
}

#[test]
fn context_estimate_counts_only_resolved_scopes() {
    let entries = vec![
        WalkEntry {
            path: PathBuf::from("/repo/a/one.rs"),
            modified: None,
            len: 400,
        },
        WalkEntry {
            path: PathBuf::from("/repo/b/two.rs"),
            modified: None,
            len: 800,
        },
    ];
    assert_eq!(
        estimate_context_tokens(&entries, &[PathBuf::from("/repo/a")]),
        100
    );
}

#[cfg(target_os = "macos")]
#[test]
fn acp_command_is_wrapped_in_an_os_write_denial() {
    let root = tempfile::tempdir().unwrap();
    let command = os_read_only_command(
        &["codex".to_string(), "acp".to_string()],
        root.path().to_str().unwrap(),
    )
    .unwrap();
    assert_eq!(command[0], "/usr/bin/sandbox-exec");
    assert!(command[2].contains("deny file-write"));
    assert_eq!(&command[4..], ["codex", "acp"]);
}
