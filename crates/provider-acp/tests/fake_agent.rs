//! End-to-end test of the ACP adapter against an in-memory fake agent.
//!
//! Drives a full turn over a `duplex` pair: initialize → session/new →
//! session/prompt with streamed updates, a permission round-trip, and the final
//! response. No external process required, so it runs in CI.

use agent_core::codec::jsonrpc::{RpcId, RpcMessage};
use agent_core::domain::{AgentEvent, RunStatus, ToolStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, SessionOptions};
use futures::StreamExt;
use provider_acp::AcpProvider;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, msg: &RpcMessage) {
    let line = msg.to_line().unwrap();
    w.write_all(line.as_bytes()).await.unwrap();
    w.flush().await.unwrap();
}

async fn read_msg<R: AsyncBufRead + Unpin>(lines: &mut tokio::io::Lines<R>) -> RpcMessage {
    loop {
        let line = lines
            .next_line()
            .await
            .unwrap()
            .expect("agent: unexpected EOF");
        if line.trim().is_empty() {
            continue;
        }
        return RpcMessage::from_line(&line).unwrap();
    }
}

fn update(body: Value) -> RpcMessage {
    RpcMessage::notification(
        "session/update",
        json!({ "sessionId": "sess-1", "update": body }),
    )
}

/// The scripted fake agent. Sequential because the client processes the stream
/// in order.
async fn fake_agent(server: tokio::io::DuplexStream) {
    let (r, mut w) = tokio::io::split(server);
    let mut lines = BufReader::new(r).lines();

    // 1) initialize
    let init = read_msg(&mut lines).await;
    write_msg(
        &mut w,
        &RpcMessage::response_ok(
            init.id.unwrap(),
            json!({ "protocolVersion": 1, "agentCapabilities": { "loadSession": true } }),
        ),
    )
    .await;

    // 2) session/new
    let sn = read_msg(&mut lines).await;
    write_msg(
        &mut w,
        &RpcMessage::response_ok(sn.id.unwrap(), json!({ "sessionId": "sess-1" })),
    )
    .await;

    // 3) session/prompt — stream a turn
    let prompt = read_msg(&mut lines).await;
    write_msg(
        &mut w,
        &update(json!({ "type": "agent_message_chunk", "content": { "type": "text", "text": "Reading " } })),
    )
    .await;
    write_msg(
        &mut w,
        &update(json!({
            "type": "tool_call", "toolCallId": "t1", "title": "Read main.rs",
            "kind": "read_file", "status": "in_progress",
            "locations": [{ "path": "/x/main.rs", "line": 1 }]
        })),
    )
    .await;
    write_msg(
        &mut w,
        &update(json!({
            "type": "tool_call_update", "toolCallId": "t1", "status": "completed",
            "content": [{ "type": "content", "content": { "type": "text", "text": "fn main(){}" } }]
        })),
    )
    .await;
    write_msg(
        &mut w,
        &update(
            json!({ "type": "plan", "entries": [{ "content": "Inspect", "status": "completed" }] }),
        ),
    )
    .await;

    // permission request (agent → client)
    write_msg(
        &mut w,
        &RpcMessage::request(
            RpcId::Num(9001),
            "session/request_permission",
            json!({
                "sessionId": "sess-1",
                "toolCall": { "toolCallId": "t1", "title": "Run cargo build" },
                "options": [
                    { "optionId": "allow", "name": "Allow", "kind": "allow_once" },
                    { "optionId": "reject", "name": "Reject", "kind": "reject_once" }
                ]
            }),
        ),
    )
    .await;

    // wait for the client's permission decision
    let decision = read_msg(&mut lines).await;
    let opt = decision
        .result
        .as_ref()
        .and_then(|r| r.get("outcome"))
        .and_then(|o| o.get("optionId"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_eq!(opt, "allow", "client should have selected the allow option");

    // finish the turn
    write_msg(
        &mut w,
        &update(json!({ "type": "agent_message_chunk", "content": { "type": "text", "text": "done." } })),
    )
    .await;
    write_msg(
        &mut w,
        &RpcMessage::response_ok(prompt.id.unwrap(), json!({ "stopReason": "completion" })),
    )
    .await;
}

#[tokio::test]
async fn full_turn_with_permission_round_trip() {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (cr, cw) = tokio::io::split(client);
    tokio::spawn(fake_agent(server));

    let mut provider = AcpProvider::new();
    provider
        .setup(Box::new(cr), Box::new(cw))
        .await
        .expect("setup/initialize");

    // capabilities reflect the agent's reported loadSession=true
    assert!(provider.capabilities().load_session);

    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("session/new");
    assert_eq!(session.id.as_str(), "sess-1");

    let mut stream = provider
        .prompt(&session.id, PromptInput::text("read main.rs"))
        .await
        .expect("prompt");

    let mut events = Vec::new();
    let mut saw_permission = false;

    let collect = async {
        while let Some(ev) = stream.next().await {
            if let AgentEvent::PermissionRequest { request } = &ev {
                saw_permission = true;
                let option = request.options[0].id.clone();
                provider
                    .respond(
                        &session.id,
                        ClientResponse::Permission {
                            request: request.id.clone(),
                            option,
                            feedback: None,
                        },
                    )
                    .await
                    .expect("respond");
            }
            let finished = matches!(ev, AgentEvent::RunFinished { .. });
            events.push(ev);
            if finished {
                break;
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), collect)
        .await
        .expect("turn timed out");

    assert!(saw_permission, "should have received a permission request");

    // Fold the event stream and assert the projected truth.
    let snap = agent_core::reduce_all(&events);

    let agent_text: String = snap
        .timeline
        .iter()
        .filter_map(|t| match t {
            agent_core::TimelineItem::Message {
                role: agent_core::Role::Agent,
                blocks,
                ..
            } => Some(
                blocks
                    .iter()
                    .filter_map(|b| match b {
                        agent_core::ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect();
    assert!(agent_text.contains("Reading"), "agent text: {agent_text:?}");
    assert!(agent_text.contains("done."), "agent text: {agent_text:?}");

    let tc = snap
        .tool_calls
        .get(&agent_core::ids::ToolCallId::new("t1"))
        .expect("tool call t1");
    assert_eq!(tc.status, ToolStatus::Completed);
    assert_eq!(
        tc.content,
        vec![agent_core::ContentBlock::text("fn main(){}")]
    );

    assert!(snap.plan.is_some(), "plan should be present");
    assert!(
        snap.pending_permission.is_none(),
        "permission gate cleared on run finish"
    );

    let run = snap.runs.values().next().expect("a run");
    assert_eq!(run.status, RunStatus::Done);
}

#[tokio::test]
async fn set_mode_sends_session_set_mode_request() {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (cr, cw) = tokio::io::split(client);

    tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(server);
        let mut lines = BufReader::new(r).lines();

        let init = read_msg(&mut lines).await;
        write_msg(
            &mut w,
            &RpcMessage::response_ok(
                init.id.unwrap(),
                json!({ "protocolVersion": 1, "agentCapabilities": {} }),
            ),
        )
        .await;

        let sn = read_msg(&mut lines).await;
        write_msg(
            &mut w,
            &RpcMessage::response_ok(sn.id.unwrap(), json!({ "sessionId": "sess-1" })),
        )
        .await;

        let sm = read_msg(&mut lines).await;
        assert_eq!(sm.method.as_deref(), Some("session/set_mode"));
        assert_eq!(
            sm.params,
            Some(json!({ "sessionId": "sess-1", "modeId": "plan" }))
        );
        write_msg(&mut w, &RpcMessage::response_ok(sm.id.unwrap(), json!({}))).await;
    });

    let mut provider = AcpProvider::new();
    provider
        .setup(Box::new(cr), Box::new(cw))
        .await
        .expect("setup/initialize");

    let session = provider
        .new_session(SessionOptions::default())
        .await
        .expect("session/new");

    provider
        .set_mode(&session.id, "plan".to_string())
        .await
        .expect("set_mode never errors");
}

#[tokio::test]
async fn set_mode_before_connect_is_a_harmless_no_op() {
    let mut provider = AcpProvider::new();
    let session = agent_core::ids::SessionId::new("s1");
    provider
        .set_mode(&session, "plan".to_string())
        .await
        .expect("set_mode never errors even when not connected");
}
