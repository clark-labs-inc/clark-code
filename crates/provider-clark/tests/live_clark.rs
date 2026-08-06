//! Live smoke test against a running Clark gateway. Exercises the real
//! `ClarkProvider` production path: connect → new_session → prompt → streamed
//! events.
//!
//! Ignored by default and env-driven — set `CLARK_WS_URL` (e.g.
//! `ws://host:port/ws`) and, if the gateway requires it, `CLARK_AUTH_TOKEN`,
//! then run:
//!
//! ```sh
//! CLARK_WS_URL=ws://… CLARK_AUTH_TOKEN=… \
//!     cargo test -p provider-clark --test live_clark -- --ignored --nocapture
//! ```

use std::time::Duration;

use agent_core::domain::{AgentEvent, ContentBlock, Role};
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_clark::ClarkProvider;

#[tokio::test]
#[ignore = "requires a reachable Clark gateway (set CLARK_WS_URL); makes a real model call"]
async fn live_clark_says_pong() {
    let Ok(endpoint) = std::env::var("CLARK_WS_URL") else {
        eprintln!("skipping: set CLARK_WS_URL to point at a Clark gateway");
        return;
    };
    let auth_token = std::env::var("CLARK_AUTH_TOKEN").ok();

    let mut provider = ClarkProvider::new();
    provider
        .connect(ProviderConfig {
            endpoint: Some(endpoint),
            auth_token,
            ..Default::default()
        })
        .await
        .expect("connect to gateway");

    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("resume_session");

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text("Reply with exactly one word: pong"),
        )
        .await
        .expect("canonical conversation command");

    let mut agent_text = String::new();
    let collect = async {
        while let Some(ev) = stream.next().await {
            match &ev {
                AgentEvent::MessageChunk {
                    role: Role::Agent,
                    delta: ContentBlock::Text { text },
                    ..
                } => agent_text.push_str(text),
                AgentEvent::RunFinished { outcome, .. } => {
                    println!("run finished: {outcome:?}");
                    break;
                }
                other => println!("event: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(120), collect)
        .await
        .expect("live Clark turn timed out");

    println!("agent text: {agent_text:?}");
    assert!(
        agent_text.to_lowercase().contains("pong"),
        "expected 'pong', got: {agent_text:?}"
    );
}
