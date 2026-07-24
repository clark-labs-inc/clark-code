//! Full model/tool-loop simulation for native computer use. The fake model is
//! configured as Kimi K3 so screenshot results must travel over the same native
//! image-content path used in production.

mod support;

use agent_core::domain::{AgentEvent, ContentBlock, RunStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use computer_use::SimulatedComputerBackend;
use futures::StreamExt;
use serde_json::json;

use support::{final_body, scripted_model, tool_call_body};

#[tokio::test]
async fn kimi_k3_observes_types_clicks_and_reobserves_the_simulated_desktop() {
    let target = json!({
        "app_bundle_id": SimulatedComputerBackend::BUNDLE_ID,
        "pid": 42_424,
        "window_id": 7,
    });
    let (base_url, captured) = scripted_model(vec![
        tool_call_body(
            "search",
            "tool_search",
            json!({"query": "computer control apps windows screen", "limit": 12}),
        ),
        tool_call_body("list", "computer_list_windows", json!({})),
        tool_call_body("observe-1", "computer_get_state", target.clone()),
        tool_call_body(
            "type",
            "computer_type_text",
            merge(
                &target,
                json!({
                    "risk": "routine",
                    "reason": "enter text into the local test field",
                    "observation_id": "sim-observation-0",
                    "element_id": "ax-1",
                    "text": "hello from Kimi K3",
                    "replace": true,
                }),
            ),
        ),
        tool_call_body(
            "commit-type",
            "computer_commit_action",
            json!({"prepared_action_id": "sim-prepared-0"}),
        ),
        tool_call_body("observe-2", "computer_get_state", target.clone()),
        tool_call_body(
            "click",
            "computer_click",
            merge(
                &target,
                json!({
                    "risk": "routine",
                    "reason": "open the benign simulated example",
                    "observation_id": "sim-observation-1",
                    "element_id": "ax-2"
                }),
            ),
        ),
        tool_call_body(
            "commit-click",
            "computer_commit_action",
            json!({"prepared_action_id": "sim-prepared-1"}),
        ),
        tool_call_body("observe-3", "computer_get_state", target),
        final_body("The simulated app contains the entered text and reports Opened example."),
    ])
    .await;

    let project = tempfile::tempdir().unwrap();
    let permissions = json!({
        "computer:window-discovery": "allow",
        (format!("computer:{}", SimulatedComputerBackend::BUNDLE_ID)): "allow",
        "computer-action:sim-prepared-0": "allow",
        "computer-action:sim-prepared-1": "allow",
    });
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".to_string()),
            cwd: Some(project.path().to_string_lossy().into_owned()),
            extra: json!({
                "base_url": base_url,
                "model": "clark-code:kimi_k3",
                "memories": false,
                "research": false,
                "sandbox_mode": "disabled",
                "orchestration": {"enabled": false},
                "computer_use_enabled": true,
                "computer_use_backend": "simulated",
                "permissions": permissions,
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(project.path().to_string_lossy().into_owned()),
            mode: Some("ask".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text(
                "Use computer control to type a greeting, open the example, and verify both.",
            ),
        )
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        let finished = matches!(event, AgentEvent::RunFinished { .. });
        events.push(event);
        if finished {
            break;
        }
    }

    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
    )));
    let completed_tools = events
        .iter()
        .filter(|event| match event {
            AgentEvent::ToolCall { call, .. } => call
                .tool_name
                .as_deref()
                .is_some_and(|name| name == "tool_search" || name.starts_with("computer_")),
            _ => false,
        })
        .count();
    assert_eq!(completed_tools, 9, "tool events: {events:?}");
    let final_text = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(final_text.contains("Opened example"));

    let requests = captured.await.unwrap();
    assert_eq!(requests.len(), 10);
    assert!(requests
        .iter()
        .all(|request| request.model() == Some("clark-code:kimi_k3")));
    let first_observation_follow_up = &requests[3];
    let image_urls = first_observation_follow_up.image_urls();
    assert_eq!(image_urls.len(), 1);
    assert!(image_urls[0].starts_with("data:image/png;base64,iVBORw0KGgo"));
    assert!(requests[6]
        .tool_results()
        .iter()
        .any(|result| result.contains("hello from Kimi K3")));
    assert!(requests[6]
        .tool_results()
        .iter()
        .any(|result| result.contains("Accessibility diff from sim-observation-0")));
    assert!(requests[9]
        .tool_results()
        .iter()
        .any(|result| result.contains("Opened example")));
    assert!(requests[9]
        .tool_results()
        .iter()
        .any(|result| result.contains("Accessibility diff from sim-observation-1")));
}

fn merge(base: &serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut merged = base.as_object().unwrap().clone();
    merged.extend(extra.as_object().unwrap().clone());
    serde_json::Value::Object(merged)
}
