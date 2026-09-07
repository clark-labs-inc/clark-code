//! End-to-end context-budget regressions through the public provider boundary.

mod support;

use agent_core::domain::{AgentEvent, RunStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde_json::json;

use support::{final_body, scripted_model, tool_call_body};

#[tokio::test]
async fn broad_grep_reports_complete_count_without_flooding_the_next_prompt() {
    let project = tempfile::tempdir().expect("create project");
    let source = (0..3_000)
        .map(|index| format!("author row {index}: {}\n", "x".repeat(100)))
        .collect::<String>();
    std::fs::write(project.path().join("submissions.jsonl"), source)
        .expect("write broad-search fixture");

    let (base_url, captured) = scripted_model(vec![
        tool_call_body(
            "broad-search",
            "grep",
            json!({"pattern": "author", "path": "."}),
        ),
        final_body("The broad search needs a narrower scope."),
    ])
    .await;

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-key".into()),
            extra: json!({
                "base_url": base_url,
                "model": "scripted-context-model",
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
        .prompt(&session.id, PromptInput::text("find every author"))
        .await
        .expect("start run");
    let mut outcome = None;
    while let Some(event) = stream.next().await {
        if let AgentEvent::RunFinished {
            outcome: result, ..
        } = event
        {
            outcome = Some(result);
            break;
        }
    }
    assert_eq!(outcome.expect("run finished").status, RunStatus::Done);

    let requests = captured.await.expect("scripted model");
    let results = requests[1].tool_results();
    let result = results.first().expect("grep result reaches model");
    assert!(result.len() <= 64 * 1024, "{} bytes", result.len());
    assert!(result.contains("3000 total matches"), "{result}");
    assert!(result.contains("Narrow `path`/`glob`"), "{result}");
    assert!(result.contains("`files_with_matches`/`count`"), "{result}");
}
