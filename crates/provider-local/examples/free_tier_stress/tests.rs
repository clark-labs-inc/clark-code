use std::collections::BTreeMap;

use serde_json::json;

use super::model::{summarize, CaseReceipt, TrajectoryReceipt};
use super::turn::route_failures;
use super::MODEL;

#[test]
fn route_contract_rejects_missing_identity_and_fallbacks() {
    assert!(route_failures(&[])
        .iter()
        .any(|failure| failure.contains("no model_response")));
    let failures = route_failures(&[json!({
        "requested_model": MODEL,
        "resolved_model": "resolved/free",
        "provider": "provider-a",
        "fallback_model": "paid/model"
    })]);
    assert_eq!(failures, ["response 0 used a fallback model"]);
}

#[test]
fn summary_requires_each_scenario_to_cross_the_gate() {
    let case = |id, passed| CaseReceipt {
        id,
        repetition: 0,
        verdict: if passed { "passed" } else { "quality_failure" },
        passed,
        infrastructure_failure: false,
        route_valid: true,
        duration_ms: 1,
        outcome: None,
        usage: None,
        text: String::new(),
        tools: Vec::new(),
        goal_completed: false,
        event_counts: BTreeMap::new(),
        model_responses: Vec::new(),
        errors: Vec::new(),
        oracle_failures: Vec::new(),
    };
    let trajectories = vec![TrajectoryReceipt {
        repetition: 0,
        workspace: "fixture".into(),
        error: None,
        cases: vec![case("strong", true), case("weak", false)],
    }];
    let summary = summarize(&trajectories);
    assert!(!summary.gate_passed);
    assert_eq!(summary.by_scenario["weak"].pass_rate, 0.0);
}
