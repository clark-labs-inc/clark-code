//! `ClarkProvider` — the `agent_core::Provider` implementation over the Clark
//! gateway WebSocket. Clean-room, built from the observed wire contract.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_core::domain::{AgentEvent, ContentBlock, RunOutcome, RunStatus};
use agent_core::error::{Error, Result};
use agent_core::ids::{ProviderId, RunId, SessionId};
use agent_core::provider::{
    ClientResponse, EventStream, PromptInput, Provider, ProviderCapabilities, ProviderConfig,
    Session, SessionOptions,
};
use async_channel::Sender;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::Mutex;

use crate::translate;
use crate::transport::ClarkSocket;

/// Wire protocol version negotiated in `resume_session` (observed: server
/// rejects 1, requires 2).
const PROTOCOL_VERSION: u32 = 2;

#[derive(Default)]
struct EngineState {
    run_sender: Option<Sender<AgentEvent>>,
    run_id: Option<RunId>,
    conversation_id: Option<String>,
    /// Whether any streamed assistant text was seen this run. If not, the final
    /// content from `run_completed` is surfaced as the answer (the gateway may
    /// `message_stream_bounce` and skip deltas when a turn resolves quickly).
    saw_agent_text: bool,
    /// HTTP origin of the gateway (derived from the WS endpoint) used to turn
    /// relative artifact/preview paths into openable URLs.
    http_base: Option<String>,
}

/// `ws://host/ws` → `http://host`, `wss://host/...` → `https://host`.
fn http_base_from_ws(endpoint: &str) -> Option<String> {
    let (scheme, rest) = if let Some(r) = endpoint.strip_prefix("wss://") {
        ("https", r)
    } else if let Some(r) = endpoint.strip_prefix("ws://") {
        ("http", r)
    } else {
        return None;
    };
    let host = rest.split('/').next().unwrap_or(rest);
    Some(format!("{scheme}://{host}"))
}

/// Make a relative artifact URL absolute against the gateway origin.
fn absolutize(uri: &str, base: Option<&str>) -> String {
    match base {
        Some(b) if uri.starts_with('/') => format!("{b}{uri}"),
        _ => uri.to_string(),
    }
}

/// Extract the final answer text from a `run_completed` event.
fn final_content(event: &Value) -> Option<String> {
    let data = event.get("data")?;
    let from_envelope = data
        .get("result_envelope")
        .and_then(|r| r.get("payload"))
        .and_then(|p| p.get("content"))
        .and_then(Value::as_str);
    from_envelope
        .or_else(|| data.get("summary").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

type Shared = Arc<Mutex<EngineState>>;

pub struct ClarkProvider {
    socket: Option<ClarkSocket>,
    shared: Shared,
    tier_id: String,
    run_counter: AtomicU64,
}

impl ClarkProvider {
    pub fn new() -> Self {
        Self {
            socket: None,
            shared: Arc::new(Mutex::new(EngineState::default())),
            tier_id: "clark".to_string(),
            run_counter: AtomicU64::new(0),
        }
    }

    fn socket(&self) -> Result<ClarkSocket> {
        self.socket.clone().ok_or(Error::NotConnected)
    }

    fn session_message(&self, conversation_id: &str) -> Value {
        json!({
            "protocol_version": PROTOCOL_VERSION,
            "conversation_id": conversation_id,
            "tier_id": self.tier_id,
        })
    }
}

impl Default for ClarkProvider {
    fn default() -> Self {
        Self::new()
    }
}

async fn emit(shared: &Shared, ev: AgentEvent) {
    let tx = { shared.lock().await.run_sender.clone() };
    if let Some(tx) = tx {
        let _ = tx.send(ev).await;
    }
}

/// Close out the active run's stream.
async fn finish_run(shared: &Shared) {
    let tx = {
        let mut s = shared.lock().await;
        s.run_id = None;
        s.saw_agent_text = false;
        s.run_sender.take()
    };
    if let Some(tx) = tx {
        tx.close();
    }
}

/// Single consumer of decoded server messages; routes the active conversation's
/// events into the active run.
async fn engine(mut rx: UnboundedReceiver<Value>, shared: Shared) {
    while let Some(msg) = rx.recv().await {
        match msg.get("type").and_then(Value::as_str) {
            Some("event") => {
                let Some(event) = msg.get("event") else {
                    continue;
                };
                let (run, conv, has_sender) = {
                    let s = shared.lock().await;
                    (
                        s.run_id.clone(),
                        s.conversation_id.clone(),
                        s.run_sender.is_some(),
                    )
                };
                if !has_sender {
                    continue;
                }
                // Route only the active conversation's events.
                if let Some(ec) = event.get("conversation_id").and_then(Value::as_str) {
                    if conv.as_deref() != Some(ec) {
                        continue;
                    }
                }
                let Some(run) = run else { continue };
                let event_type = event.get("type").and_then(Value::as_str);

                if event_type == Some("message_stream_delta") {
                    shared.lock().await.saw_agent_text = true;
                }
                // On completion with no streamed text, surface the final content.
                if matches!(event_type, Some("run_completed") | Some("turn_completed")) {
                    let saw = { shared.lock().await.saw_agent_text };
                    if !saw {
                        if let Some(content) = final_content(event) {
                            emit(
                                &shared,
                                AgentEvent::MessageChunk {
                                    run: run.clone(),
                                    role: agent_core::domain::Role::Agent,
                                    delta: ContentBlock::text(content),
                                },
                            )
                            .await;
                        }
                    }
                }

                if let Some(mut ev) = translate::event_to_agent(event, &run) {
                    // Make relative artifact/preview URLs openable.
                    if let AgentEvent::Artifact { artifact, .. } = &mut ev {
                        if let Some(uri) = &artifact.uri {
                            let base = { shared.lock().await.http_base.clone() };
                            artifact.uri = Some(absolutize(uri, base.as_deref()));
                        }
                    }
                    let finishing = matches!(ev, AgentEvent::RunFinished { .. });
                    emit(&shared, ev).await;
                    if finishing {
                        finish_run(&shared).await;
                    }
                }
            }
            Some("error") => {
                let message = msg
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("gateway error")
                    .to_string();
                let run = { shared.lock().await.run_id.clone() };
                if let Some(run) = run {
                    emit(
                        &shared,
                        AgentEvent::RunFinished {
                            run,
                            outcome: RunOutcome {
                                status: RunStatus::Failed,
                                stop_reason: None,
                                error: Some(message),
                            },
                        },
                    )
                    .await;
                    finish_run(&shared).await;
                }
            }
            _ => {} // "connected" and others: no-op
        }
    }
}

#[async_trait]
impl Provider for ClarkProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("clark")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            permissions: true,
            fs: false,
            terminal: false,
            load_session: true,
            modes: vec!["clark".into(), "clark_max".into()],
        }
    }

    async fn connect(&mut self, config: ProviderConfig) -> Result<()> {
        let endpoint = config
            .endpoint
            .clone()
            .ok_or_else(|| Error::Unsupported("Clark provider requires an `endpoint`".into()))?;
        if let Some(tier) = config.extra.get("tier_id").and_then(Value::as_str) {
            self.tier_id = tier.to_string();
        }
        self.shared.lock().await.http_base = http_base_from_ws(&endpoint);
        let (socket, rx) = ClarkSocket::connect(&endpoint, config.auth_token.as_deref()).await?;
        tokio::spawn(engine(rx, self.shared.clone()));
        self.socket = Some(socket);
        Ok(())
    }

    async fn new_session(&mut self, options: SessionOptions) -> Result<Session> {
        let socket = self.socket()?;
        let conversation_id = uuid::Uuid::new_v4().to_string();
        if let Some(mode) = &options.mode {
            self.tier_id = mode.clone();
        }
        socket
            .send(&json!({
                "type": "resume_session",
                "session": self.session_message(&conversation_id),
            }))
            .await?;
        self.shared.lock().await.conversation_id = Some(conversation_id.clone());
        Ok(Session {
            id: SessionId::new(conversation_id),
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: Some(self.tier_id.clone()),
        })
    }

    async fn load_session(&mut self, id: SessionId) -> Result<Session> {
        let socket = self.socket()?;
        socket
            .send(&json!({
                "type": "resume_session",
                "session": self.session_message(id.as_str()),
            }))
            .await?;
        self.shared.lock().await.conversation_id = Some(id.0.clone());
        Ok(Session {
            id,
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: Some(self.tier_id.clone()),
        })
    }

    async fn prompt(&mut self, session: &SessionId, input: PromptInput) -> Result<EventStream> {
        let socket = self.socket()?;
        let conversation_id = session.0.clone();
        let run = RunId::new(format!(
            "run-{}",
            self.run_counter.fetch_add(1, Ordering::SeqCst) + 1
        ));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        {
            let mut s = self.shared.lock().await;
            s.run_sender = Some(tx.clone());
            s.run_id = Some(run.clone());
            s.conversation_id = Some(conversation_id.clone());
            s.saw_agent_text = false;
        }
        let _ = tx.send(AgentEvent::RunStarted { run }).await;

        let text: String = input
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Ingest attachments as Clark attachment records (inline base64 data).
        let attachments: Vec<Value> = input
            .attachments
            .iter()
            .map(|a| {
                json!({
                    "filename": a.filename,
                    "content_type": a.content_type,
                    "data": a.data_base64,
                    "size_bytes": a.data_base64.len() / 4 * 3,
                })
            })
            .collect();

        socket
            .send(&json!({
                "type": "send_message",
                "session": self.session_message(&conversation_id),
                "text": text,
                "attachments": attachments,
                "client_request_id": uuid::Uuid::new_v4().to_string(),
            }))
            .await?;

        Ok(rx.boxed())
    }

    async fn cancel(&mut self, session: &SessionId, _run: &RunId) -> Result<()> {
        let socket = self.socket()?;
        socket
            .send(&json!({
                "type": "cancel",
                "session": self.session_message(session.as_str()),
            }))
            .await
    }

    async fn respond(&mut self, session: &SessionId, response: ClientResponse) -> Result<()> {
        match response {
            ClientResponse::Permission { option, .. } => {
                let socket = self.socket()?;
                let approved = option.contains("allow") || option == "approve";
                socket
                    .send(&json!({
                        "type": "confirm",
                        "session": self.session_message(session.as_str()),
                        "approved": approved,
                    }))
                    .await
            }
        }
    }
}
