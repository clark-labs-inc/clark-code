use std::collections::HashMap;
use std::path::PathBuf;

use agent_orchestration::{OrchestrationPurpose, RiskSignals};
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
fn tool_pair_shares_an_opt_in_control_plane() {
    let tools = orchestration_tools(tools_config());
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name(), "delegate_read_only");
    assert_eq!(tools[1].name(), "resolve_delegation");
    assert!(!tools[0].mutating());
    assert!(!tools[1].mutating());
}

#[test]
fn delegation_schema_conditions_decision_before_workstreams() {
    let wire = serde_json::to_string(&delegate_schema(&[])).unwrap();
    let objective = wire.find("\"objective\"").unwrap();
    let purpose = wire.find("\"purpose\"").unwrap();
    let workstreams = wire.find("\"workstreams\"").unwrap();
    assert!(objective < purpose);
    assert!(purpose < workstreams);
}

#[test]
fn partial_risk_signals_default_fail_closed_fields() {
    let args: DelegateArgs = serde_json::from_value(json!({
        "objective": "review the security boundary",
        "purpose": "review",
        "risk": {"touches_auth_or_security": true},
        "workstreams": [{
            "id": "review",
            "objective": "inspect auth",
            "scopes": ["src/auth"],
            "acceptance": ["cite concrete evidence"]
        }]
    }))
    .unwrap();
    assert_eq!(args.purpose, OrchestrationPurpose::Review);
    assert_eq!(
        args.risk,
        RiskSignals {
            touches_auth_or_security: true,
            ..Default::default()
        }
    );
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
    let command = os_read_only_command(&["codex".to_string(), "acp".to_string()]).unwrap();
    assert_eq!(command[0], "/usr/bin/sandbox-exec");
    assert!(command[2].contains("deny file-write"));
    assert_eq!(&command[4..], ["codex", "acp"]);
}
