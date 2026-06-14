//! Real-gateway scenario matrix. Drives the production `ClarkProvider` (now on
//! the resumable SSE event channel) through a spread of agent states against a
//! live Clark gateway, plus exercises real SSE replay/resume — to surface
//! backend or transport bugs across many shapes of run, not just the happy path.
//!
//! ```sh
//! CLARK_WS_URL=ws://localhost:8400/ws CLARK_AUTH_TOKEN=test-ui-local \
//!   cargo test -p provider-clark --test clark_matrix -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::time::Duration;

use agent_core::domain::{AgentEvent, ContentBlock, RunStatus};
use agent_core::ids::RunId;
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_clark::ClarkProvider;
use serde_json::Value;

fn env() -> Option<(String, Option<String>)> {
    let endpoint = std::env::var("CLARK_WS_URL").ok()?;
    Some((endpoint, std::env::var("CLARK_AUTH_TOKEN").ok()))
}

fn http_base(ws: &str) -> String {
    ws.strip_prefix("wss://")
        .map(|r| format!("https://{}", r.split('/').next().unwrap_or(r)))
        .or_else(|| {
            ws.strip_prefix("ws://")
                .map(|r| format!("http://{}", r.split('/').next().unwrap_or(r)))
        })
        .unwrap_or_else(|| ws.to_string())
}

#[derive(Default, Debug)]
struct Summary {
    finished: bool,
    status: Option<RunStatus>,
    tool_calls: u32,
    plans: u32,
    artifacts: u32,
    text_len: usize,
    kinds: BTreeMap<String, u32>,
}

/// Drive one prompt to its terminal state. If `cancel_after` is set, cancel the
/// run at that point to exercise the cancel path.
async fn drive(
    endpoint: &str,
    token: Option<&str>,
    query: &str,
    budget: Duration,
    cancel_after: Option<Duration>,
) -> (String, Summary) {
    let mut provider = ClarkProvider::new();
    provider
        .connect(ProviderConfig {
            endpoint: Some(endpoint.to_string()),
            auth_token: token.map(str::to_string),
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

    let mut s = Summary::default();
    let mut cancel_at = cancel_after.map(|d| Box::pin(tokio::time::sleep(d)));
    let deadline = tokio::time::Instant::now() + budget;

    loop {
        tokio::select! {
            _ = async { cancel_at.as_mut().unwrap().await }, if cancel_at.is_some() => {
                cancel_at = None;
                let _ = provider.cancel(&session.id, &RunId::new("x")).await;
            }
            ev = tokio::time::timeout_at(deadline, stream.next()) => {
                match ev {
                    Err(_) => break, // budget exceeded — freeze/no terminal
                    Ok(None) => break,
                    Ok(Some(ev)) => {
                        *s.kinds.entry(variant(&ev)).or_default() += 1;
                        match ev {
                            AgentEvent::ToolCall { .. } => s.tool_calls += 1,
                            AgentEvent::Plan { .. } => s.plans += 1,
                            AgentEvent::Artifact { .. } => s.artifacts += 1,
                            AgentEvent::MessageChunk { delta: ContentBlock::Text { text }, .. } => {
                                s.text_len += text.len();
                            }
                            AgentEvent::RunFinished { outcome, .. } => {
                                s.finished = true;
                                s.status = Some(outcome.status);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    (session.id.0, s)
}

fn variant(ev: &AgentEvent) -> String {
    match ev {
        AgentEvent::RunStarted { .. } => "RunStarted",
        AgentEvent::RunFinished { .. } => "RunFinished",
        AgentEvent::MessageChunk { .. } => "MessageChunk",
        AgentEvent::ToolCall { .. } => "ToolCall",
        AgentEvent::ToolCallUpdate { .. } => "ToolCallUpdate",
        AgentEvent::Plan { .. } => "Plan",
        AgentEvent::Surface { .. } => "Surface",
        AgentEvent::Artifact { .. } => "Artifact",
        AgentEvent::PermissionRequest { .. } => "PermissionRequest",
        _ => "other",
    }
    .to_string()
}

/// Raw SSE replay probe: count `conversation_event` frames the gateway replays
/// from `after_seq`, returning (count, min_seq, max_seq).
async fn sse_replay(
    base: &str,
    token: Option<&str>,
    conv: &str,
    after_seq: u64,
) -> (u32, u64, u64) {
    let url = format!("{base}/api/conversations/{conv}/events/stream?after_seq={after_seq}");
    let mut req = reqwest::Client::new()
        .get(&url)
        .header("Accept", "text/event-stream");
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let resp = req.send().await.expect("sse connect");
    assert!(resp.status().is_success(), "sse status {}", resp.status());

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let (mut count, mut min, mut max) = (0u32, u64::MAX, 0u64);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    'outer: while let Ok(Some(chunk)) = tokio::time::timeout_at(deadline, stream.next())
        .await
        .map_err(|_| ())
    {
        let Ok(bytes) = chunk else { break };
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(i) = buf.find("\n\n") {
            let frame: String = buf.drain(..i + 2).collect();
            if frame.contains("event: conversation_event") {
                if let Some(line) = frame.lines().find(|l| l.starts_with("data:")) {
                    if let Ok(v) = serde_json::from_str::<Value>(line[5..].trim()) {
                        if let Some(seq) = v.get("seq").and_then(Value::as_u64) {
                            count += 1;
                            min = min.min(seq);
                            max = max.max(seq);
                            if count > 5_000 {
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
    }
    (count, if count == 0 { 0 } else { min }, max)
}

#[tokio::test]
#[ignore = "requires a reachable Clark gateway (set CLARK_WS_URL)"]
async fn matrix_pong_files_cancel_and_resume() {
    let Some((endpoint, token)) = env() else {
        eprintln!("skipping: set CLARK_WS_URL");
        return;
    };
    let tok = token.as_deref();

    // 1) Fast turn — exercises the bounce/final-content path.
    let (_c, pong) = drive(
        &endpoint,
        tok,
        "Reply with exactly one word: pong",
        Duration::from_secs(60),
        None,
    )
    .await;
    println!("[pong]   {pong:?}");
    assert!(
        pong.finished,
        "pong did not reach a terminal state (froze?)"
    );
    assert_eq!(pong.status, Some(RunStatus::Done));

    // 2) Multi-step file work — plan + several tool calls stream over SSE.
    let (conv, files) = drive(
        &endpoint,
        tok,
        "Create /home/user/workspace/m.txt with three lines, read it back, append one line, then list the workspace.",
        Duration::from_secs(150),
        None,
    )
    .await;
    println!("[files]  conv={conv} {files:?}");
    assert!(files.finished, "file run froze before RunFinished");
    assert_eq!(files.status, Some(RunStatus::Done));
    assert!(files.tool_calls > 0, "expected tool calls over SSE");

    // 3) Cancel mid-run — the run must still reach a terminal state quickly.
    let (_c, cancelled) = drive(
        &endpoint,
        tok,
        "Write a detailed 10-paragraph essay about the history of the bicycle, slowly.",
        Duration::from_secs(60),
        Some(Duration::from_secs(3)),
    )
    .await;
    println!("[cancel] {cancelled:?}");
    assert!(
        cancelled.finished,
        "cancel did not yield a terminal state (froze?)"
    );

    // 4) Real SSE replay/resume against the live gateway+DB for the file run.
    let base = http_base(&endpoint);
    let (all, min0, max) = sse_replay(&base, tok, &conv, 0).await;
    println!("[resume] replay@0 -> count={all} min={min0} max={max}");
    assert!(
        all > 0 && max > 0,
        "expected the conversation to replay from 0"
    );
    if max > 2 {
        let (tail, min_tail, _max) = sse_replay(&base, tok, &conv, max - 2).await;
        println!("[resume] replay@{} -> count={tail} min={min_tail}", max - 2);
        assert!(
            tail < all,
            "resuming from a later cursor must replay fewer events"
        );
        assert!(
            min_tail > max - 2,
            "resume must only deliver events after the cursor"
        );
    }
}
