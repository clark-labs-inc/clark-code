//! [`LocalAgentProvider`] — the local coding agent behind the `agent_core`
//! `Provider` trait. Connect sets the model endpoint + tool registry; each
//! session is bound to a project root; each prompt drives a local tool-calling
//! loop ([`crate::engine`]) whose normalized events stream back to the UI.

mod prompt_input;
mod state;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_core::domain::AgentEvent;
#[cfg(test)]
use agent_core::domain::{ContentBlock, Role};
use agent_core::error::{Error, Result};
use agent_core::ids::{ProviderId, RunId, SessionId};
use agent_core::provider::{
    ClientResponse, EventStream, PromptInput, Provider, ProviderCapabilities, ProviderConfig,
    Session, SessionEnvironment, SessionOptions,
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

use prompt_input::*;

// The Plan Mode workflow reminder and its exit note live in
// `crate::prompt::{plan_mode_reminder, plan_mode_exit_note}` — injected
// per-turn below (never baked into the cached system-prompt prefix) since the
// mode can flip mid-session via Shift+Tab or a plan approval.

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
    /// Stable identity for the active project, when private project knowledge
    /// is enabled and the selected root is a Git repository.
    repository_fingerprint: Option<String>,
    instruction_snapshot: Option<crate::instructions::ProjectInstructions>,
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
            modes: vec![
                "ask".to_string(),
                "auto".to_string(),
                "full".to_string(),
                "plan".to_string(),
            ],
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
        if let Some(api_key) = local.api_key.clone() {
            registry.enable_organization_knowledge(
                crate::tools::organization_knowledge::OrganizationKnowledgeConfig {
                    base_url: local.base_url.clone(),
                    api_key,
                },
            );
        }
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

        self.repository_fingerprint = if config.project_knowledge_enabled {
            crate::repository::inspect_repository(self.executor.as_ref(), sandbox.root())
                .await
                .ok()
                .flatten()
                .map(|repository| repository.fingerprint)
        } else {
            None
        };

        let mut prompt = system_prompt(&sandbox, config.clark.is_some());
        self.instruction_snapshot =
            crate::instructions::load(self.executor.as_ref(), sandbox.root())
                .await
                .ok()
                .flatten();
        if let Some(instructions) = self.instruction_snapshot.as_ref() {
            prompt.push('\n');
            prompt.push_str(&instructions.render());
            prompt.push('\n');
        }
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
                Some(sandbox.root()),
            )
            .await
            {
                mem.push_str(&proj);
                mem.push('\n');
            }
            if let Some(gdir) = crate::memory::global_memory_dir() {
                if let Some(glob) =
                    crate::memory::scope_listing(&crate::exec::LocalExecutor, &gdir, "Global", None)
                        .await
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
                    let mems = crate::platform::scope_personal_memories(
                        mems,
                        self.repository_fingerprint.as_deref(),
                    );
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
        let resumed_transcript = crate::resume::to_agent_messages(options.resume.as_ref());
        // Project-scoped config (`.clark/settings.json`): permission arrays
        // union with the global (UI-driven) ones; deny always wins because
        // `PermissionGate::hard_refusal` checks `deny_commands` before
        // `command_preapproved` checks `allow_commands`.
        let project = crate::project_settings::load(self.executor.as_ref(), sandbox.root()).await;
        {
            let mut s = self.session.lock().await;
            s.system_prompt = prompt;
            s.transcript = resumed_transcript;
            // The session starts in the mode the client asked for (and a
            // provider instance reused across "new chat" must not inherit a
            // stale plan_mode from its previous session).
            s.plan_mode = options.mode.as_deref() == Some("plan");
            s.plan_exited = false;
            s.steering = None;
            s.goal = None;
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
        let sandbox = self.sandbox.as_ref().expect("sandbox was just installed");
        let checkout_root = sandbox.root().to_string_lossy().into_owned();
        let docs_root = sandbox
            .docs_root()
            .map(|root| root.to_string_lossy().into_owned());
        let mut workspace_roots = vec![checkout_root.clone()];
        if let Some(docs_root) = docs_root.as_ref() {
            workspace_roots.push(docs_root.clone());
        }
        let repository_root =
            crate::git_metadata::common_repository_root(self.executor.as_ref(), sandbox.root())
                .await
                .ok()
                .flatten()
                .map(|root| root.to_string_lossy().into_owned());
        Ok(Session {
            id,
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: options.mode,
            environment: Some(SessionEnvironment {
                checkout_root: Some(checkout_root),
                repository_root,
                workspace_roots,
                docs_root,
                remote: config.remote.is_some(),
            }),
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
        if let Ok(current_instructions) =
            crate::instructions::load(self.executor.as_ref(), sandbox.root()).await
        {
            if let Some(refresh) = crate::instructions::refresh_context(
                self.instruction_snapshot.as_ref(),
                current_instructions.as_ref(),
            ) {
                text = format!("{refresh}\n\n{text}");
            }
            self.instruction_snapshot = current_instructions;
        }
        text = format!(
            "{}\n\n{text}",
            environment_context(&sandbox, config.remote.is_some())
        );
        let knowledge_query = text.clone();
        let attachment_context = crate::attachments::process_attachments(
            &input.attachments,
            &text,
            config.vision.as_ref(),
            &cancel,
        );
        let repository_context = async {
            if !config.project_knowledge_enabled {
                return None;
            }
            let api_key = config.api_key.as_deref()?;
            let fingerprint = self.repository_fingerprint.as_deref()?;
            let context = crate::platform::recall_repository_context(
                &config.base_url,
                api_key,
                fingerprint,
                &knowledge_query,
            )
            .await
            .ok()?;
            crate::platform::repository_context_section(&context)
        };

        // The tree may be shared with other agents, so git state is re-taken
        // per turn (a session-start snapshot would go stale) and lands in the
        // turn message, keeping the cached system-prompt prefix stable.
        let git_snapshot =
            crate::repository::working_tree_snapshot(self.executor.as_ref(), sandbox.root());

        // Attachment extraction/vision, repository recall, and the git
        // snapshot are independent preflight work. Overlap them so
        // first-token latency is bounded by the slowest branch instead of
        // adding the durations together.
        let (attachment_context, repository_context, git_snapshot) =
            tokio::join!(attachment_context, repository_context, git_snapshot);
        text.push_str(&attachment_context);
        if let Some(section) = repository_context {
            text = format!("{section}\n\nUser request:\n{text}");
        }
        if let Some(git) = git_snapshot {
            text = format!("{git}\n{text}");
        }
        let docs_root = self
            .sandbox
            .as_ref()
            .and_then(|sb| sb.docs_root())
            .map(std::path::Path::to_path_buf);
        let text = {
            let mut s = self.session.lock().await;
            let mut text = text;
            let style = crate::prompt::output_style_instructions(&s.output_style);
            if !style.is_empty() {
                text = format!("{style}\n\n{text}");
            }
            if s.plan_mode {
                let reminder = crate::prompt::plan_mode_reminder(docs_root.as_deref());
                text = format!("{reminder}\n\n{text}");
            } else if std::mem::take(&mut s.plan_exited) {
                let note = crate::prompt::plan_mode_exit_note(docs_root.as_deref());
                text = format!("{note}\n\n{text}");
            }
            text
        };

        let run = RunId::new(format!(
            "run-{}",
            self.run_counter.fetch_add(1, Ordering::SeqCst) + 1
        ));
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();

        // Post-turn durable-fact extraction (structural memory proactivity):
        // only when memories are on, and always off the turn's latency path.
        // Extraction quality shouldn't inherit a weaker session model — on the
        // Clark platform, pin it to the default clark-code tier.
        let memory_extraction = config.memories_enabled.then(|| {
            let extraction_llm = if config.model.starts_with("clark-code") {
                llm.clone().with_model("clark-code")
            } else {
                llm.clone()
            };
            crate::memory_extraction::ExtractionCtx {
                llm: extraction_llm,
                executor: self.executor.clone(),
                project_root: sandbox.root().to_path_buf(),
                global_dir: crate::memory::global_memory_dir(),
            }
        });

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
                // Filled in per tool call by `DesktopToolAdapter::execute`,
                // which owns the call's update sink.
                progress: None,
            },
            session: self.session.clone(),
            control: self.control.clone(),
            session_id,
            max_iterations,
            compaction: config.compaction,
            model: config.model,
            temperature: config.temperature,
            user_text: text,
            memory_extraction,
        };
        tokio::spawn(run_turn(tc, tx, run));
        Ok(rx.boxed())
    }

    async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> Result<()> {
        self.cancel.cancel();
        self.control.lock().await.clear();
        Ok(())
    }

    async fn close_session(&mut self, _session: &SessionId) -> Result<()> {
        self.cancel.cancel();
        self.control.lock().await.clear();
        self.background.clear_all().await;
        Ok(())
    }

    async fn respond(&mut self, _session: &SessionId, response: ClientResponse) -> Result<()> {
        match response {
            ClientResponse::Permission {
                request,
                option,
                feedback,
            } => {
                let resolution = crate::loop_state::Resolution {
                    decision: Decision::from_option(&option),
                    feedback: feedback.filter(|f| !f.trim().is_empty()),
                };
                self.control.lock().await.resolve(&request, resolution);
                Ok(())
            }
        }
    }

    async fn steer(&mut self, _session: &SessionId, input: PromptInput) -> Result<()> {
        let text = prompt_input::prompt_text(&input);
        if text.trim().is_empty() {
            return Err(Error::Unsupported("steering message was empty".into()));
        }
        let steering = self.session.lock().await.steering.clone();
        match steering {
            // Raw user text, no per-turn scaffolding (env/git context rides
            // the turn that's already in flight).
            Some(queue) => {
                queue.push_user_text(text);
                Ok(())
            }
            None => Err(Error::Unsupported(
                "no active run to steer — send it as a new message".into(),
            )),
        }
    }

    async fn set_mode(&mut self, _session: &SessionId, mode: String) -> Result<()> {
        let mut s = self.session.lock().await;
        let entering_plan = mode == "plan";
        // Leaving plan mode queues the one-shot "plan mode is off" note for the
        // next turn; re-entering cancels a queued note so a quick toggle
        // doesn't tell the model it both entered and exited.
        if s.plan_mode && !entering_plan {
            s.plan_exited = true;
        } else if entering_plan {
            s.plan_exited = false;
        }
        s.plan_mode = entering_plan;
        Ok(())
    }

    async fn set_output_style(&mut self, _session: &SessionId, style: String) -> Result<()> {
        self.session.lock().await.output_style = style;
        Ok(())
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
