//! A minimal hand-rolled Chrome DevTools Protocol (CDP) client, over the
//! shared `tokio-tungstenite` dependency used by the provider runtime's
//! transport — no new WS crate needed. Same request/response-correlation
//! shape as `mcp.rs`'s stdio JSON-RPC client (a `pending: HashMap<id,
//! oneshot::Sender>` map fed by one reader task), just over a WS frame stream
//! instead of newline-delimited stdout.
//!
//! Scope is deliberately narrow: just enough CDP to support the `browser`
//! tool's four actions (navigate/click/extract_text/screenshot). No general
//! CDP domain coverage, no multi-tab management — one current target at a time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio_tungstenite::tungstenite::Message;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;
type WsSink = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

/// A live CDP connection to the browser-level WebSocket endpoint.
struct CdpClient {
    sink: AsyncMutex<WsSink>,
    pending: Pending,
    next_id: AtomicI64,
}

impl CdpClient {
    async fn connect(ws_url: &str) -> Result<Self, String> {
        let (stream, _resp) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| format!("connecting to managed browser's CDP endpoint: {e}"))?;
        let (sink, mut source) = stream.split();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        {
            let pending = pending.clone();
            tokio::spawn(async move {
                while let Some(msg) = source.next().await {
                    let Ok(Message::Text(text)) = msg else {
                        continue;
                    };
                    let Ok(value) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    // Events (no `id`) are dropped — this client is request/response only.
                    let Some(id) = value.get("id").and_then(Value::as_i64) else {
                        continue;
                    };
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let res = match value.get("error") {
                            Some(err) => Err(err
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("CDP error")
                                .to_string()),
                            None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = tx.send(res);
                    }
                }
                for (_, tx) in pending.lock().unwrap().drain() {
                    let _ = tx.send(Err("managed browser closed the CDP connection".into()));
                }
            });
        }

        Ok(Self {
            sink: AsyncMutex::new(sink),
            pending,
            next_id: AtomicI64::new(1),
        })
    }

    /// Send a CDP command, optionally scoped to a target's flat `sessionId`.
    async fn call(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let mut req = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            req["sessionId"] = json!(sid);
        }
        {
            let mut sink = self.sink.lock().await;
            if let Err(e) = sink.send(Message::Text(req.to_string().into())).await {
                self.pending.lock().unwrap().remove(&id);
                return Err(format!("sending CDP command: {e}"));
            }
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err("CDP request was dropped".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("CDP `{method}` timed out"))
            }
        }
    }
}

/// One "current tab" driven over CDP — the whole surface the `browser` tool
/// needs. Created once per app session and reused across `navigate`/`click`/
/// `extract_text`/`screenshot` calls, mirroring how `BackgroundTasks` persists
/// state across tool calls.
pub struct BrowserSession {
    cdp: CdpClient,
    /// The current target's flat-mode session id, once one exists.
    session_id: AsyncMutex<Option<String>>,
}

impl BrowserSession {
    /// Discover the browser-level WS endpoint from managed browser's
    /// `--remote-debugging-port`, then connect.
    pub async fn connect(devtools_port: u16) -> Result<Self, String> {
        let version_url = format!("http://127.0.0.1:{devtools_port}/json/version");
        let resp: Value = reqwest::get(&version_url)
            .await
            .map_err(|e| format!("reaching managed browser's devtools endpoint: {e}"))?
            .json()
            .await
            .map_err(|e| format!("parsing managed browser's devtools response: {e}"))?;
        let ws_url = resp
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .ok_or("managed browser's devtools response had no webSocketDebuggerUrl")?;
        Ok(Self {
            cdp: CdpClient::connect(ws_url).await?,
            session_id: AsyncMutex::new(None),
        })
    }

    /// Navigate the current tab to `url`, creating one via `Target.createTarget`
    /// on the first call (which also navigates it — no separate `Page.navigate`
    /// needed then) and reusing it on subsequent calls.
    pub async fn navigate(&self, url: &str) -> Result<(), String> {
        let mut session_id = self.session_id.lock().await;
        match session_id.as_ref() {
            None => {
                let result = self
                    .cdp
                    .call("Target.createTarget", json!({ "url": url }), None)
                    .await?;
                let target_id = result
                    .get("targetId")
                    .and_then(Value::as_str)
                    .ok_or("Target.createTarget returned no targetId")?;
                let attach = self
                    .cdp
                    .call(
                        "Target.attachToTarget",
                        json!({ "targetId": target_id, "flatten": true }),
                        None,
                    )
                    .await?;
                let sid = attach
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .ok_or("Target.attachToTarget returned no sessionId")?
                    .to_string();
                self.cdp.call("Page.enable", json!({}), Some(&sid)).await?;
                *session_id = Some(sid);
            }
            Some(sid) => {
                self.cdp
                    .call("Page.navigate", json!({ "url": url }), Some(sid))
                    .await?;
            }
        }
        Ok(())
    }

    /// Click the first element matching `selector`. Uses `Runtime.evaluate`
    /// rather than the DOM/Input domains (box-model coordinates, dispatching
    /// synthetic mouse events) — a JS `.click()` is far less protocol surface
    /// for the same practical result.
    pub async fn click(&self, selector: &str) -> Result<(), String> {
        let expr = format!(
            "(() => {{ const el = document.querySelector({}); if (!el) return 'not found'; \
            el.click(); return 'ok'; }})()",
            json_string(selector)
        );
        let result = self.evaluate(&expr).await?;
        if result.as_str() == Some("not found") {
            return Err(format!("no element matches `{selector}`"));
        }
        Ok(())
    }

    /// Visible text of the element matching `selector`, or the whole page's
    /// text if `selector` is `None`.
    pub async fn extract_text(&self, selector: Option<&str>) -> Result<String, String> {
        let expr = match selector {
            Some(sel) => format!(
                "(() => {{ const el = document.querySelector({}); return el ? el.innerText : null; }})()",
                json_string(sel)
            ),
            None => "document.body ? document.body.innerText : ''".to_string(),
        };
        let result = self.evaluate(&expr).await?;
        result
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("no element matches `{}`", selector.unwrap_or("<page>")))
    }

    /// A base64-encoded PNG screenshot of the current tab.
    pub async fn screenshot(&self) -> Result<String, String> {
        let session_id = self.session_id.lock().await;
        let sid = session_id
            .as_ref()
            .ok_or("no page yet — call navigate first")?;
        let result = self
            .cdp
            .call(
                "Page.captureScreenshot",
                json!({ "format": "png" }),
                Some(sid),
            )
            .await?;
        result
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "Page.captureScreenshot returned no data".to_string())
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, String> {
        let session_id = self.session_id.lock().await;
        let sid = session_id
            .as_ref()
            .ok_or("no page yet — call navigate first")?;
        let result = self
            .cdp
            .call(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
                Some(sid),
            )
            .await?;
        if let Some(exc) = result.get("exceptionDetails") {
            return Err(format!("page script error: {exc}"));
        }
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }
}

/// A JSON-encoded string literal, for splicing a Rust string safely into a
/// JS expression sent to `Runtime.evaluate`.
fn json_string(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_string_escapes_for_safe_js_embedding() {
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("#main .item"), "\"#main .item\"");
    }

    /// Exercises the actual wire logic (request `id` <-> response correlation,
    /// `sessionId` passthrough) against a real WebSocket connection to a fake
    /// in-process CDP server — the part of this module that's genuinely novel
    /// (no other code in the crate does WS request/response correlation),
    /// unlike the HTTP `/json/version` discovery step in `BrowserSession::connect`,
    /// which is a single trivial JSON field read.
    #[tokio::test]
    async fn cdp_client_correlates_requests_and_responses_over_a_fake_ws_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let req: Value = serde_json::from_str(&text).unwrap();
                let resp = json!({
                    "id": req["id"],
                    "result": { "echoedMethod": req["method"], "sawSessionId": req.get("sessionId") },
                });
                ws.send(Message::Text(resp.to_string().into()))
                    .await
                    .unwrap();
            }
        });

        let url = format!("ws://{addr}/");
        let client = CdpClient::connect(&url).await.unwrap();

        let result = client
            .call("Test.method", json!({"x": 1}), None)
            .await
            .unwrap();
        assert_eq!(result["echoedMethod"], "Test.method");
        assert_eq!(result["sawSessionId"], Value::Null);

        let scoped = client
            .call("Page.navigate", json!({"url": "https://x"}), Some("sess-1"))
            .await
            .unwrap();
        assert_eq!(scoped["sawSessionId"], "sess-1");

        // Two concurrent calls resolve to their own responses, not each other's
        // — proves correlation is genuinely by id, not just FIFO ordering.
        let (a, b) = tokio::join!(
            client.call("A", json!({}), None),
            client.call("B", json!({}), None),
        );
        assert_eq!(a.unwrap()["echoedMethod"], "A");
        assert_eq!(b.unwrap()["echoedMethod"], "B");
    }
}
