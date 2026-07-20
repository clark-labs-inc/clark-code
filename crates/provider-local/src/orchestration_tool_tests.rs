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
    }
}

#[test]
fn orchestration_tools_share_a_bounded_control_plane() {
    let tools = orchestration_tools(tools_config());
    assert_eq!(tools.len(), 4);
    assert_eq!(tools[0].name(), "delegate_read_only");
    assert_eq!(tools[1].name(), "resolve_delegation");
    assert_eq!(tools[2].name(), "delegate_coding_workstreams");
    assert_eq!(tools[3].name(), "resolve_coding_workstreams");
    assert!(!tools[0].mutating());
    assert!(!tools[1].mutating());
    assert!(tools[2].mutating());
    assert!(tools[3].mutating());
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
