//! MCP (Model Context Protocol) client over the stdio transport.
//!
//! Spawns a configured server process, performs the JSON-RPC `initialize`
//! handshake, discovers its tools (`tools/list`), and exposes `call_tool`
//! (`tools/call`). Each discovered tool is wrapped as a [`ToolExecutor`]
//! ([`McpTool`]) and registered into the agent's tool registry, so MCP tools are
//! callable by the model and pass through the same permission gate as the
//! built-ins. The server process is killed when its client is dropped.
//!
//! Newline-delimited JSON-RPC 2.0 is used (the MCP stdio framing). The client is
//! robust to slow/dead servers: requests time out, a closed stream fails all
//! pending calls, and a failing server is skipped rather than breaking the
//! session.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};

const PROTOCOL_VERSION: &str = "2024-11-05";
const INIT_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// One stdio MCP server the user has configured.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Outcome of connecting a server, surfaced to the settings UI.
#[derive(Clone, Debug, Serialize)]
pub struct McpStatus {
    pub server: String,
    pub connected: bool,
    pub tool_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Namespaced tool names this server contributed.
    pub tools: Vec<String>,
}

#[derive(Clone, Debug)]
struct McpToolDef {
    name: String,
    description: String,
    input_schema: Value,
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// A live connection to one MCP server.
pub struct McpClient {
    name: String,
    stdin: AsyncMutex<ChildStdin>,
    pending: Pending,
    next_id: AtomicI64,
    tools: Vec<McpToolDef>,
    /// Kept alive (with `kill_on_drop`) so the server dies with the client.
    _child: Child,
}

impl McpClient {
    /// Spawn the server, handshake, and list its tools.
    pub async fn connect(cfg: &McpServerConfig) -> Result<Self, String> {
        let mut child = Command::new(&cfg.command)
            .args(&cfg.args)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("failed to start `{}`: {e}", cfg.command))?;

        let stdin = child.stdin.take().ok_or("server has no stdin")?;
        let stdout = child.stdout.take().ok_or("server has no stdout")?;
        let stderr = child.stderr.take();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        // Reader: match responses to pending requests by id; ignore notifications.
        {
            let pending = pending.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if let Some(id) = msg.get("id").and_then(Value::as_i64) {
                        if let Some(tx) = pending.lock().unwrap().remove(&id) {
                            let res = match msg.get("error") {
                                Some(err) => Err(err
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("server error")
                                    .to_string()),
                                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = tx.send(res);
                        }
                    }
                }
                // Stream closed: fail anything still pending.
                for (_, tx) in pending.lock().unwrap().drain() {
                    let _ = tx.send(Err("MCP server closed the connection".into()));
                }
            });
        }
        // Drain stderr to the log so a chatty server can't block on a full pipe.
        if let Some(stderr) = stderr {
            let name = cfg.name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(server = %name, "mcp: {line}");
                }
            });
        }

        let mut client = Self {
            name: cfg.name.clone(),
            stdin: AsyncMutex::new(stdin),
            pending,
            next_id: AtomicI64::new(1),
            tools: Vec::new(),
            _child: child,
        };

        // initialize handshake.
        tokio::time::timeout(
            INIT_TIMEOUT,
            client.call(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "clark-desktop", "version": env!("CARGO_PKG_VERSION") },
                }),
            ),
        )
        .await
        .map_err(|_| "MCP initialize timed out".to_string())??;
        client.notify("notifications/initialized", json!({})).await;

        // tools/list.
        let listed = tokio::time::timeout(INIT_TIMEOUT, client.call("tools/list", json!({})))
            .await
            .map_err(|_| "MCP tools/list timed out".to_string())??;
        client.tools = listed
            .get("tools")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_tool_def).collect())
            .unwrap_or_default();

        Ok(client)
    }

    /// The configured server name (used to namespace its tools). Part of the
    /// client's public surface alongside `tool_names`/`tool_count`.
    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Namespaced tool names this server exposes.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|t| namespaced_tool_name(&self.name, &t.name))
            .collect()
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    async fn send_line(&self, line: String) -> Result<(), String> {
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stdin.write_all(b"\n").await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.send_line(req.to_string()).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err("MCP request was dropped".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("MCP request timed out".into())
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) {
        let req = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let _ = self.send_line(req.to_string()).await;
    }

    /// Call a tool by its raw (server-side) name; returns `(text, is_error)`.
    pub async fn call_tool(&self, raw_name: &str, args: Value) -> Result<(String, bool), String> {
        let result = self
            .call("tools/call", json!({ "name": raw_name, "arguments": args }))
            .await?;
        Ok(extract_content(&result))
    }

    /// Wrap this server's tools as [`ToolExecutor`]s.
    pub fn executors(self: &Arc<Self>) -> Vec<Arc<dyn ToolExecutor>> {
        self.tools
            .iter()
            .map(|def| Arc::new(McpTool::new(&self.name, def, self.clone())) as Arc<dyn ToolExecutor>)
            .collect()
    }
}

/// Connect each server, collect its status (tools discovered, or the error),
/// then drop it. A stateless "test connection" for the settings UI.
pub async fn probe_mcp_servers(servers: &[McpServerConfig]) -> Vec<McpStatus> {
    let mut out = Vec::new();
    for cfg in servers {
        out.push(match McpClient::connect(cfg).await {
            Ok(client) => McpStatus {
                server: cfg.name.clone(),
                connected: true,
                tool_count: client.tool_count(),
                error: None,
                tools: client.tool_names(),
            },
            Err(error) => McpStatus {
                server: cfg.name.clone(),
                connected: false,
                tool_count: 0,
                error: Some(error),
                tools: Vec::new(),
            },
        });
    }
    out
}

fn parse_tool_def(v: &Value) -> Option<McpToolDef> {
    let name = v.get("name").and_then(Value::as_str)?.to_string();
    Some(McpToolDef {
        name,
        description: v
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        input_schema: v
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" })),
    })
}

fn extract_content(result: &Value) -> (String, bool) {
    let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| match i.get("type").and_then(Value::as_str) {
                    Some("text") => i.get("text").and_then(Value::as_str).map(String::from),
                    Some(other) => Some(format!("[{other} content]")),
                    None => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let text = if text.trim().is_empty() {
        "(no output)".to_string()
    } else {
        text
    };
    (text, is_error)
}

/// Namespace + sanitize an MCP tool name into a valid function name
/// (`mcp_<server>_<tool>`, alnum/`_`/`-`, ≤ 64 chars). The `mcp_` prefix also
/// marks the tool as external for the permission gate.
pub fn namespaced_tool_name(server: &str, tool: &str) -> String {
    let san = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    let mut name = format!("mcp_{}_{}", san(server), san(tool));
    name.truncate(64);
    name
}

/// True if a tool name belongs to an MCP server (external) — used by the gate.
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp_")
}

/// One MCP server tool, presented to the agent as a [`ToolExecutor`].
pub struct McpTool {
    full_name: String,
    raw_name: String,
    description: String,
    schema: Value,
    client: Arc<McpClient>,
}

impl McpTool {
    fn new(server: &str, def: &McpToolDef, client: Arc<McpClient>) -> Self {
        Self {
            full_name: namespaced_tool_name(server, &def.name),
            raw_name: def.name.clone(),
            description: def.description.clone(),
            schema: def.input_schema.clone(),
            client,
        }
    }
}

#[async_trait]
impl ToolExecutor for McpTool {
    fn name(&self) -> &str {
        &self.full_name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        self.schema.clone()
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    fn mutating(&self) -> bool {
        true // external side effects → always gated
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        tokio::select! {
            _ = ctx.cancel.cancelled() => ToolOutcome::error("cancelled"),
            r = self.client.call_tool(&self.raw_name, args) => match r {
                Ok((text, true)) => ToolOutcome::error(text),
                Ok((text, false)) => ToolOutcome::ok(text),
                Err(e) => ToolOutcome::error(format!("MCP tool `{}` failed: {e}", self.raw_name)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_and_sanitizes_tool_names() {
        assert_eq!(namespaced_tool_name("github", "create_issue"), "mcp_github_create_issue");
        assert_eq!(namespaced_tool_name("my server", "do.thing"), "mcp_my_server_do_thing");
        assert!(is_mcp_tool("mcp_github_create_issue"));
        assert!(!is_mcp_tool("read_file"));
        assert!(namespaced_tool_name(&"x".repeat(80), "y").len() <= 64);
    }

    #[test]
    fn extracts_text_content_and_error_flag() {
        let ok = json!({ "content": [{ "type": "text", "text": "hello" }] });
        assert_eq!(extract_content(&ok), ("hello".to_string(), false));
        let err = json!({ "isError": true, "content": [{ "type": "text", "text": "boom" }] });
        assert_eq!(extract_content(&err), ("boom".to_string(), true));
        let empty = json!({ "content": [] });
        assert_eq!(extract_content(&empty).0, "(no output)");
    }

    #[test]
    fn parses_tool_defs() {
        let def = parse_tool_def(&json!({
            "name": "search", "description": "Search the web",
            "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } } }
        }))
        .unwrap();
        assert_eq!(def.name, "search");
        assert_eq!(def.description, "Search the web");
        assert_eq!(def.input_schema["type"], "object");
    }

    /// A minimal stdio MCP server (newline-delimited JSON-RPC) for the roundtrip test.
    const MOCK_SERVER_JS: &str = r#"
const rl = require('readline').createInterface({ input: process.stdin });
rl.on('line', (line) => {
  let m; try { m = JSON.parse(line); } catch { return; }
  if (m.id === undefined || m.id === null) return; // notification
  let result = {};
  if (m.method === 'initialize') result = { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'mock', version: '0' } };
  else if (m.method === 'tools/list') result = { tools: [{ name: 'echo', description: 'Echo the message', inputSchema: { type: 'object', properties: { message: { type: 'string' } }, required: ['message'] } }] };
  else if (m.method === 'tools/call') result = { content: [{ type: 'text', text: String(m.params.arguments.message) }] };
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result }) + '\n');
});
"#;

    fn node_available() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn connects_lists_and_calls_a_mock_server() {
        if !node_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mock-mcp.js");
        std::fs::write(&script, MOCK_SERVER_JS).unwrap();

        let cfg = McpServerConfig {
            name: "mock".into(),
            command: "node".into(),
            args: vec![script.to_string_lossy().into_owned()],
            env: HashMap::new(),
        };
        let client = McpClient::connect(&cfg).await.expect("connect");
        assert_eq!(client.tool_count(), 1);
        assert_eq!(client.tool_names(), vec!["mcp_mock_echo".to_string()]);

        let (text, is_error) = client
            .call_tool("echo", json!({ "message": "hello mcp" }))
            .await
            .expect("call");
        assert_eq!(text, "hello mcp");
        assert!(!is_error);
    }

    #[tokio::test]
    async fn a_missing_server_command_fails_cleanly() {
        let cfg = McpServerConfig {
            name: "nope".into(),
            command: "definitely-not-a-real-binary-xyz".into(),
            args: vec![],
            env: HashMap::new(),
        };
        assert!(McpClient::connect(&cfg).await.is_err());
    }
}
