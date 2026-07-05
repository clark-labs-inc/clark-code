//! [`LocalAgentProvider`] — the local coding agent behind the `agent_core`
//! `Provider` trait. Connect sets the model endpoint + tool registry; each
//! session is bound to a project root; each prompt drives a local tool-calling
//! loop ([`crate::engine`]) whose normalized events stream back to the UI.

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
use crate::engine::{run_turn, TurnContext};
use crate::exec::{Executor, LocalExecutor, RemoteExecutor};
use crate::llm::LlmClient;
use crate::loop_state::{Decision, RunControl, SessionState};
use crate::prompt::system_prompt;
use crate::sandbox::Sandbox;
use crate::tools::{ReadTracker, ToolCtx, ToolRegistry};

/// Injected as a prefix to the user's turn text while Plan Mode is active.
/// Per-turn (not baked into the cached system-prompt prefix) since the mode
/// can flip mid-session via Shift+Tab — see `prompt.rs`'s doc comment on
/// keeping volatile facts out of the stable prompt.
const PLAN_MODE_REMINDER: &str = "Plan mode is active. You MUST NOT edit files, run non-read-only \
shell commands, or otherwise mutate the system — this supersedes any other instruction. Read-only \
tools (read_file, grep, glob, list_dir) remain available. Research thoroughly, then call \
propose_plan with your full plan written out in markdown for the user to approve.";

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
    /// Session-scoped `bash(run_in_background: true)` task registry.
    background: Arc<crate::background::BackgroundTasks>,
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
            session: Arc::new(Mutex::new(SessionState::default())),
            control: Arc::new(Mutex::new(RunControl::default())),
            reads: Arc::new(std::sync::Mutex::new(ReadTracker::default())),
            background: Arc::new(crate::background::BackgroundTasks::default()),
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
        let memory = local
            .memories_enabled
            .then(|| crate::tools::memory::MemoryConfig {
                global_dir: crate::memory::global_memory_dir(),
                personal: local.api_key.clone().map(|api_key| {
                    crate::tools::memory::PersonalRecall {
                        base_url: local.base_url.clone(),
                        api_key,
                    }
                }),
            });
        let mut registry = ToolRegistry::new(local.clark.clone(), memory);
        if local.browser_enabled {
            registry.enable_browser();
        }
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
        let (sandbox, executor): (Sandbox, Arc<dyn Executor>) = if let Some(remote) = &config.remote
        {
            let sandbox = Sandbox::new_remote(&remote.cwd).map_err(Error::Other)?;
            let exec = RemoteExecutor::connect(&remote.ws_url, &remote.token)
                .await
                .map_err(Error::Other)?;
            (sandbox, Arc::new(exec))
        } else {
            let cwd = options.cwd.or(config.cwd.clone()).ok_or_else(|| {
                Error::Unsupported("local provider requires a project `cwd`".into())
            })?;
            let sandbox = Sandbox::new(&cwd).map_err(Error::Io)?;
            (sandbox, Arc::new(LocalExecutor))
        };
        self.executor = executor;

        let id = SessionId::new(uuid::Uuid::new_v4().to_string());

        // Provision a per-session, app-managed workspace for agent-authored
        // documents (local sessions only — a remote executor can't reach a local
        // path). Extend the sandbox to permit writes there in addition to the
        // project root; the prompt then points the agent at it for documents.
        let mut sandbox = sandbox;
        if config.remote.is_none() {
            if let Some(ws) = crate::workspace::session_workspace(id.as_str()) {
                if std::fs::create_dir_all(&ws).is_ok() {
                    sandbox = sandbox.with_docs(ws);
                }
            }
        }
        let sandbox = Arc::new(sandbox);

        let mut prompt = system_prompt(&sandbox, config.clark.is_some());
        if let Some(docs) = sandbox.docs_root() {
            prompt.push_str(&crate::workspace::prompt_section(docs));
        }
        // Surface the user's Claude Code skills (read `.claude` through the
        // session executor — the local disk, or the remote host over the tunnel).
        if let Some(skills) =
            crate::claude_import::skills_prompt_section(self.executor.as_ref(), sandbox.root())
                .await
        {
            prompt.push_str(&skills);
        }
        // Durable memory, when enabled: list the project scope (through the
        // session executor — local or remote) and the global scope (always
        // local), and tell the agent it has the `memory` tool.
        if config.memories_enabled {
            let mut mem = String::new();
            if let Some(proj) = crate::memory::scope_listing(
                self.executor.as_ref(),
                &crate::memory::memory_dir(sandbox.root()),
                "Project",
            )
            .await
            {
                mem.push_str(&proj);
                mem.push('\n');
            }
            if let Some(gdir) = crate::memory::global_memory_dir() {
                if let Some(glob) =
                    crate::memory::scope_listing(&crate::exec::LocalExecutor, &gdir, "Global").await
                {
                    mem.push_str(&glob);
                    mem.push('\n');
                }
            }
            // Personal memory Clark extracted from the user's conversations
            // (read-only; best-effort — offline / missing scope degrades silently).
            if let Some(key) = &config.api_key {
                if let Ok(mems) =
                    crate::platform::recall_personal_memories(&config.base_url, key).await
                {
                    if let Some(sec) = crate::platform::personal_memory_section(&mems) {
                        mem.push_str(&sec);
                        mem.push('\n');
                    }
                }
            }
            prompt.push_str("\n# Memory\n");
            prompt.push_str(crate::memory::memory_guidance());
            if !mem.is_empty() {
                prompt.push('\n');
                prompt.push_str(&mem);
            }
        }
        // Project-scoped config (`.clark/settings.json`): permission arrays
        // union with the global (UI-driven) ones; deny always wins because
        // `PermissionGate::hard_refusal` checks `deny_commands` before
        // `command_preapproved` checks `allow_commands`.
        let project = crate::project_settings::load(self.executor.as_ref(), sandbox.root()).await;
        {
            let mut s = self.session.lock().await;
            s.system_prompt = prompt;
            s.transcript.clear();
            s.policy = config.permissions.clone();
            s.allow_commands = crate::project_settings::union_unique(
                config.command_allowlist.clone(),
                project.permissions.allow.clone(),
            );
            s.deny_commands = crate::project_settings::union_unique(
                config.command_denylist.clone(),
                project.permissions.deny.clone(),
            );
            s.output_style = String::new();
            s.hooks = project.hooks;
            s.check_command = project.check_command;
            s.diagnostics_baseline = None;
        }
        self.control.lock().await.clear();
        // A new session starts with no files "read".
        if let Ok(mut reads) = self.reads.lock() {
            *reads = ReadTracker::default();
        }
        // Kill any background tasks from a prior session on this same
        // provider instance (new_session reuses it across "new chat"/project
        // switches) — otherwise they'd leak past this point.
        self.background.clear_all().await;

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
        let config = self.config()?.clone();
        let max_iterations = config.max_iterations;

        // Fresh cancellation scope for this run — created early so it can also
        // gate the attachment pre-processing below (vision call / doc parsing).
        let cancel = CancellationToken::new();
        self.cancel = cancel.clone();

        let mut text = prompt_text(&input);
        text.push_str(
            &crate::attachments::process_attachments(
                &input.attachments,
                &text,
                config.vision.as_ref(),
                &cancel,
            )
            .await,
        );
        let text = {
            let s = self.session.lock().await;
            let mut text = text;
            let style = crate::prompt::output_style_instructions(&s.output_style);
            if !style.is_empty() {
                text = format!("{style}\n\n{text}");
            }
            if s.plan_mode {
                text = format!("{PLAN_MODE_REMINDER}\n\n{text}");
            }
            text
        };

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
                background: self.background.clone(),
                session: self.session.clone(),
            },
            session: self.session.clone(),
            control: self.control.clone(),
            session_id,
            max_iterations,
            compaction: config.compaction,
            model: config.model,
            temperature: config.temperature,
            user_text: text,
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

    async fn set_mode(&mut self, _session: &SessionId, mode: String) -> Result<()> {
        self.session.lock().await.plan_mode = mode == "plan";
        Ok(())
    }

    async fn set_output_style(&mut self, _session: &SessionId, style: String) -> Result<()> {
        self.session.lock().await.output_style = style;
        Ok(())
    }
}

/// Flatten a prompt's text blocks (and inline any text attachments) into one
/// user message. Non-text attachments (images, PDFs, DOCX, anything else) are
/// handled separately by [`crate::attachments::process_attachments`], which
/// needs an async context this sync helper doesn't have.
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
    fn prompt_text_does_not_note_non_text_attachments() {
        // A non-text attachment (e.g. an image) must never get a bare
        // filename note here — that's exactly what previously sent the model
        // hunting the filesystem for a file that only existed as inline
        // base64. Non-text handling now lives in `crate::attachments`.
        let input = PromptInput {
            blocks: vec![ContentBlock::text("look at this")],
            attachments: vec![PendingUpload {
                filename: "image.webp".into(),
                content_type: "image/webp".into(),
                data_base64: "aGVsbG8=".into(),
            }],
        };
        let text = prompt_text(&input);
        assert!(!text.contains("attached file:"));
        assert!(!text.contains("image.webp"));
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
    async fn set_mode_flips_plan_mode_flag() {
        let mut p = LocalAgentProvider::new();
        let session_id = SessionId::new("s1");
        assert!(!p.session.lock().await.plan_mode);

        p.set_mode(&session_id, "plan".to_string()).await.unwrap();
        assert!(p.session.lock().await.plan_mode);

        p.set_mode(&session_id, "ask".to_string()).await.unwrap();
        assert!(!p.session.lock().await.plan_mode);
    }

    #[tokio::test]
    async fn set_output_style_persists_on_session_state() {
        let mut p = LocalAgentProvider::new();
        let session_id = SessionId::new("s1");
        assert_eq!(p.session.lock().await.output_style, "");

        p.set_output_style(&session_id, "terse".to_string())
            .await
            .unwrap();
        assert_eq!(p.session.lock().await.output_style, "terse");
    }

    #[tokio::test]
    async fn new_session_seeds_system_prompt_without_history() {
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
        assert!(!s.system_prompt.is_empty());
        assert!(s.transcript.is_empty());
    }
}
