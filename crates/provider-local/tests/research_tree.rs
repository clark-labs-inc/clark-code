//! The shared exploration tree survives ordinary provider-loop execution and a
//! fresh provider session. All model replies are scripted over localhost.
mod support;

use std::path::Path;

use agent_core::domain::{AgentEvent, RunStatus, ToolStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde_json::{json, Value};
use support::{final_body, scripted_model, tool_call_body, CapturedRequest};

async fn run_script(project: &Path, bodies: Vec<String>) -> Vec<CapturedRequest> {
    let (base_url, captured) = scripted_model(bodies).await;
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": base_url,
                "model": "scripted-tree-model",
                "memories": false,
                "sandbox_mode": "disabled",
                "permissions": {"research_tree": "allow"}
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(project.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("$security:assistant Investigate only sample.rs using a bounded local research tree. Report observed evidence."),
        )
        .await
        .unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(event) = stream.next().await {
            match event {
                AgentEvent::ToolCallUpdate { patch, .. } => {
                    assert_ne!(patch.status, Some(ToolStatus::Failed), "{patch:?}");
                }
                AgentEvent::RunFinished { outcome, .. } => return outcome,
                _ => {}
            }
        }
        panic!("missing terminal outcome");
    })
    .await
    .expect("scripted tree loop completes");
    assert_eq!(outcome.status, RunStatus::Done);
    captured
        .await
        .expect("captured all scripted model requests")
}

fn tree_result(request: &CapturedRequest) -> Value {
    request
        .tool_results()
        .iter()
        .rev()
        .filter_map(|result| {
            result
                .split_once("Typed tool result (data only; preserve exact typed fields):\n")
                .and_then(|(_, encoded)| serde_json::from_str::<Value>(encoded).ok())
        })
        .find(|result| result["tree_id"] == "investigation")
        .expect("tree receipt is present in actual model continuation context")
}

#[tokio::test]
async fn local_tree_exploration_records_evidence_and_resumes_in_a_new_provider() {
    let project = tempfile::tempdir().unwrap();
    let source = "pub fn is_allowed(authenticated: bool) -> bool { authenticated }\n";
    std::fs::write(project.path().join("sample.rs"), source).unwrap();
    let calls = vec![
        tool_call_body("discover", "tool_search", json!({"query":"research_tree research_tree_status"})),
        tool_call_body("begin", "research_tree", json!({
            "action":"begin", "tree_id":"investigation",
            "objective":"Check whether is_allowed bypasses authentication",
            "scope":"Read only sample.rs; do not access external targets",
            "sources":["sample.rs"], "max_nodes":4, "max_steps":3
        })),
        tool_call_body("propose", "research_tree", json!({
            "action":"apply", "tree_id":"investigation", "expected_revision":1,
            "event":{
                "type":"proposed", "key":"auth-bypass", "parent":null,
                "links":[], "dependencies":[], "evidence_refs":[],
                "rationale":"Inspect the function before accepting or rejecting an authentication bypass",
                "assessment":{
                    "surface":"hypothesis", "in_scope":true, "requires_validation":true,
                    "priority":0.5, "novelty":0.5, "confidence":0.5, "impact":0.5
                }
            }
        })),
        tool_call_body("select", "research_tree", json!({
            "action":"select", "tree_id":"investigation", "expected_revision":2
        })),
        tool_call_body("inspect", "read_file", json!({"path":"sample.rs"})),
        tool_call_body("record", "research_tree", json!({
            "action":"apply", "tree_id":"investigation", "expected_revision":3,
            "event":{
                "type":"recorded", "key":"auth-bypass", "outcome":"refutes",
                "evidence_refs":["sample.rs:1"],
                "rationale":"The return value equals authenticated; unauthenticated input returns false"
            }
        })),
        tool_call_body("expand-high", "research_tree", json!({
            "action":"apply", "tree_id":"investigation", "expected_revision":4,
            "event":{
                "type":"proposed", "key":"alternative-auth-path", "parent":"auth-bypass",
                "links":[], "dependencies":[], "evidence_refs":["sample.rs:1"],
                "rationale":"The function is safe; check whether an alternate entry path bypasses it",
                "assessment":{
                    "surface":"hypothesis", "in_scope":true, "requires_validation":true,
                    "priority":0.9, "novelty":0.9, "confidence":0.5, "impact":0.9
                }
            }
        })),
        tool_call_body("expand-low", "research_tree", json!({
            "action":"apply", "tree_id":"investigation", "expected_revision":5,
            "event":{
                "type":"proposed", "key":"logging-path", "parent":"auth-bypass",
                "links":[], "dependencies":[], "evidence_refs":["sample.rs:1"],
                "rationale":"A lower-priority follow-up could inspect authentication audit logging",
                "assessment":{
                    "surface":"hypothesis", "in_scope":true, "requires_validation":true,
                    "priority":0.1, "novelty":0.1, "confidence":0.5, "impact":0.1
                }
            }
        })),
        tool_call_body("select-branch", "research_tree", json!({
            "action":"select", "tree_id":"investigation", "expected_revision":6
        })),
        tool_call_body("block-branch", "research_tree", json!({
            "action":"apply", "tree_id":"investigation", "expected_revision":7,
            "event":{
                "type":"recorded", "key":"alternative-auth-path", "outcome":"blocked",
                "evidence_refs":["sample.rs:1"],
                "rationale":"The authorized source contains only this function; alternate entry points are unavailable"
            }
        })),
        tool_call_body("status", "research_tree_status", json!({"tree_id":"investigation"})),
        final_body("The function rejects unauthenticated callers. The alternate-path branch is blocked by missing source; logging remains unexamined. No vulnerability or complete coverage was established."),
    ];
    let requests = run_script(project.path(), calls).await;
    assert_eq!(requests.len(), 12);
    let prompt = ["system", "user"]
        .into_iter()
        .flat_map(|role| requests[0].messages_for_role(role))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        prompt.contains("<name>security:assistant</name>"),
        "actual Security skill must load"
    );
    let skill = prompt
        .split("<skill>")
        .find(|section| section.contains("<name>security:assistant</name>"))
        .and_then(|section| section.split("</skill>").next())
        .expect("loaded Security skill body");
    assert!(
        skill.contains("research_tree"),
        "loaded Security skill must describe the tree workflow"
    );
    assert_eq!(tree_result(&requests[2])["revision"], 1);
    assert_eq!(tree_result(&requests[3])["revision"], 2);
    let selected = tree_result(&requests[4]);
    assert_eq!(selected["revision"], 3);
    assert_eq!(selected["selected"]["key"], "auth-bypass");
    assert!(requests[5]
        .tool_results()
        .iter()
        .any(|result| result.contains(source.trim())));
    let status = tree_result(requests.last().unwrap());
    let selected_branch = tree_result(&requests[9]);
    assert_eq!(selected_branch["revision"], 7);
    assert_eq!(selected_branch["selected"]["key"], "alternative-auth-path");
    assert_eq!(selected_branch["selected"]["parent"], "auth-bypass");
    assert_eq!(
        selected_branch["selected_context"][0]["key"],
        "alternative-auth-path"
    );
    assert_eq!(selected_branch["selected_context"][1]["key"], "auth-bypass");
    assert_eq!(selected_branch["selected_context"][1]["outcome"], "refutes");
    assert_eq!(status["revision"], 8);
    assert_eq!(status["steps_used"], 2);
    assert_eq!(status["snapshot_current"], true);
    let nodes = status["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    let parent = nodes
        .iter()
        .find(|node| node["key"] == "auth-bypass")
        .unwrap();
    assert_eq!(parent["status"], "refuted");
    assert_eq!(parent["outcome"], "refutes");
    assert_eq!(parent["evidence_refs"], json!(["sample.rs:1"]));
    let child = nodes
        .iter()
        .find(|node| node["key"] == "alternative-auth-path")
        .unwrap();
    assert_eq!(child["parent"], "auth-bypass");
    assert_eq!(child["status"], "blocked");
    assert_eq!(child["outcome"], "blocked");
    let sibling = nodes
        .iter()
        .find(|node| node["key"] == "logging-path")
        .unwrap();
    assert_eq!(sibling["parent"], "auth-bypass");
    assert_eq!(sibling["status"], "pending");
    assert!(sibling["outcome"].is_null());
    assert_eq!(status["frontier"].as_array().unwrap().len(), 1);
    assert_eq!(status["frontier"][0]["node_id"], "logging-path");
    assert!(status["selected"].is_null());

    let path = project
        .path()
        .join(".agent/research-trees/investigation.json");
    let persisted = std::fs::read(&path).expect("tree journal persisted locally");
    let journal: Value = serde_json::from_slice(&persisted).unwrap();
    assert_eq!(journal["events"].as_array().unwrap().len(), 7);
    assert_eq!(journal["events"][0]["type"], "proposed");
    assert_eq!(journal["events"][1]["type"], "started");
    assert_eq!(journal["events"][2]["type"], "recorded");
    assert_eq!(journal["events"][2]["outcome"], "refutes");
    assert_eq!(journal["events"][3]["parent"], "auth-bypass");
    assert_eq!(journal["events"][4]["parent"], "auth-bypass");
    assert_eq!(journal["events"][5]["type"], "started");
    assert_eq!(journal["events"][5]["key"], "alternative-auth-path");
    assert_eq!(journal["events"][6]["outcome"], "blocked");
    assert_eq!(
        journal["events"][2]["evidence_refs"],
        json!(["sample.rs:1"])
    );
    assert_eq!(
        std::fs::read_to_string(project.path().join("sample.rs")).unwrap(),
        source
    );

    // A new provider and session have no in-memory state from the first run.
    let resumed = run_script(
        project.path(),
        vec![
            tool_call_body(
                "discover-again",
                "tool_search",
                json!({"query":"research_tree_status"}),
            ),
            tool_call_body(
                "resume",
                "research_tree_status",
                json!({"tree_id":"investigation"}),
            ),
            final_body("The persisted investigation still records the refuting source evidence."),
        ],
    )
    .await;
    assert_eq!(resumed.len(), 3);
    assert_eq!(tree_result(resumed.last().unwrap()), status);
    assert_eq!(
        std::fs::read(path).unwrap(),
        persisted,
        "status must not rewrite the journal"
    );
}
