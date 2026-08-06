//! Dev-only WebSocket bridge.
//!
//! Wraps the real `agent_core::Provider` implementations + projection and speaks
//! a tiny JSON protocol to a browser, so the Vite app can drive **real** Clark /
//! ACP turns without the Tauri host. Used for headless UI testing and video
//! capture. Not shipped in the app.
//!
//! Browser → server: `{id?, cmd, ...}` where cmd ∈ list_providers |
//!   open_session | prompt | cancel | respond.
//! Server → browser: `{type:"providers"|"session"|"snapshot"|"ok"|"error", ...}`.

use std::sync::Arc;

use agent_core::{
    apply, AgentEvent, ClientResponse, ContentBlock, PendingUpload, PromptInput, Provider,
    ProviderCapabilities, ProviderConfig, Role, RunId, SessionId, SessionOptions, Snapshot,
};
use futures::{SinkExt, StreamExt};
use provider_acp::AcpProvider;
use provider_clark::ClarkProvider;
use provider_local::LocalAgentProvider;
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

type Sink = futures::stream::SplitSink<WebSocketStream<TcpStream>, Message>;
type SharedSink = Arc<Mutex<Sink>>;

#[derive(Default)]
struct Conn {
    provider: Option<Box<dyn Provider>>,
    snapshot: Snapshot,
}
type SharedConn = Arc<Mutex<Conn>>;

fn caps(streaming_only: bool) -> ProviderCapabilities {
    ProviderCapabilities {
        streaming: true,
        permissions: true,
        fs: !streaming_only,
        terminal: false,
        load_session: true,
        attachment_kinds: Vec::new(),
        modes: vec![],
        collaboration_modes: vec![],
    }
}

fn providers() -> Value {
    json!([
        { "id": "local", "label": "Clark Code", "capabilities": LocalAgentProvider::new().capabilities() },
        { "id": "clark", "label": "Clark", "capabilities": caps(true) },
    ])
}

fn make_provider(id: &str) -> Result<Box<dyn Provider>, String> {
    match id {
        "acp" => Ok(Box::new(AcpProvider::new())),
        "clark" => Ok(Box::new(ClarkProvider::new())),
        "local" => Ok(Box::new(LocalAgentProvider::new())),
        other => Err(format!("unknown provider: {other}")),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "devbridge=info,provider_local=info,provider_clark=info,provider_acp=info".into()
            }),
        )
        .init();

    let addr = std::env::var("DEVBRIDGE_ADDR").unwrap_or_else(|_| "127.0.0.1:7878".into());
    let listener = TcpListener::bind(&addr).await.expect("bind devbridge");
    tracing::info!("devbridge listening on ws://{addr}");

    while let Ok((stream, peer)) = listener.accept().await {
        tracing::info!(%peer, "client connected");
        tokio::spawn(handle(stream));
    }
}

async fn handle(stream: TcpStream) {
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "ws handshake failed");
            return;
        }
    };
    let (sink, mut read) = ws.split();
    let sink: SharedSink = Arc::new(Mutex::new(sink));
    let conn: SharedConn = Arc::new(Mutex::new(Conn::default()));

    while let Some(Ok(msg)) = read.next().await {
        if let Message::Text(text) = msg {
            if let Ok(cmd) = serde_json::from_str::<Value>(&text) {
                handle_cmd(cmd, &conn, &sink).await;
            }
        }
    }
}

async fn send(sink: &SharedSink, v: Value) {
    let _ = sink
        .lock()
        .await
        .send(Message::Text(v.to_string().into()))
        .await;
}

async fn handle_cmd(cmd: Value, conn: &SharedConn, sink: &SharedSink) {
    let id = cmd.get("id").cloned().unwrap_or(Value::Null);
    match cmd.get("cmd").and_then(Value::as_str) {
        Some("list_providers") => {
            send(
                sink,
                json!({ "type": "providers", "id": id, "providers": providers() }),
            )
            .await;
        }

        Some("open_session") => {
            let provider_id = cmd.get("provider").and_then(Value::as_str).unwrap_or("");
            let config: ProviderConfig =
                serde_json::from_value(cmd.get("config").cloned().unwrap_or(json!({})))
                    .unwrap_or_default();
            let mut provider = match make_provider(provider_id) {
                Ok(p) => p,
                Err(e) => {
                    return send(sink, json!({ "type": "error", "id": id, "message": e })).await
                }
            };
            if let Err(error) = provider.connect(config).await {
                return send(
                    sink,
                    json!({ "type": "error", "id": id, "message": error.to_string() }),
                )
                .await;
            }

            let request = cmd.get("request").cloned().unwrap_or(json!({}));
            let session = match request.get("kind").and_then(Value::as_str) {
                Some("new") => {
                    let options: SessionOptions = serde_json::from_value(
                        request.get("options").cloned().unwrap_or(json!({})),
                    )
                    .unwrap_or_default();
                    provider.new_session(options).await
                }
                Some("load") => {
                    let sid =
                        SessionId::new(request.get("id").and_then(Value::as_str).unwrap_or(""));
                    provider.load_session(sid).await
                }
                _ => return send(
                    sink,
                    json!({ "type": "error", "id": id, "message": "invalid session-open request" }),
                )
                .await,
            };

            match session {
                Ok(session) => {
                    let mut snapshot = Snapshot::new();
                    snapshot.session = Some(session.id.clone());
                    let mut c = conn.lock().await;
                    c.provider = Some(provider);
                    c.snapshot = snapshot.clone();
                    drop(c);
                    send(
                        sink,
                        json!({ "type": "session", "id": id, "session": session }),
                    )
                    .await;
                    send(sink, json!({ "type": "snapshot", "snapshot": snapshot })).await;
                }
                Err(e) => {
                    send(
                        sink,
                        json!({ "type": "error", "id": id, "message": e.to_string() }),
                    )
                    .await
                }
            }
        }

        Some("prompt") => {
            let session = SessionId::new(cmd.get("session").and_then(Value::as_str).unwrap_or(""));
            let blocks: Vec<ContentBlock> =
                serde_json::from_value(cmd.get("blocks").cloned().unwrap_or(json!([])))
                    .unwrap_or_default();
            let attachments: Vec<PendingUpload> =
                serde_json::from_value(cmd.get("attachments").cloned().unwrap_or(json!([])))
                    .unwrap_or_default();

            let stream = {
                let mut c = conn.lock().await;
                let input = PromptInput {
                    blocks: blocks.clone(),
                    attachments: attachments.clone(),
                };
                let validation = match c.provider.as_ref() {
                    Some(provider) => provider.validate_prompt(&session, &input).await,
                    None => Err(agent_core::Error::NotConnected),
                };
                if let Err(error) = validation {
                    return send(
                        sink,
                        json!({ "type": "error", "message": error.to_string() }),
                    )
                    .await;
                }
                // Echo the user's turn (text + attachment thumbnails/chips) so
                // the timeline shows what was sent, mirroring the Tauri host.
                for b in &blocks {
                    apply(
                        &mut c.snapshot,
                        &AgentEvent::MessageChunk {
                            run: RunId::new("user"),
                            role: Role::User,
                            delta: b.clone(),
                        },
                    );
                }
                for a in &attachments {
                    apply(
                        &mut c.snapshot,
                        &AgentEvent::MessageChunk {
                            run: RunId::new("user"),
                            role: Role::User,
                            delta: a.echo_block(),
                        },
                    );
                }
                let snapshot = c.snapshot.clone();
                let result = match c.provider.as_mut() {
                    Some(p) => p.prompt(&session, input).await,
                    None => Err(agent_core::Error::NotConnected),
                };
                drop(c);
                send(sink, json!({ "type": "snapshot", "snapshot": snapshot })).await;
                match result {
                    Ok(s) => s,
                    Err(e) => {
                        return send(sink, json!({ "type": "error", "message": e.to_string() }))
                            .await
                    }
                }
            };

            let mut stream = stream;
            let Some(first) = stream.next().await else {
                return send(
                    sink,
                    json!({ "type": "error", "id": id, "message": "prompt ended before it allocated a run" }),
                )
                .await;
            };
            let run_id = match &first {
                AgentEvent::RunStarted { run } => run.as_str().to_string(),
                _ => {
                    return send(
                        sink,
                        json!({ "type": "error", "id": id, "message": "prompt did not begin with a run identity" }),
                    )
                    .await
                }
            };
            let snapshot = {
                let mut c = conn.lock().await;
                apply(&mut c.snapshot, &first);
                c.snapshot.clone()
            };
            send(sink, json!({ "type": "snapshot", "snapshot": snapshot })).await;
            send(
                sink,
                json!({ "type": "prompt_receipt", "id": id, "runId": run_id }),
            )
            .await;

            let conn2 = conn.clone();
            let sink2 = sink.clone();
            tokio::spawn(async move {
                while let Some(ev) = stream.next().await {
                    let snapshot = {
                        let mut c = conn2.lock().await;
                        apply(&mut c.snapshot, &ev);
                        c.snapshot.clone()
                    };
                    send(&sink2, json!({ "type": "snapshot", "snapshot": snapshot })).await;
                }
            });
        }

        Some("cancel") => {
            let session = SessionId::new(cmd.get("session").and_then(Value::as_str).unwrap_or(""));
            let run = RunId::new(cmd.get("run").and_then(Value::as_str).unwrap_or(""));
            let mut c = conn.lock().await;
            if let Some(p) = c.provider.as_mut() {
                let _ = p.cancel(&session, &run).await;
            }
        }

        Some("respond") => {
            let session = SessionId::new(cmd.get("session").and_then(Value::as_str).unwrap_or(""));
            if let Ok(response) = serde_json::from_value::<ClientResponse>(
                cmd.get("response").cloned().unwrap_or(Value::Null),
            ) {
                let mut c = conn.lock().await;
                if let Some(p) = c.provider.as_mut() {
                    let _ = p.respond(&session, response).await;
                }
            }
        }

        _ => {}
    }
}
