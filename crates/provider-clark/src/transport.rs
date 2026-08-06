//! WebSocket + MessagePack transport for the Clark gateway.
//!
//! Clean-room: built from the observed wire contract, not Clark source. The
//! gateway speaks msgpack frames over a WebSocket at `/ws`, authenticates via an
//! `Authorization: Bearer <token>` header, and pushes `{type, ...}` messages
//! (notably `{type:"event", event:{type, data, conversation_id, ...}}`).

use std::sync::Arc;

use agent_core::error::{Error, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// A cloneable handle to the gateway socket. Sends client messages; a background
/// reader forwards decoded server messages to the engine.
#[derive(Clone)]
pub struct ClarkSocket {
    sink: Arc<Mutex<futures::stream::SplitSink<WsStream, Message>>>,
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl ClarkSocket {
    /// Connect to `url` (ws:// or wss://) with an optional bearer token. Returns
    /// the socket handle plus a receiver of decoded server messages.
    pub async fn connect(
        url: &str,
        token: Option<&str>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Value>)> {
        let mut request = url
            .into_client_request()
            .map_err(|e| Error::Transport(format!("bad url: {e}")))?;
        if let Some(tok) = token {
            let value = format!("Bearer {tok}")
                .parse()
                .map_err(|_| Error::Transport("invalid auth token".into()))?;
            request.headers_mut().insert("Authorization", value);
        }

        let (ws, _resp) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| Error::Transport(format!("ws connect failed: {e}")))?;
        let (sink, mut stream) = ws.split();
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(Message::Binary(bytes)) => {
                        match agent_core::codec::msgpack::decode::<Value>(&bytes) {
                            Ok(v) => {
                                if tx.send(v).is_err() {
                                    break;
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "clark: undecodable frame"),
                        }
                    }
                    Ok(Message::Text(t)) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&t) {
                            let _ = tx.send(v);
                        }
                    }
                    Ok(Message::Close(_)) => {
                        tracing::info!("clark: socket closed by server");
                        break;
                    }
                    Ok(_) => {} // ping/pong/frame
                    Err(e) => {
                        tracing::warn!(error = %e, "clark: socket error");
                        break;
                    }
                }
            }
        });

        Ok((
            Self {
                sink: Arc::new(Mutex::new(sink)),
            },
            rx,
        ))
    }

    /// Send a client message (encoded as a msgpack binary frame).
    pub async fn send(&self, value: &Value) -> Result<()> {
        let bytes = agent_core::codec::msgpack::encode(value)?;
        let mut sink = self.sink.lock().await;
        sink.send(Message::Binary(bytes.into()))
            .await
            .map_err(|e| Error::Transport(format!("ws send failed: {e}")))?;
        Ok(())
    }
}
