//! Throwaway diagnostic: drive the real `ClarkProvider` (which now consumes
//! events over the resumable SSE stream) through a MULTI-STEP run and print
//! every normalized `AgentEvent` with timing — to confirm a long run streams
//! plan/tool events and finishes instead of freezing.
//!
//! ```sh
//! CLARK_WS_URL=ws://localhost:8400/ws CLARK_AUTH_TOKEN=test-ui-local \
//!   cargo test -p provider-clark --test diag_clark -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use agent_core::domain::AgentEvent;
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_clark::ClarkProvider;

#[tokio::test]
#[ignore = "requires a reachable Clark gateway (set CLARK_WS_URL)"]
async fn diag_multistep_over_sse() {
    let Ok(endpoint) = std::env::var("CLARK_WS_URL") else {
        eprintln!("skipping: set CLARK_WS_URL");
        return;
    };
    let auth_token = std::env::var("CLARK_AUTH_TOKEN").ok();
    let query = std::env::var("CLARK_QUERY").unwrap_or_else(|_| {
        "Create /home/user/workspace/a.txt with three lines about cats. \
         Then read it back. Then append one more line. Then create b.txt \
         with a short poem. Finally list the workspace directory."
            .into()
    });

    let mut provider = ClarkProvider::new();
    provider
        .connect(ProviderConfig {
            endpoint: Some(endpoint),
            auth_token,
            ..Default::default()
        })
        .await
        .expect("connect");
    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("new_session");
    let mut stream = provider
        .prompt(&session.id, PromptInput::text(query))
        .await
        .expect("prompt");

    let start = Instant::now();
    let mut last = Instant::now();
    let mut tool_calls = 0u32;
    let mut plans = 0u32;
    let mut text_len = 0usize;

    let collect = async {
        while let Some(ev) = stream.next().await {
            let now = Instant::now();
            let gap = now.duration_since(last).as_millis();
            last = now;
            let label = match &ev {
                AgentEvent::RunStarted { .. } => "RunStarted".to_string(),
                AgentEvent::MessageChunk { delta, .. } => {
                    if let agent_core::domain::ContentBlock::Text { text } = delta {
                        text_len += text.len();
                    }
                    "MessageChunk".to_string()
                }
                AgentEvent::ToolCall { call, .. } => {
                    tool_calls += 1;
                    format!("ToolCall[{:?}] {}", call.kind, call.title)
                }
                AgentEvent::ToolCallUpdate { id, patch, .. } => {
                    format!("ToolCallUpdate {} -> {:?}", id.as_str(), patch.status)
                }
                AgentEvent::ExecutionChecklistUpdated { checklist, .. } => {
                    plans += 1;
                    format!("ExecutionChecklist({} steps)", checklist.steps.len())
                }
                AgentEvent::ProposedPlanUpdated { .. } => "ProposedPlan".to_string(),
                AgentEvent::GoalUpdated { .. } => "GoalUpdated".to_string(),
                AgentEvent::Surface { focus } => format!("Surface({:?})", focus.surface),
                AgentEvent::Artifact { artifact, .. } => {
                    format!("Artifact[{:?}] {}", artifact.kind, artifact.title)
                }
                AgentEvent::PermissionRequest { .. } => "PermissionRequest".to_string(),
                AgentEvent::RunFinished { outcome, .. } => {
                    println!(
                        "[{:>6}ms +{:>5}ms] RunFinished {:?}",
                        start.elapsed().as_millis(),
                        gap,
                        outcome.status
                    );
                    return true;
                }
                other => format!("{other:?}"),
            };
            println!(
                "[{:>6}ms +{:>5}ms] {label}",
                start.elapsed().as_millis(),
                gap
            );
        }
        false
    };

    let finished = tokio::time::timeout(Duration::from_secs(150), collect)
        .await
        .unwrap_or(false);

    println!("\n=== summary ===");
    println!("finished cleanly: {finished}");
    println!("tool_calls: {tool_calls}  plans: {plans}  agent_text_bytes: {text_len}");
    assert!(finished, "run did not reach RunFinished (froze?)");
    assert!(tool_calls > 0, "expected tool calls to stream over SSE");
}
