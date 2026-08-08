//! `browser` — an opt-in, experimental tool driving a real (stealth) browser
//! via managed browser (host-approved distribution). Off by
//! default: registered only when the user turns it on in Settings (see
//! `ToolRegistry::enable_browser`), since the underlying binary is Alpha,
//! 135-320MB, and lazily downloaded on first use rather than bundled.
//!
//! One tool with an `action` enum (navigate/click/extract_text/screenshot)
//! rather than four separate tools, keeping the schema footprint small. Gated
//! MCP-tool-style (`mutating() == true`, always-ask) rather than
//! a brokered product tool's zero-gate posture — this drives a real browser against
//! live sites, a materially larger blast radius than a bounded server-side call.

use std::process::Stdio;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome};
use crate::browser_binary::{ensure_binary, BrowserBinaryConfig, DownloadProgress};
use crate::browser_cdp::BrowserSession;

#[derive(Default)]
struct BrowserState {
    /// Kept alive for its `kill_on_drop`; dropped (killing the process) when
    /// the tool itself is dropped at session end.
    child: Option<Child>,
    /// On Windows, owns the browser's whole child tree for the session.
    _process_fence: Option<exec_core::ProcessFence>,
    session: Option<BrowserSession>,
}

pub struct BrowserTool {
    config: BrowserBinaryConfig,
    state: Mutex<BrowserState>,
}

impl BrowserTool {
    pub fn new(config: BrowserBinaryConfig) -> Self {
        Self {
            config,
            state: Mutex::new(BrowserState::default()),
        }
    }
}

#[async_trait]
impl ToolExecutor for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }
    fn description(&self) -> &str {
        "Drive a real browser (experimental — managed browser, downloaded on first use). Actions: \
        navigate (open a URL), click (a CSS selector), extract_text (a CSS selector, or the whole \
        page if omitted), screenshot. Keeps one tab across calls in a session. For simple page/doc \
        lookups without interaction, prefer web_fetch — it's faster and doesn't need a browser."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "click", "extract_text", "screenshot"]
                },
                "url": {"type": "string", "description": "Required for navigate."},
                "selector": {"type": "string", "description": "CSS selector, for click/extract_text."}
            },
            "required": ["action"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let action = match arg_str(&args, "action") {
            Ok(a) => a,
            Err(e) => return ToolOutcome::error(e),
        };

        let mut state = self.state.lock().await;
        if state.session.is_none() {
            match start_browser(&self.config, ctx).await {
                Ok((child, process_fence, session)) => {
                    state.child = Some(child);
                    state._process_fence = Some(process_fence);
                    state.session = Some(session);
                }
                Err(e) => return ToolOutcome::error(e),
            }
        }
        let session = state.session.as_ref().expect("just ensured above");

        match action.as_str() {
            "navigate" => {
                let url = match arg_str(&args, "url") {
                    Ok(u) => u,
                    Err(e) => return ToolOutcome::error(e),
                };
                match session.navigate(&url).await {
                    Ok(()) => ToolOutcome::ok(format!("Navigated to {url}.")),
                    Err(e) => ToolOutcome::error(e),
                }
            }
            "click" => {
                let selector = match arg_str(&args, "selector") {
                    Ok(s) => s,
                    Err(e) => return ToolOutcome::error(e),
                };
                match session.click(&selector).await {
                    Ok(()) => ToolOutcome::ok(format!("Clicked `{selector}`.")),
                    Err(e) => ToolOutcome::error(e),
                }
            }
            "extract_text" => {
                let selector = arg_str_opt(&args, "selector");
                match session.extract_text(selector.as_deref()).await {
                    Ok(text) => ToolOutcome::ok(text),
                    Err(e) => ToolOutcome::error(e),
                }
            }
            "screenshot" => match session.screenshot().await {
                Ok(png_base64) => ToolOutcome::ok("Captured a screenshot.").with_image(
                    "image/png",
                    png_base64,
                    None,
                ),
                Err(e) => ToolOutcome::error(e),
            },
            other => ToolOutcome::error(format!("unknown action `{other}`")),
        }
    }
}

async fn start_browser(
    config: &BrowserBinaryConfig,
    _ctx: &ToolCtx,
) -> Result<(Child, exec_core::ProcessFence, BrowserSession), String> {
    // Downloads (and caches) the binary if this is the first use — can take a
    // while for a ~150-300MB archive; the tool call blocks on it rather than
    // streaming progress into the tool-call UI in v1 (no "long download" UI
    // pattern to hook into yet — see browser_binary.rs's doc comment). We do
    // log progress at debug so a slow first-run download is diagnosable.
    let binary = ensure_binary(config, |progress: DownloadProgress| match progress.total {
        Some(total) => tracing::debug!(
            downloaded = progress.downloaded,
            total,
            "managed-browser download progress"
        ),
        None => tracing::debug!(
            downloaded = progress.downloaded,
            "managed-browser download progress"
        ),
    })
    .await?;

    let port = free_local_port()?;
    // Spawned directly (not through `ctx.executor`) — managed browser is a
    // property of the machine running Agent Desktop's own GUI, not of
    // whichever project executor is active (same reasoning `tools/mobile.rs`
    // already documents for simulators/emulators).
    let mut command = Command::new(&binary);
    command
        .arg("--headless=new")
        .arg("--no-sandbox")
        .arg(format!("--remote-debugging-port={port}"))
        .arg("--remote-debugging-address=127.0.0.1")
        .arg("--remote-allow-origins=*")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    exec_core::suppress_console_window(&mut command);
    let child = command
        .spawn()
        .map_err(|e| format!("failed to start managed browser: {e}"))?;
    let process_fence = exec_core::ProcessFence::attach(child.id());

    // Give the browser a moment to open its devtools port before we probe it.
    let mut last_err = String::new();
    for _ in 0..50 {
        match BrowserSession::connect(port).await {
            Ok(session) => return Ok((child, process_fence, session)),
            Err(e) => {
                last_err = e;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
    Err(format!(
        "managed browser didn't come up in time: {last_err}"
    ))
}

/// Bind to port 0 and read back the OS-assigned free port.
fn free_local_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_is_mutating_and_advertises_the_action_enum() {
        let t = BrowserTool::new(crate::browser_binary::test_browser_config());
        assert!(t.mutating());
        assert_eq!(t.name(), "browser");
        let actions = t.parameters()["properties"]["action"]["enum"].clone();
        assert_eq!(
            actions,
            json!(["navigate", "click", "extract_text", "screenshot"])
        );
    }

    #[test]
    fn free_local_port_returns_a_bindable_port() {
        let port = free_local_port().unwrap();
        assert!(port > 0);
    }
}
