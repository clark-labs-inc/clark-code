//! JSON-RPC peer over a byte stream (line-delimited JSON).
//!
//! Generic over any `AsyncRead`/`AsyncWrite`, so it drives a spawned agent
//! process in production and an in-memory `duplex` pair in tests. A background
//! reader task correlates responses to in-flight requests and forwards inbound
//! requests/notifications to the engine via an mpsc channel.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use agent_core::codec::jsonrpc::{RpcError, RpcId, RpcKind, RpcMessage};
use agent_core::error::{Error, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

pub type BoxRead = Box<dyn AsyncRead + Unpin + Send>;
pub type BoxWrite = Box<dyn AsyncWrite + Unpin + Send>;

/// Every inbound frame, forwarded to the single engine consumer so that
/// notifications, peer requests, and responses to our own requests are all
/// processed in stream order (critical: a turn's `session/update`s must be
/// drained before its `session/prompt` response is resolved).
#[derive(Debug)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: RpcId,
        method: String,
        params: Value,
    },
    Response {
        id: RpcId,
        result: std::result::Result<Value, RpcError>,
    },
    /// The peer stream ended. Emitted once, after every preceding frame, so the
    /// engine can fail any still-pending requests in order.
    Closed,
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<std::result::Result<Value, RpcError>>>>>;

/// A cloneable handle to the JSON-RPC connection.
#[derive(Clone)]
pub struct Peer {
    writer: Arc<Mutex<BoxWrite>>,
    pending: Pending,
    next_id: Arc<AtomicI64>,
}

impl Peer {
    /// Wire up a peer over the given streams. Returns the peer plus the receiver
    /// of inbound requests/notifications.
    pub fn new(reader: BoxRead, writer: BoxWrite) -> (Self, mpsc::UnboundedReceiver<Incoming>) {
        let pending: Pending = Default::default();
        let (inc_tx, inc_rx) = mpsc::unbounded_channel();
        let peer = Peer {
            writer: Arc::new(Mutex::new(writer)),
            pending: pending.clone(),
            next_id: Arc::new(AtomicI64::new(1)),
        };

        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let msg = match RpcMessage::from_line(&line) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::warn!(error = %e, line = %line, "acp: dropping unparseable frame");
                                continue;
                            }
                        };
                        // Forward everything to the engine in arrival order.
                        match msg.classify() {
                            RpcKind::Response { id, result } => {
                                let _ = inc_tx.send(Incoming::Response { id, result });
                            }
                            RpcKind::Notification { method, params } => {
                                let _ = inc_tx.send(Incoming::Notification { method, params });
                            }
                            RpcKind::Request { id, method, params } => {
                                let _ = inc_tx.send(Incoming::Request { id, method, params });
                            }
                        }
                    }
                    Ok(None) => {
                        tracing::info!("acp: peer stream closed (EOF)");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "acp: read error, closing");
                        break;
                    }
                }
            }
            // Signal close in stream order; the engine fails remaining pending
            // requests only after draining real responses ahead of this.
            let _ = inc_tx.send(Incoming::Closed);
        });

        (peer, inc_rx)
    }

    async fn write_msg(&self, msg: &RpcMessage) -> Result<()> {
        let line = msg.to_line().map_err(|e| Error::Codec(e.to_string()))?;
        let mut w = self.writer.lock().await;
        w.write_all(line.as_bytes())
            .await
            .map_err(|e| Error::Io(e.to_string()))?;
        w.flush().await.map_err(|e| Error::Io(e.to_string()))?;
        Ok(())
    }

    /// Send a request and await its response.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        self.write_msg(&RpcMessage::request(RpcId::Num(id), method, params))
            .await?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(Error::Protocol(format!("{} (code {})", e.message, e.code))),
            Err(_) => Err(Error::Transport("connection closed before response".into())),
        }
    }

    /// Deliver a response (forwarded by the engine) to the waiting `request`
    /// caller. Unknown ids are ignored (late/duplicate responses).
    pub async fn resolve_response(&self, id: RpcId, result: std::result::Result<Value, RpcError>) {
        if let RpcId::Num(n) = id {
            if let Some(tx) = self.pending.lock().await.remove(&n) {
                let _ = tx.send(result);
            }
        }
    }

    /// Fail every still-pending request (called by the engine on stream close).
    pub async fn fail_all_pending(&self) {
        let mut p = self.pending.lock().await;
        for (_, tx) in p.drain() {
            let _ = tx.send(Err(RpcError {
                code: -1,
                message: "connection closed".into(),
                data: None,
            }));
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write_msg(&RpcMessage::notification(method, params))
            .await
    }

    pub async fn respond_ok(&self, id: RpcId, result: Value) -> Result<()> {
        self.write_msg(&RpcMessage::response_ok(id, result)).await
    }

    pub async fn respond_err(&self, id: RpcId, code: i64, message: &str) -> Result<()> {
        self.write_msg(&RpcMessage::response_err(
            id,
            RpcError {
                code,
                message: message.into(),
                data: None,
            },
        ))
        .await
    }
}

/// Spawn an agent CLI as a child process and return its stdio as boxed streams.
/// stderr is inherited so the agent's logs land in our console.
pub fn spawn_child(command: &[String], cwd: Option<&str>) -> Result<(BoxRead, BoxWrite, Child)> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| Error::Other("empty ACP command".into()))?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Io(format!("failed to spawn `{program}`: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Io("child stdout missing".into()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Io("child stdin missing".into()))?;
    Ok((Box::new(stdout), Box::new(stdin), child))
}
