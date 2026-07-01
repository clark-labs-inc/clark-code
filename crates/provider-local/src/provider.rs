//! [`LocalAgentProvider`] — the local coding agent behind the `agent_core`
//! `Provider` trait. Connect sets the model endpoint + tool registry; each
//! session is bound to a project root; each prompt drives a local tool-calling
//! loop ([`crate::engine`]) whose normalized events stream back to the UI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_core::domain::{AgentEvent, ContentBlock, Role};
use agent_core::error::{Error, Result};
use agent_core::ids::{ProviderId, RunId, SessionId};
use agent_core::provider::{
    ClientResponse, EventStream, PromptInput, Provider, ProviderCapabilities, ProviderConfig,
    Session, SessionOptions,
};
use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::config::LocalConfig;
use crate::engine::{run_turn, Decision, RunControl, SessionState, TurnContext};
use crate::exec::{Executor, LocalExecutor, RemoteExecutor};
use crate::llm::{ChatMessage, LlmClient};
use crate::prompt::system_prompt;
use crate::sandbox::Sandbox;
use crate::tools::{ReadTracker, ToolCtx, ToolRegistry};

pub struct LocalAgentProvider {
    config: Option<LocalConfig>,
    llm: Option<LlmClient>,
    registry: Option<Arc<ToolRegistry>>,
    sandbox: Option<Arc<Sandbox>>,
    session_id: Option<SessionId>,
    session: Arc<Mutex<SessionState>>,
    control: Arc<Mutex<RunControl>>,
    /// Session-scoped read tracker (read-before-edit/write invariant).
    reads: Arc<std::sync::Mutex<ReadTracker>>,
    /// Cancellation token for the in-flight run (replaced each prompt).
    cancel: CancellationToken,
    /// Where this session's tool I/O runs — local today, remote (over the
    /// exec-server) once a remote project is selected. Chosen in `new_session`.
    executor: Arc<dyn crate::exec::Executor>,
    run_counter: AtomicU64,
    /// Last MCP connection result, surfaced to the settings UI.
    mcp_status: Vec<crate::mcp::McpStatus>,
}

impl LocalAgentProvider {
    pub fn new() -> Self {
        Self {
            config: None,
            llm: None,
            registry: None,
            sandbox: None,
            session_id: None,
            session: Arc::new(Mutex::new(SessionState {
                transcript: Vec::new(),
                policy: HashMap::new(),
                allow_commands: Vec::new(),
                deny_commands: Vec::new(),
            })),
            control: Arc::new(Mutex::new(RunControl::default())),
            reads: Arc::new(std::sync::Mutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
            executor: Arc::new(crate::exec::LocalExecutor),
            run_counter: AtomicU64::new(0),
            mcp_status: Vec::new(),
        }
    }

    /// MCP connection statuses from the last `connect`, for the settings UI.
    pub fn mcp_status(&self) -> &[crate::mcp::McpStatus] {
        &self.mcp_status
    }

    fn config(&self) -> Result<&LocalConfig> {
        self.config.as_ref().ok_or(Error::NotConnected)
    }
}

impl Default for LocalAgentProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for LocalAgentProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("local")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            permissions: true,
            fs: true,
            terminal: true,
            load_session: false,
            modes: Vec::new(),
        }
    }

    async fn connect(&mut self, config: ProviderConfig) -> Result<()> {
        let local = LocalConfig::from_provider_config(&config);
        let llm = LlmClient::new(&local).map_err(Error::Other)?;
        let mut registry = ToolRegistry::new(local.clark.clone());
        // Connect MCP servers and register their tools (failures are non-fatal).
        self.mcp_status = registry.connect_mcp(&local.mcp_servers).await;
        self.llm = Some(llm);
        self.registry = Some(Arc::new(registry));
        self.config = Some(local);
        Ok(())
    }

    async fn new_session(&mut self, options: SessionOptions) -> Result<Session> {
        let config = self.config()?.clone();

        // A remote project runs its tools on a remote host over the exec-server;
        // a local project runs them here. Pick the sandbox + executor to match.
        let (sandbox, executor): (Arc<Sandbox>, Arc<dyn Executor>) =
            if let Some(remote) = &config.remote {
                let sandbox = Arc::new(Sandbox::new_remote(&remote.cwd).map_err(Error::Other)?);
                let exec = RemoteExecutor::connect(&remote.ws_url, &remote.token)
                    .await
                    .map_err(Error::Other)?;
                (sandbox, Arc::new(exec))
            } else {
                let cwd = options.cwd.or(config.cwd.clone()).ok_or_else(|| {
                    Error::Unsupported("local provider requires a project `cwd`".into())
                })?;
                let sandbox = Arc::new(Sandbox::new(&cwd).map_err(Error::Io)?);
                (sandbox, Arc::new(LocalExecutor))
            };
        self.executor = executor;

        let mut prompt = system_prompt(&sandbox, config.clark.is_some());
        // Surface the user's Claude Code skills (read `.claude` through the
        // session executor — the local disk, or the remote host over the tunnel).
        if let Some(skills) =
            crate::claude_import::skills_prompt_section(self.executor.as_ref(), sandbox.root())
                .await
        {
            prompt.push_str(&skills);
        }
        {
            let mut s = self.session.lock().await;
            s.transcript = vec![ChatMessage::system(prompt)];
            s.policy = config.permissions.clone();
            s.allow_commands = config.command_allowlist.clone();
            s.deny_commands = config.command_denylist.clone();
        }
        self.control.lock().await.clear();
        // A new session starts with no files "read".
        if let Ok(mut reads) = self.reads.lock() {
            *reads = ReadTracker::default();
        }

        let id = SessionId::new(uuid::Uuid::new_v4().to_string());
        self.sandbox = Some(sandbox);
        self.session_id = Some(id.clone());
        Ok(Session {
            id,
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: None,
        })
    }

    async fn load_session(&mut self, _id: SessionId) -> Result<Session> {
        Err(Error::Unsupported(
            "local provider does not support resuming sessions".into(),
        ))
    }

    async fn prompt(&mut self, _session: &SessionId, input: PromptInput) -> Result<EventStream> {
        let llm = self.llm.clone().ok_or(Error::NotConnected)?;
        let registry = self.registry.clone().ok_or(Error::NotConnected)?;
        let sandbox = self.sandbox.clone().ok_or(Error::NotConnected)?;
        let session_id = self.session_id.clone().ok_or(Error::NotConnected)?;
        let max_iterations = self.config()?.max_iterations;

        let text = prompt_text(&input);
        {
            let mut s = self.session.lock().await;
            s.transcript.push(ChatMessage::user(text));
        }

        // Fresh cancellation scope for this run.
        let cancel = CancellationToken::new();
        self.cancel = cancel.clone();

        let run = RunId::new(format!(
            "run-{}",
            self.run_counter.fetch_add(1, Ordering::SeqCst) + 1
        ));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();

        let tc = TurnContext {
            llm,
            registry,
            ctx: ToolCtx {
                sandbox,
                reads: self.reads.clone(),
                cancel,
                executor: self.executor.clone(),
            },
            session: self.session.clone(),
            control: self.control.clone(),
            session_id,
            max_iterations,
        };
        tokio::spawn(run_turn(tc, tx, run));
        Ok(rx.boxed())
    }

    async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> Result<()> {
        self.cancel.cancel();
        self.control.lock().await.clear();
        Ok(())
    }

    async fn respond(&mut self, _session: &SessionId, response: ClientResponse) -> Result<()> {
        match response {
            ClientResponse::Permission { request, option } => {
                let decision = Decision::from_option(&option);
                self.control.lock().await.resolve(&request, decision);
                Ok(())
            }
        }
    }
}

/// Flatten a prompt's text blocks (and inline any text attachments) into one
/// user message. Non-text attachments are noted by name.
fn prompt_text(input: &PromptInput) -> String {
    let mut text: String = input
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    for att in &input.attachments {
        if att.is_text() {
            if let Ok(decoded) = decode_base64_text(&att.data_base64) {
                text.push_str(&format!(
                    "\n\n--- attached file: {} ---\n{decoded}\n",
                    att.filename
                ));
            }
        } else {
            text.push_str(&format!("\n\n[attached file: {}]", att.filename));
        }
    }
    let _ = Role::User; // role is fixed for user prompts
    text
}

/// Minimal standard-base64 decoder (no external dep) for inlining text files.
fn decode_base64_text(data: &str) -> std::result::Result<String, ()> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in data.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = val(c).ok_or(())? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::domain::PendingUpload;

    #[test]
    fn prompt_text_joins_blocks() {
        let input = PromptInput {
            blocks: vec![ContentBlock::text("hello "), ContentBlock::text("world")],
            attachments: Vec::new(),
        };
        assert_eq!(prompt_text(&input), "hello world");
    }

    #[test]
    fn prompt_text_inlines_text_attachment() {
        let input = PromptInput {
            blocks: vec![ContentBlock::text("see file")],
            attachments: vec![PendingUpload {
                filename: "note.txt".into(),
                content_type: "text/plain".into(),
                data_base64: "aGVsbG8=".into(), // "hello"
            }],
        };
        let text = prompt_text(&input);
        assert!(text.contains("see file"));
        assert!(text.contains("attached file: note.txt"));
        assert!(text.contains("hello"));
    }

    #[test]
    fn base64_decodes_text() {
        assert_eq!(
            decode_base64_text("aGVsbG8gd29ybGQ=").unwrap(),
            "hello world"
        );
    }

    #[tokio::test]
    async fn new_session_requires_cwd() {
        let mut p = LocalAgentProvider::new();
        p.connect(ProviderConfig::default()).await.unwrap();
        let err = p.new_session(SessionOptions::default()).await.unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
    }

    #[tokio::test]
    async fn new_session_seeds_transcript_with_system_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = LocalAgentProvider::new();
        p.connect(ProviderConfig::default()).await.unwrap();
        let opts = SessionOptions {
            cwd: Some(dir.path().to_string_lossy().to_string()),
            mode: None,
        };
        let session = p.new_session(opts).await.unwrap();
        assert_eq!(session.provider, ProviderId::new("local"));
        let s = p.session.lock().await;
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript[0].role, "system");
    }
}
