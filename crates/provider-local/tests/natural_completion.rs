//! Production-shaped regression for the October 20 non-tool final response.
//! No private incident text or hosted provider is used.

mod support;

use agent_core::domain::{AgentEvent, ContentBlock, MessagePhase, RunStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde_json::json;

async fn run(
    provider: &mut provider_local::LocalAgentProvider,
    session: &agent_core::ids::SessionId,
    prompt: &str,
) -> Vec<AgentEvent> {
    let mut stream = provider
        .prompt(session, PromptInput::text(prompt))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            let finished = matches!(event, AgentEvent::RunFinished { .. });
            events.push(event);
            if finished {
                return events;
            }
        }
        panic!("run ended without a terminal receipt");
    })
    .await
    .expect("natural completion did not finish")
}

#[tokio::test]
async fn post_work_answer_finishes_once_and_survives_a_follow_up() {
    const ANSWER: &str = "The evidence was read and the result is ready.";
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("evidence.txt"),
        "synthetic verification value",
    )
    .unwrap();
    let (base_url, server) = support::scripted_model(vec![
        support::tool_call_body(
            "read-evidence",
            "read_file",
            json!({"path": "evidence.txt"}),
        ),
        support::plain_body(ANSWER),
        support::plain_body("The prior answer and evidence are still available."),
    ])
    .await;
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            cwd: Some(root.path().to_string_lossy().into_owned()),
            auth_token: Some("offline-fixture".into()),
            extra: json!({
                "base_url": base_url, "model": "offline-fixture",
                "memories": false, "sandbox_mode": "disabled",
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(root.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let events = run(&mut provider, &session.id, "Read the evidence and answer.").await;
    let outcome = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::RunFinished { outcome, .. } => Some(outcome),
            _ => None,
        })
        .unwrap();
    assert_eq!(outcome.status, RunStatus::Done, "{outcome:?}");
    assert_eq!(outcome.failure_kind, None);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentEvent::ToolCall { .. }))
            .count(),
        1,
    );
    let text = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, ANSWER, "the final answer must appear exactly once");
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::MessagePhase {
            phase: MessagePhase::FinalAnswer,
            ..
        }
    )));
    assert!(!events.iter().any(|event| matches!(event,
        AgentEvent::Trace { source, .. } if source == "model_tool_protocol_recovery"
    )));

    let follow_up = run(
        &mut provider,
        &session.id,
        "Continue from the saved evidence.",
    )
    .await;
    assert!(follow_up.iter().any(|event| matches!(event,
        AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
    )));
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3, "no hidden final-answer repair requests");
    assert!(requests[2]
        .messages_for_role("assistant")
        .iter()
        .any(|text| text == ANSWER));
    assert!(requests[2]
        .tool_results()
        .iter()
        .any(|text| text.contains("synthetic verification value")));
    for request in requests {
        let body = request.body();
        assert_eq!(body["tool_choice"], "auto");
        assert!(!body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| { tool["function"]["name"] == "final_answer" }));
    }
}

#[tokio::test]
async fn generated_files_wait_for_effect_verification_before_publication() {
    let root = tempfile::tempdir().unwrap();
    let (base_url, server) = support::scripted_model(vec![
        support::tool_call_body(
            "mutation",
            "bash",
            json!({
                "command": "printf verified > result.txt",
                "sandbox_permissions": "require_escalated",
                "effect": "create", "effect_target": "result.txt",
            }),
        ),
        support::tool_call_body(
            "premature-files",
            "deliver_files",
            json!({"files": ["result.txt"]}),
        ),
        support::tool_call_body(
            "readback",
            "bash",
            json!({
                "command": "cat result.txt", "effect": "none",
            }),
        ),
        support::tool_call_body(
            "verify",
            "verify_effect",
            json!({
                "effect_id": "mutation", "status": "verified",
                "evidence": "Independent canonical read-back returned the expected value.",
                "expected": "verified", "observed": "verified",
            }),
        ),
        support::tool_call_body(
            "ready-files",
            "deliver_files",
            json!({"files": ["result.txt"]}),
        ),
        support::plain_body("The verified result file is ready."),
    ])
    .await;
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            cwd: Some(root.path().to_string_lossy().into_owned()),
            auth_token: Some("offline-fixture".into()),
            extra: json!({
                "base_url": base_url, "model": "offline-fixture",
                "memories": false, "sandbox_mode": "disabled",
                "permissions": {"bash": "allow"},
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(root.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let events = run(
        &mut provider,
        &session.id,
        "Create, verify, and deliver the result.",
    )
    .await;
    assert!(events.iter().any(|event| matches!(event,
        AgentEvent::RunFinished { outcome, .. } if outcome.status == RunStatus::Done
    )));
    let artifacts = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Artifact { artifact, .. } => Some(artifact),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        artifacts.len(),
        1,
        "the premature file delivery must be rejected"
    );
    assert_eq!(
        artifacts[0].tool_call.as_ref().unwrap().as_str(),
        "ready-files"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 6);
    assert!(requests[2]
        .tool_results()
        .iter()
        .any(|text| { text.contains("cannot deliver generated files") }));
    assert!(requests[5]
        .tool_results()
        .iter()
        .any(|text| text.contains("Files delivered")));
}
