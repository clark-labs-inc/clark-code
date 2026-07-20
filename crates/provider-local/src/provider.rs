//! [`LocalAgentProvider`] — the local coding agent behind the `agent_core`
//! `Provider` trait. Connect sets the model endpoint + tool registry; each
//! session is bound to a project root; each prompt drives a local tool-calling
//! loop ([`crate::engine`]) whose normalized events stream back to the UI.

mod isolation;
mod prompt_input;
mod state;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_core::domain::AgentEvent;
#[cfg(test)]
use agent_core::domain::{ContentBlock, Role};
use agent_core::error::{Error, Result};
use agent_core::ids::{ProviderId, RunId, SessionId};
use agent_core::provider::{
    ClientResponse, CollaborationMode, EventStream, PlanDecision, PromptInput, Provider,
    ProviderCapabilities, ProviderConfig, Session, SessionEnvironment, SessionOptions,
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

use isolation::ProviderIsolation;
use prompt_input::*;

// The Plan Mode workflow reminder and its exit note live in `planning` — injected
// per-turn below (never baked into the cached system-prompt prefix) since the
// mode can flip mid-session via Shift+Tab or a plan approval.

/// Run-addressed cancellation. A provider can have overlapping prompt tasks,
/// so a single "most recently assigned" token cannot implement
/// `Provider::cancel(session, run)` correctly.
#[derive(Clone, Default)]
pub(crate) struct RunCancellationRegistry {
    tokens: Arc<std::sync::Mutex<HashMap<String, CancellationToken>>>,
}

impl RunCancellationRegistry {
    fn register(&self, run: &RunId, token: CancellationToken) {
        self.tokens
            .lock()
            .expect("run cancellation registry lock")
            .insert(run.as_str().to_string(), token);
    }

    fn cancel(&self, run: &RunId) -> bool {
        let token = self
            .tokens
            .lock()
            .expect("run cancellation registry lock")
            .get(run.as_str())
            .cloned();
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub(crate) fn remove(&self, run: &RunId) {
        self.tokens
            .lock()
            .expect("run cancellation registry lock")
            .remove(run.as_str());
    }

    fn cancel_all(&self) {
        let tokens = self
            .tokens
            .lock()
            .expect("run cancellation registry lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for token in tokens {
            token.cancel();
        }
    }
}

pub struct LocalAgentProvider {
    isolation: ProviderIsolation,
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
    /// Cancellation token for prompt setup and the newest run. Exact run
    /// cancellation uses `run_cancellations` below.
    cancel: CancellationToken,
    run_cancellations: RunCancellationRegistry,
    /// Where this session's tool I/O runs — local today, remote (over the
    /// exec-server) once a remote project is selected. Chosen in `new_session`.
    executor: Arc<dyn crate::exec::Executor>,
    run_counter: AtomicU64,
    /// Last MCP connection result, surfaced to the settings UI.
    mcp_status: Vec<crate::mcp::McpStatus>,
    /// Stable identity for the active project, when private project knowledge
    /// is enabled and the selected root is a Git repository.
    repository_fingerprint: Option<String>,
    /// Owns the narrow temporary write root exposed to sandboxed children.
    sandbox_temp: Option<tempfile::TempDir>,
}

fn build_local_executor(
    config: &LocalConfig,
    sandbox: &Sandbox,
    preset: exec_sandbox::SandboxPreset,
) -> Result<(Arc<dyn Executor>, Option<tempfile::TempDir>)> {
    if config.sandbox_mode == crate::config::LocalSandboxMode::Disabled
        || preset == exec_sandbox::SandboxPreset::DangerFullAccess
    {
        return Ok((Arc::new(LocalExecutor), None));
    }

    let mut extra_write_roots = Vec::new();
    if let Some(docs) = sandbox.docs_root() {
        extra_write_roots.push(docs.to_path_buf());
    }
    #[cfg(windows)]
    if let Some(docs_root) = crate::workspace::workspace_root() {
        extra_write_roots.push(docs_root);
    }
    #[cfg(windows)]
    let private_temp = {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| Error::Io("LOCALAPPDATA is unavailable".to_string()))?
            .join("Clark Code")
            .join("sandbox-tmp");
        std::fs::create_dir_all(&base).map_err(|error| Error::Io(error.to_string()))?;
        extra_write_roots.push(base.clone());
        tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(base)
            .map_err(|error| Error::Io(error.to_string()))?
    };
    #[cfg(not(windows))]
    let private_temp = tempfile::Builder::new()
        .prefix("clark-sandbox-")
        .tempdir()
        .map_err(|error| Error::Io(error.to_string()))?;
    let policy = match preset {
        exec_sandbox::SandboxPreset::ReadOnly => {
            exec_sandbox::SandboxPolicy::read_only().with_write_roots(extra_write_roots)
        }
        exec_sandbox::SandboxPreset::WorkspaceWrite => {
            exec_sandbox::SandboxPolicy::workspace_write(
                sandbox.root().to_path_buf(),
                extra_write_roots,
            )
        }
        exec_sandbox::SandboxPreset::DangerFullAccess => unreachable!(),
    }
    .with_process_temp_root(private_temp.path().to_path_buf());
    let install = clark_install_context::InstallContext::current();
    let runtime = exec_sandbox::SandboxRuntime {
        linux_bubblewrap: install.bundled_tool(clark_install_context::BUBBLEWRAP),
        windows_runner: install.bundled_tool(clark_install_context::WINDOWS_SANDBOX_RUNNER),
        windows_setup: install.bundled_tool(clark_install_context::WINDOWS_SANDBOX_SETUP),
        windows_state_dir: None,
    };
    let manager =
        exec_sandbox::SandboxManager::current_with_runtime(policy.clone(), runtime.clone())
            .map_err(Error::Other)?;
    #[cfg(windows)]
    let manager = auto_enroll_windows_workspace(manager, policy, runtime)?;
    if matches!(
        manager.status(),
        exec_sandbox::SandboxStatus::Enforced { .. }
    ) {
        let executor =
            Arc::new(exec_sandbox::SandboxedExecutor::with_manager(manager).map_err(Error::Other)?);
        return Ok((executor, Some(private_temp)));
    }
    if config.sandbox_mode == crate::config::LocalSandboxMode::Required {
        return Err(Error::Unsupported(format!(
            "required local sandbox is not ready: {:?}",
            manager.status()
        )));
    }
    tracing::warn!(status = ?manager.status(), "local sandbox is not ready; using explicit host execution");
    Ok((Arc::new(LocalExecutor), None))
}

#[cfg(windows)]
fn auto_enroll_windows_workspace(
    manager: exec_sandbox::SandboxManager,
    policy: exec_sandbox::SandboxPolicy,
    runtime: exec_sandbox::SandboxRuntime,
) -> Result<exec_sandbox::SandboxManager> {
    if !matches!(
        manager.status(),
        exec_sandbox::SandboxStatus::SetupRequired { .. }
    ) {
        return Ok(manager);
    }
    let Some(action) = manager.setup_action().map_err(Error::Other)? else {
        return Ok(manager);
    };
    if action.requires_elevation {
        for path in action.cleanup_paths {
            let _ = std::fs::remove_file(path);
        }
        return Ok(manager);
    }
    match exec_sandbox_windows::run_setup_action(
        &action.program,
        &action.args,
        false,
        action.cleanup_paths,
    ) {
        Ok(()) => exec_sandbox::SandboxManager::current_with_runtime(policy, runtime)
            .map_err(Error::Other),
        Err(error) => {
            tracing::warn!(
                error,
                "automatic user-mode Windows workspace enrollment failed"
            );
            Ok(manager)
        }
    }
}

/// Stable policy used by the desktop's explicit Windows setup flow. Session
/// directories nest under these roots, so one consented ACL reconciliation is
/// reusable without broadening access beyond Clark's project/docs/temp areas.
pub fn local_sandbox_setup_policy(cwd: &std::path::Path) -> Result<exec_sandbox::SandboxPolicy> {
    #[cfg(not(windows))]
    let write_roots = Vec::new();
    #[cfg(windows)]
    let mut write_roots = Vec::new();
    #[cfg(windows)]
    {
        if let Some(docs_root) = crate::workspace::workspace_root() {
            write_roots.push(docs_root);
        }
        let temp_root = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| Error::Io("LOCALAPPDATA is unavailable".to_string()))?
            .join("Clark Code")
            .join("sandbox-tmp");
        write_roots.push(temp_root);
    }
    Ok(exec_sandbox::SandboxPolicy::workspace_write(
        cwd.to_path_buf(),
        write_roots,
    ))
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
            modes: vec!["ask".to_string(), "auto".to_string(), "full".to_string()],
            collaboration_modes: vec![CollaborationMode::Default, CollaborationMode::Plan],
        }
    }

    async fn connect(&mut self, config: ProviderConfig) -> Result<()> {
        self.isolation = ProviderIsolation::from_provider_config(&config);
        let mut local = LocalConfig::from_provider_config(&config);
        // Child writers must never see orchestration tools or policy, even now
        // that the root capability is available by default.
        if self.isolation.disposable_writer() {
            local.orchestration.enabled = false;
        }
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
            registry.enable_image_generation(crate::tools::image::ImageGenerationConfig {
                base_url: local.base_url.clone(),
                api_key,
            });
        }
        if local.remote.is_some() {
            registry.disable_desktop_mobile_tools();
        }
        if !self.isolation.disposable_writer() {
            if let Some(api_key) = local.api_key.clone() {
                registry.enable_organization_knowledge(
                    crate::tools::organization_knowledge::OrganizationKnowledgeConfig {
                        base_url: local.base_url.clone(),
                        api_key,
                    },
                );
            }
        }
        if local.browser_enabled {
            registry.enable_browser();
        }
        if local.orchestration.enabled {
            registry.enable_orchestration(
                crate::orchestration::OrchestrationToolsConfig::from_local(&local),
            );
        }
        self.llm = Some(llm);
        self.registry = Some(Arc::new(registry));
        self.config = Some(local);
        Ok(())
    }

    async fn new_session(&mut self, options: SessionOptions) -> Result<Session> {
        let config = self.config()?.clone();
        let collaboration_mode = options.collaboration_mode.unwrap_or_default();
        let restored_goal = options.resume.as_ref().and_then(|resume| {
            resume.items.iter().rev().find_map(|item| match item {
                agent_core::provider::ResumeItem::Goal { goal } => Some(goal.clone()),
                _ => None,
            })
        });
        let restored_proposed_plan = options.resume.as_ref().and_then(|resume| {
            resume.items.iter().rev().find_map(|item| match item {
                agent_core::provider::ResumeItem::ProposedPlan { plan } => Some(plan.clone()),
                _ => None,
            })
        });

        let id = SessionId::new(uuid::Uuid::new_v4().to_string());
        let sandbox_preset = match collaboration_mode {
            CollaborationMode::Plan => exec_sandbox::SandboxPreset::ReadOnly,
            CollaborationMode::Default => {
                exec_sandbox::SandboxPreset::for_session_mode(options.mode.as_deref())
            }
        };

        // A remote project runs its tools on a remote host over the exec-server;
        // a local project runs them here. Resolve every writable root before
        // selecting the executor so one immutable policy reaches both direct
        // filesystem operations and child-process compilation.
        let (sandbox, executor, sandbox_temp): (
            Sandbox,
            Arc<dyn Executor>,
            Option<tempfile::TempDir>,
        ) = if let Some(remote) = &config.remote {
            let sandbox = Sandbox::new_remote(&remote.cwd).map_err(Error::Other)?;
            let exec = RemoteExecutor::connect(&remote.ws_url, &remote.token)
                .await
                .map_err(Error::Other)?;
            (sandbox, Arc::new(exec), None)
        } else {
            let cwd = options.cwd.or(config.cwd.clone()).ok_or_else(|| {
                Error::Unsupported("local provider requires a project `cwd`".into())
            })?;
            let mut sandbox = Sandbox::new(&cwd).map_err(Error::Io)?;
            if !self.isolation.disposable_writer() {
                if let Some(workspace) = crate::workspace::session_workspace(id.as_str()) {
                    if std::fs::create_dir_all(&workspace).is_ok() {
                        sandbox = sandbox.with_docs(workspace);
                    }
                }
            }

            let (executor, sandbox_temp) = build_local_executor(&config, &sandbox, sandbox_preset)?;
            (sandbox, executor, sandbox_temp)
        };
        self.executor = executor;
        self.sandbox_temp = sandbox_temp;
        let sandbox = Arc::new(sandbox);

        // MCP stdio servers are agent-owned subprocesses. Start them only after
        // the session has a canonical root and scoped executor, so their launch
        // passes through the same OS sandbox as shell and helper processes.
        if !config.mcp_servers.is_empty() && self.mcp_status.is_empty() {
            let registry = self
                .registry
                .as_mut()
                .and_then(Arc::get_mut)
                .ok_or_else(|| Error::Other("tool registry is already shared".to_string()))?;
            self.mcp_status = registry
                .connect_mcp(&config.mcp_servers, self.executor.as_ref(), sandbox.root())
                .await;
        }

        self.repository_fingerprint = if config.project_knowledge_enabled {
            crate::repository::inspect_repository(self.executor.as_ref(), sandbox.root())
                .await
                .ok()
                .flatten()
                .map(|repository| repository.fingerprint)
        } else {
            None
        };

        let mut prompt = system_prompt(&sandbox, config.clark.is_some(), config.remote.is_some());
        if let Some(docs) = sandbox.docs_root() {
            prompt.push_str(&crate::workspace::prompt_section(docs));
        }
        // Surface compatible Codex and Claude skills through the session
        // executor — local disk or the remote host over the SSH tunnel.
        if !self.isolation.disposable_writer() {
            if let Some(skills) = crate::external_import::skills_prompt_section(
                self.executor.as_ref(),
                sandbox.root(),
            )
            .await
            {
                prompt.push_str(&skills);
            }
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
        let project = if self.isolation.disposable_writer() {
            crate::project_settings::ProjectSettings::default()
        } else {
            crate::project_settings::load(self.executor.as_ref(), sandbox.root()).await
        };
        {
            let mut s = self.session.lock().await;
            s.system_prompt = prompt;
            s.transcript = resumed_transcript;
            s.planning = crate::planning::PlanningState::default();
            s.planning.mode = collaboration_mode;
            s.planning.proposed_plan = restored_proposed_plan;
            s.steering = None;
            s.active_execution = None;
            s.goal = restored_goal.map(crate::loop_state::SessionGoal::from_state);
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
            collaboration_mode,
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

        let parts = prompt_parts(&input);
        let knowledge_query = prompt_text(&input);
        let user_request = parts.user_request;
        let goal_command = goal_command_objective(&user_request);
        if let Some(objective) = goal_command.as_ref() {
            let mut session = self.session.lock().await;
            crate::tools::goal::start_goal(&mut session, objective.clone(), None)
                .map_err(Error::Other)?;
        }
        let native_image_support = crate::config::model_supports_images(&config.model);
        let mut context_sections = Vec::new();
        if config.orchestration.enabled {
            context_sections.push(
                crate::orchestration::turn_policy_section(config.orchestration.mode).to_string(),
            );
        }
        if let Ok(current_instructions) =
            crate::instructions::load(self.executor.as_ref(), sandbox.root()).await
        {
            if let Some(instructions) = current_instructions.as_ref() {
                context_sections.push(instructions.render());
            }
        }
        context_sections.push(environment_context(&sandbox, config.remote.is_some()));
        if !parts.text_attachment_context.is_empty() {
            context_sections.push(parts.text_attachment_context);
        }
        if goal_command.is_some() {
            context_sections.push(goal_command_context());
        }
        let attachment_context = crate::attachments::process_attachments(
            &input.attachments,
            &knowledge_query,
            config.vision.as_ref(),
            native_image_support,
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
        if let Some(section) = repository_context {
            context_sections.push(section);
        }
        if let Some(git) = git_snapshot {
            context_sections.push(git);
        }
        if !attachment_context.trim().is_empty() {
            context_sections.push(attachment_context);
        }
        let approved_plan = {
            let mut s = self.session.lock().await;
            let style = crate::prompt::output_style_instructions(&s.output_style);
            if !style.is_empty() {
                context_sections.push(style.to_string());
            }
            if s.planning.plan_mode() {
                let reminder = crate::planning::plan_mode_instructions_for(
                    config.planning_prompt_profile,
                    s.planning.proposed_plan.as_ref(),
                );
                context_sections.push(reminder);
                None
            } else if std::mem::take(&mut s.planning.exited) {
                let note = crate::planning::plan_mode_exit_note(s.planning.proposed_plan.as_ref());
                context_sections.push(note);
                s.planning.proposed_plan.clone()
            } else {
                None
            }
        };
        let text = assemble_turn_prompt(&context_sections, &user_request);
        let user_content = prompt_input::model_user_content(
            text.clone(),
            &input.attachments,
            native_image_support,
        );

        let run = RunId::new(format!(
            "run-{}",
            self.run_counter.fetch_add(1, Ordering::SeqCst) + 1
        ));
        self.run_cancellations.register(&run, cancel.clone());
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
                agent_progress: None,
            },
            session: self.session.clone(),
            control: self.control.clone(),
            session_id,
            max_iterations,
            compaction: config.compaction,
            model: config.model,
            temperature: config.temperature,
            user_text: text,
            user_content,
            initial_events: approved_plan
                .into_iter()
                .map(|plan| AgentEvent::ProposedPlanUpdated {
                    run: run.clone(),
                    plan,
                })
                .collect(),
            memory_extraction,
            execution: config.execution,
            run_cancellations: self.run_cancellations.clone(),
            tool_image_policy: crate::agent_adapter::ToolImagePolicy {
                native_image_support,
                vision: config.vision.clone(),
            },
        };
        tokio::spawn(run_turn(tc, tx, run));
        Ok(rx.boxed())
    }

    async fn cancel(&mut self, _session: &SessionId, run: &RunId) -> Result<()> {
        if !self.run_cancellations.cancel(run) {
            return Err(Error::Other(format!(
                "no active local run named {}",
                run.as_str()
            )));
        }
        self.control.lock().await.clear();
        Ok(())
    }

    async fn close_session(&mut self, _session: &SessionId) -> Result<()> {
        self.cancel.cancel();
        self.run_cancellations.cancel_all();
        self.control.lock().await.clear();
        self.background.clear_all().await;
        self.sandbox_temp = None;
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
            ClientResponse::PlanDecision { plan_id, decision } => {
                let (implement, fresh) = match decision {
                    PlanDecision::Implement { context } => (
                        true,
                        context == agent_core::provider::PlanImplementationContext::Fresh,
                    ),
                    PlanDecision::ContinuePlanning { .. } => (false, false),
                };
                {
                    let mut session = self.session.lock().await;
                    let plan = session
                        .planning
                        .proposed_plan
                        .as_mut()
                        .filter(|plan| plan.id == plan_id)
                        .ok_or_else(|| Error::Other("no matching proposed plan".into()))?;
                    if implement {
                        plan.status = agent_core::domain::ProposedPlanStatus::Approved;
                        session.planning.set_mode(CollaborationMode::Default);
                        if fresh {
                            session.transcript.clear();
                        }
                    } else {
                        session.planning.set_mode(CollaborationMode::Plan);
                    }
                }
                if implement {
                    if let (Some(config), Some(sandbox)) =
                        (self.config.as_ref(), self.sandbox.as_ref())
                    {
                        if config.remote.is_none() {
                            let (executor, sandbox_temp) = build_local_executor(
                                config,
                                sandbox,
                                exec_sandbox::SandboxPreset::WorkspaceWrite,
                            )?;
                            self.executor = executor;
                            self.sandbox_temp = sandbox_temp;
                        }
                    }
                }
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
        if let (Some(config), Some(sandbox)) = (self.config.as_ref(), self.sandbox.as_ref()) {
            if config.remote.is_none() {
                let preset = exec_sandbox::SandboxPreset::for_session_mode(Some(&mode));
                let (executor, sandbox_temp) = build_local_executor(config, sandbox, preset)?;
                self.executor = executor;
                self.sandbox_temp = sandbox_temp;
            }
        }
        Ok(())
    }

    async fn set_collaboration_mode(
        &mut self,
        _session: &SessionId,
        mode: CollaborationMode,
    ) -> Result<()> {
        if let (Some(config), Some(sandbox)) = (self.config.as_ref(), self.sandbox.as_ref()) {
            if config.remote.is_none() {
                let preset = match mode {
                    CollaborationMode::Plan => exec_sandbox::SandboxPreset::ReadOnly,
                    CollaborationMode::Default => exec_sandbox::SandboxPreset::WorkspaceWrite,
                };
                let (executor, sandbox_temp) = build_local_executor(config, sandbox, preset)?;
                self.executor = executor;
                self.sandbox_temp = sandbox_temp;
            }
        }
        self.session.lock().await.planning.set_mode(mode);
        Ok(())
    }

    async fn set_output_style(&mut self, _session: &SessionId, style: String) -> Result<()> {
        self.session.lock().await.output_style = style;
        Ok(())
    }

    async fn side_question(&mut self, _session: &SessionId, question: &str) -> Result<String> {
        self.side_question_impl(question).await
    }
}

/// Map a no-tools side-question LLM failure to the engine's error vocabulary.
/// Cancelled is silent (the user dismissed the overlay); credit/auth failures
/// keep their typed shape so the UI can prompt appropriately; everything else
/// becomes a transport error.
fn map_llm_error(error: crate::llm::LlmError) -> Error {
    match error {
        crate::llm::LlmError::Cancelled => Error::Other("side question cancelled".into()),
        crate::llm::LlmError::InsufficientCredits => Error::Other("insufficient_credits".into()),
        crate::llm::LlmError::PlatformKeyRejected(message) => {
            Error::Other(format!("platform key rejected: {message}"))
        }
        crate::llm::LlmError::RateLimited(message) => Error::Transport(message),
        crate::llm::LlmError::Transport(message) => Error::Transport(message),
        crate::llm::LlmError::Provider(message) => Error::Other(message),
        crate::llm::LlmError::ContextOverflow(message) => Error::Other(message),
    }
}

impl LocalAgentProvider {
    /// `/btw` — answer a one-off side question against the session's current
    /// context WITHOUT interrupting the active run or mutating session state
    /// (a forked, single-turn, tool-less model call; ported from Claude
    /// Code's `runSideQuestion`).
    ///
    /// Snapshot the session's system prompt + transcript by clone under the
    /// session lock, release, then build the wire messages lock-free and run a
    /// single no-tools `stream_chat`. Nothing is written back into `transcript`
    /// (or `reads`/`control`/`run_counter`), so the active run — if any — is
    /// untouched and keeps streaming into its own event channel.
    async fn side_question_impl(&self, question: &str) -> Result<String> {
        let llm = self.llm.clone().ok_or(Error::NotConnected)?;
        let (system_prompt, transcript) = {
            let s = self.session.lock().await;
            (s.system_prompt.clone(), s.transcript.clone())
        };

        let wrapped = format!(
            "<system-reminder>This is a side question from the user. Answer it directly in a \
             single response.\n\nIMPORTANT CONTEXT:\n- You are a separate, lightweight agent \
             spawned to answer this one question.\n- The main agent is NOT interrupted — it \
             continues working independently in the background.\n- You share the conversation \
             context but are a completely separate instance.\n- Do NOT reference being \
             interrupted or what you were \"previously doing\" — that framing is incorrect.\n\n\
             CRITICAL CONSTRAINTS:\n- You have NO tools available — you cannot read files, run \
             commands, search, or take any actions.\n- This is a one-off response — there will \
             be no follow-up turns.\n- You can ONLY provide information based on what you already \
             know from the conversation context.\n- NEVER say things like \"Let me…\", \
             \"I'll now…\", or promise to take any action.\n- If you don't know the answer, say \
             so — do not offer to look it up or investigate.\n\nSimply answer the question with \
             the information you have.</system-reminder>\n\n{question}"
        );

        let mut messages = crate::agent_adapter::to_wire_messages(&system_prompt, &transcript);
        messages.push(crate::llm::ChatMessage::user(wrapped));

        let cancel = CancellationToken::new();
        let turn = llm
            .stream_chat(&messages, &[], &cancel, |_| {}, |_| {})
            .await
            .map_err(map_llm_error)?;
        Ok(turn.text)
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
