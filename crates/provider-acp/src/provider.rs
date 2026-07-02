//! `AcpProvider` — the `agent_core::Provider` implementation over ACP.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_core::codec::jsonrpc::RpcId;
use agent_core::domain::{AgentEvent, RunOutcome, RunStatus};
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

use crate::method as rpc;
use crate::transport::{spawn_child, BoxRead, BoxWrite, Incoming, Peer};
use crate::{translate, ACP_PROTOCOL_VERSION};

/// Engine-shared state: routes inbound updates to the active run and parks the
/// permission request awaiting the user's decision.
#[derive(Default)]
struct EngineState {
    run_sender: Option<Sender<AgentEvent>>,
    run_id: Option<RunId>,
    pending_permission: Option<RpcId>,
}

type Shared = Arc<Mutex<EngineState>>;

pub struct AcpProvider {
    peer: Option<Peer>,
    child: Option<tokio::process::Child>,
    shared: Shared,
    agent_caps: Value,
    session_id: Option<String>,
    run_counter: AtomicU64,
}

impl AcpProvider {
    pub fn new() -> Self {
        Self {
            peer: None,
            child: None,
            shared: Arc::new(Mutex::new(EngineState::default())),
            agent_caps: Value::Null,
            session_id: None,
            run_counter: AtomicU64::new(0),
        }
    }

    /// Wire the transport + engine over arbitrary streams and run `initialize`.
    /// Public for integration tests that inject an in-memory agent.
    pub async fn setup(&mut self, reader: BoxRead, writer: BoxWrite) -> Result<()> {
        let (peer, inc_rx) = Peer::new(reader, writer);
        tokio::spawn(engine(inc_rx, self.shared.clone(), peer.clone()));

        let init = peer
            .request(
                rpc::INITIALIZE,
                json!({
                    "protocolVersion": ACP_PROTOCOL_VERSION,
                    "clientCapabilities": {
                        "fs": { "readTextFile": true, "writeTextFile": true },
                        "terminal": false
                    }
                }),
            )
            .await?;
        self.agent_caps = init
            .get("agentCapabilities")
            .cloned()
            .unwrap_or(Value::Null);
        self.peer = Some(peer);
        Ok(())
    }

    fn peer(&self) -> Result<Peer> {
        self.peer.clone().ok_or(Error::NotConnected)
    }
}

impl Default for AcpProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Emit an event to the active run without holding the lock across the await.
async fn emit(shared: &Shared, ev: AgentEvent) {
    let tx = { shared.lock().await.run_sender.clone() };
    if let Some(tx) = tx {
        let _ = tx.send(ev).await;
    }
}

/// The single ordered consumer of every inbound frame.
async fn engine(mut inc_rx: UnboundedReceiver<Incoming>, shared: Shared, peer: Peer) {
    while let Some(inc) = inc_rx.recv().await {
        match inc {
            Incoming::Closed => {
                peer.fail_all_pending().await;
                break;
            }
            Incoming::Response { id, result } => peer.resolve_response(id, result).await,
            Incoming::Notification { method, params } => {
                if method == rpc::SESSION_UPDATE {
                    let run = { shared.lock().await.run_id.clone() };
                    if let (Some(update), Some(run)) = (params.get("update"), run) {
                        if let Some(ev) = translate::update_to_event(update, &run) {
                            emit(&shared, ev).await;
                        }
                    }
                }
            }
            Incoming::Request { id, method, params } => {
                handle_server_request(&shared, &peer, id, &method, params).await;
            }
        }
    }
}

async fn handle_server_request(
    shared: &Shared,
    peer: &Peer,
    id: RpcId,
    method: &str,
    params: Value,
) {
    if method == rpc::SESSION_REQUEST_PERMISSION {
        let rpc_id = match &id {
            RpcId::Num(n) => n.to_string(),
            RpcId::Str(s) => s.clone(),
        };
        let req = translate::permission_request(&params, &rpc_id);
        {
            shared.lock().await.pending_permission = Some(id);
        }
        // Surface the gate; the response is sent later via Provider::respond.
        emit(shared, AgentEvent::PermissionRequest { request: req }).await;
    } else if method == rpc::FS_READ_TEXT_FILE {
        match read_text_file(&params).await {
            Ok(content) => {
                let _ = peer.respond_ok(id, json!({ "content": content })).await;
            }
            Err(e) => {
                let _ = peer.respond_err(id, -32000, &e).await;
            }
        }
    } else if method == rpc::FS_WRITE_TEXT_FILE {
        let path = params.get("path").and_then(Value::as_str).unwrap_or("");
        let content = params.get("content").and_then(Value::as_str).unwrap_or("");
        match tokio::fs::write(path, content).await {
            Ok(()) => {
                let _ = peer.respond_ok(id, Value::Null).await;
            }
            Err(e) => {
                let _ = peer.respond_err(id, -32000, &e.to_string()).await;
            }
        }
    } else {
        tracing::debug!(method, "acp: unsupported server method");
        let _ = peer
            .respond_err(id, -32601, "method not supported by client")
            .await;
    }
}

async fn read_text_file(params: &Value) -> std::result::Result<String, String> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or("missing path")?;
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| e.to_string())?;
    let line = params.get("line").and_then(Value::as_u64);
    let limit = params.get("limit").and_then(Value::as_u64);
    if line.is_none() && limit.is_none() {
        return Ok(content);
    }
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let lines: Vec<&str> = content.lines().collect();
    let end = match limit {
        Some(l) => (start + l as usize).min(lines.len()),
        None => lines.len(),
    };
    Ok(lines.get(start..end).unwrap_or(&[]).join("\n"))
}

#[async_trait]
impl Provider for AcpProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("acp")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        let load_session = self
            .agent_caps
            .get("loadSession")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        ProviderCapabilities {
            streaming: true,
            permissions: true,
            fs: true,
            terminal: false,
            load_session,
            modes: vec![],
        }
    }

    async fn connect(&mut self, config: ProviderConfig) -> Result<()> {
        let command = config
            .command
            .clone()
            .filter(|c| !c.is_empty())
            .ok_or_else(|| {
                Error::Unsupported("ACP provider requires a non-empty `command`".into())
            })?;
        let (reader, writer, child) = spawn_child(&command, config.cwd.as_deref())?;
        self.child = Some(child);
        self.setup(reader, writer).await
    }

    async fn new_session(&mut self, options: SessionOptions) -> Result<Session> {
        let peer = self.peer()?;
        let cwd = options
            .cwd
            .clone()
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
            })
            .unwrap_or_else(|| ".".into());
        let res = peer
            .request(rpc::SESSION_NEW, json!({ "cwd": cwd, "mcpServers": [] }))
            .await?;
        let sid = res
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("session/new missing sessionId".into()))?
            .to_string();
        self.session_id = Some(sid.clone());
        Ok(Session {
            id: SessionId::new(sid),
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: options.mode,
        })
    }

    async fn load_session(&mut self, id: SessionId) -> Result<Session> {
        let peer = self.peer()?;
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".into());
        peer.request(
            rpc::SESSION_LOAD,
            json!({ "sessionId": id.as_str(), "cwd": cwd, "mcpServers": [] }),
        )
        .await?;
        self.session_id = Some(id.0.clone());
        Ok(Session {
            id,
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: None,
        })
    }

    async fn prompt(&mut self, session: &SessionId, input: PromptInput) -> Result<EventStream> {
        let peer = self.peer()?;
        let run = RunId::new(format!(
            "run-{}",
            self.run_counter.fetch_add(1, Ordering::SeqCst) + 1
        ));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        {
            let mut s = self.shared.lock().await;
            s.run_sender = Some(tx.clone());
            s.run_id = Some(run.clone());
            s.pending_permission = None;
        }
        let _ = tx.send(AgentEvent::RunStarted { run: run.clone() }).await;

        let mut prompt: Vec<Value> = input
            .blocks
            .iter()
            .map(translate::content_block_to_acp)
            .collect();
        // Ingest attachments as ACP content blocks (image inline; others linked).
        prompt.extend(input.attachments.iter().map(translate::attachment_to_acp));
        let params = json!({ "sessionId": session.as_str(), "prompt": prompt });

        let shared = self.shared.clone();
        let run_done = run.clone();
        tokio::spawn(async move {
            let outcome = match peer.request(rpc::SESSION_PROMPT, params).await {
                Ok(v) => {
                    let stop = v
                        .get("stopReason")
                        .and_then(Value::as_str)
                        .unwrap_or("completion")
                        .to_string();
                    let status = if stop == "cancelled" {
                        RunStatus::Cancelled
                    } else {
                        RunStatus::Done
                    };
                    RunOutcome {
                        status,
                        stop_reason: Some(stop),
                        error: None,
                        usage: None,
                    }
                }
                Err(e) => RunOutcome {
                    status: RunStatus::Failed,
                    stop_reason: None,
                    error: Some(e.to_string()),
                    usage: None,
                },
            };
            let _ = tx
                .send(AgentEvent::RunFinished {
                    run: run_done,
                    outcome,
                })
                .await;
            tx.close();
            let mut s = shared.lock().await;
            s.run_sender = None;
            s.run_id = None;
        });

        Ok(rx.boxed())
    }

    async fn cancel(&mut self, session: &SessionId, _run: &RunId) -> Result<()> {
        let peer = self.peer()?;
        peer.notify(
            rpc::SESSION_CANCEL,
            json!({ "sessionId": session.as_str() }),
        )
        .await
    }

    async fn respond(&mut self, _session: &SessionId, response: ClientResponse) -> Result<()> {
        match response {
            ClientResponse::Permission { option, .. } => {
                let peer = self.peer()?;
                let id = { self.shared.lock().await.pending_permission.take() };
                let id =
                    id.ok_or_else(|| Error::Other("no pending permission to resolve".into()))?;
                peer.respond_ok(
                    id,
                    json!({ "outcome": { "outcome": "selected", "optionId": option } }),
                )
                .await
            }
        }
    }
}
