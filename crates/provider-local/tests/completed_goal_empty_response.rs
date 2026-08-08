//! A typed goal completion remains authoritative when the post-tool response is empty.

mod support;

use agent_core::domain::{AgentEvent, GoalStatus, RunStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde_json::json;

use support::{plain_body, scripted_model, tool_call_body};

fn empty_sse_body() -> String {
    "data: [DONE]\n\n".to_string()
}

#[tokio::test]
async fn completed_goal_is_not_reclassified_as_an_empty_response_failure() {
    let (base_url, captured) = scripted_model(vec![
        tool_call_body(
            "discover-goals",
            "tool_search",
            json!({"query": "goal autonomy"}),
        ),
        tool_call_body(
            "create-goal",
            "create_goal",
            json!({"objective": "finish the requested work"}),
        ),
        plain_body("Goal created — starting."),
        tool_call_body(
            "complete-goal",
            "update_goal",
            json!({"status": "complete"}),
        ),
        // `agent-loop` retries a zero-output transport response once before
        // returning it to the provider engine.
        empty_sse_body(),
        empty_sse_body(),
    ])
    .await;

    let project = tempfile::tempdir().expect("create project");
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": base_url,
                "model": "scripted-goal-model",
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .expect("connect provider");
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(project.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .expect("create session");

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Work until the task is complete — make it a goal."),
        )
        .await
        .expect("start goal run");
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
        AgentEvent::GoalUpdated { goal, .. } if goal.status == GoalStatus::Complete
    )));
    let outcome = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::RunFinished { outcome, .. } => Some(outcome),
            _ => None,
        })
        .expect("run finished");
    assert_eq!(outcome.status, RunStatus::Done, "events: {events:#?}");
    assert_eq!(outcome.failure_kind, None);
    assert_eq!(outcome.error, None);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::Error { .. })),
        "completed goal must not emit an error: {events:#?}"
    );

    assert_eq!(captured.await.expect("scripted model").len(), 6);
}
