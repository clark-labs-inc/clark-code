//! Paid diagnostic for the OpenAI-compatible stream-to-desktop event boundary.
//!
//! Ignored by default. Run only with the explicit Clark live environment.

use agent_core::domain::{AgentEvent, ContentBlock, RunStatus};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_local::LocalAgentProvider;
use serde_json::json;

const SENTINEL: &str = "CLARK_STREAM_FIDELITY_SENTINEL_6621";

#[path = "live_clark_code/canonical.rs"]
mod canonical;
use canonical::canonical_message_text;

#[tokio::test]
#[ignore = "requires explicit live clark-code env; makes one real model call"]
async fn live_minimax_streamed_text_matches_canonical_final() {
    if std::env::var("CLARK_CODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping: set CLARK_CODE_LIVE=1");
        return;
    }
    let api_key = std::env::var("CLARK_CODE_API_KEY").expect("CLARK_CODE_API_KEY");
    let base_url = std::env::var("CLARK_CODE_BASE_URL")
        .unwrap_or_else(|_| "https://api.clarkslabs.com/v1".to_string());
    let model = std::env::var("CLARK_CODE_MODEL").unwrap_or_else(|_| "clark-code:free".to_string());
    assert_eq!(model, "clark-code:free");

    let project = tempfile::tempdir().expect("temporary project");
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(api_key),
            extra: json!({
                "base_url": base_url,
                "model": model,
                "cwd": project.path(),
                "temperature": 0.0,
                "max_iterations": 2,
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .expect("connect provider");
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(project.path().to_string_lossy().to_string()),
            mode: None,
            collaboration_mode: None,
            resume: None,
        })
        .await
        .expect("new session");
    let prompt = format!("Reply with exactly this token and no extra words: {SENTINEL}");
    let mut events = provider
        .prompt(&session.id, PromptInput::text(prompt))
        .await
        .expect("prompt");

    let mut streamed = String::new();
    let mut canonical = Vec::new();
    let mut status = None;
    while let Some(event) = events.next().await {
        match event {
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => streamed.push_str(&text),
            AgentEvent::Trace {
                source, payload, ..
            } if source == "clark_agent" => {
                if let Some(text) = canonical_message_text(&payload) {
                    canonical.push(text);
                }
            }
            AgentEvent::RunFinished { outcome, .. } => {
                status = Some(outcome.status);
                break;
            }
            _ => {}
        }
    }

    let final_text = canonical.last().expect("canonical assistant message");
    println!(
        "stream_fidelity streamed={streamed:?} canonical={final_text:?} credential_recorded=false"
    );
    assert_eq!(status, Some(RunStatus::Done));
    assert_eq!(streamed.trim(), SENTINEL);
    assert_eq!(final_text.trim(), SENTINEL);
    assert_eq!(streamed, *final_text);
}
