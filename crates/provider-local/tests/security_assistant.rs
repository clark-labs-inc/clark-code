//! The default Security skill uses the normal writable agent loop, without Git
//! enrollment, a scan contract, or the explicit scan model override.
mod support;

use agent_core::domain::{AgentEvent, RunStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde_json::json;
use support::{final_body, scripted_model, tool_call_body};

#[tokio::test]
async fn security_assistant_can_implement_without_a_repository_or_scan() {
    let (base_url, captured) = scripted_model(vec![
        tool_call_body(
            "write",
            "write_file",
            json!({"path":"answer.txt", "content":"verified"}),
        ),
        tool_call_body("read", "read_file", json!({"path":"answer.txt"})),
        final_body("Wrote and verified answer.txt."),
    ])
    .await;
    let project = tempfile::tempdir().unwrap();
    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url":base_url, "model":"ordinary-model", "memories":false,
                "sandbox_mode":"disabled", "permissions":{"write_file":"allow"},
                "skill_model_overrides":{"security":{"model":"scan-model","reasoning_effort":"max"}}
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(project.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("$security:assistant Write verified to answer.txt and read it back."),
        )
        .await
        .unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while let Some(event) = stream.next().await {
            if let AgentEvent::RunFinished { outcome, .. } = event {
                return outcome;
            }
        }
        panic!("missing terminal outcome");
    })
    .await
    .expect("scripted turn completes");
    assert_eq!(outcome.status, RunStatus::Done);
    assert_eq!(
        std::fs::read_to_string(project.path().join("answer.txt")).unwrap(),
        "verified"
    );
    assert!(!project.path().join(".git").exists());
    assert!(!project.path().join(".agent/security-scans").exists());
    let requests = captured.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests
        .iter()
        .all(|request| request.model() == Some("ordinary-model")));
    assert!(requests
        .last()
        .unwrap()
        .tool_results()
        .iter()
        .any(|result| result.contains("verified")));
}
