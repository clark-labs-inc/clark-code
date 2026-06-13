//! Live smoke test against the real `gemini --acp` agent, exercising the actual
//! `AcpProvider` production path (spawn → initialize → session/new → prompt).
//!
//! Ignored by default: needs the `gemini` CLI installed and authenticated, and
//! makes a real (billed, non-deterministic) model call. Run manually:
//!
//! ```sh
//! cargo test -p provider-acp --test live_gemini -- --ignored --nocapture
//! ```

use std::time::Duration;

use agent_core::domain::{AgentEvent, ContentBlock, PermissionOptionKind, Role};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_acp::AcpProvider;

#[tokio::test]
#[ignore = "requires gemini CLI + auth; makes a real model call"]
async fn live_gemini_says_pong() {
    let mut provider = AcpProvider::new();
    provider
        .connect(ProviderConfig {
            command: Some(vec!["gemini".into(), "--acp".into()]),
            cwd: Some("/tmp".into()),
            ..Default::default()
        })
        .await
        .expect("connect/initialize gemini --acp");

    let session = provider
        .new_session(SessionOptions {
            cwd: Some("/tmp".into()),
            ..Default::default()
        })
        .await
        .expect("session/new");

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Reply with exactly one word: pong"),
        )
        .await
        .expect("prompt");

    let mut agent_text = String::new();
    let collect = async {
        while let Some(ev) = stream.next().await {
            match &ev {
                AgentEvent::MessageChunk {
                    role: Role::Agent,
                    delta: ContentBlock::Text { text },
                    ..
                } => agent_text.push_str(text),
                AgentEvent::PermissionRequest { request } => {
                    let option = request
                        .options
                        .iter()
                        .find(|o| {
                            matches!(
                                o.kind,
                                PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                            )
                        })
                        .or_else(|| request.options.first())
                        .map(|o| o.id.clone())
                        .unwrap_or_default();
                    provider
                        .respond(
                            &session.id,
                            ClientResponse::Permission {
                                request: request.id.clone(),
                                option,
                            },
                        )
                        .await
                        .expect("respond permission");
                }
                AgentEvent::RunFinished { outcome, .. } => {
                    println!("run finished: {outcome:?}");
                    break;
                }
                other => println!("event: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), collect)
        .await
        .expect("live turn timed out");

    println!("agent text: {agent_text:?}");
    assert!(
        agent_text.to_lowercase().contains("pong"),
        "expected 'pong' in agent reply, got: {agent_text:?}"
    );
}
