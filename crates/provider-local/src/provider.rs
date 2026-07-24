//! [`LocalAgentProvider`] — the local coding agent behind the `agent_core`
//! `Provider` trait. Connect sets the model endpoint + tool registry; each
//! session is bound to a project root; each prompt drives a local tool-calling
//! loop ([`crate::engine`]) whose normalized events stream back to the UI.

mod cancellation;
mod isolation;
mod isolation_setup;
mod prompt_input;
mod side_question;
mod state;

use std::sync::atomic::Ordering;
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

use cancellation::ManualCompactionRegistration;
pub(crate) use cancellation::RunCancellationRegistry;
use isolation::ProviderIsolation;
use isolation_setup::build_local_executor;
pub use isolation_setup::local_sandbox_setup_policy;
use prompt_input::*;
pub use state::LocalAgentProvider;

// The Plan Mode workflow reminder and its exit note live in `planning` — injected
// per-turn below (never baked into the cached system-prompt prefix) since the
// mode can flip mid-session via Shift+Tab or a plan approval.

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
        if local.computer_use_enabled
            && local.remote.is_none()
            && !self.isolation.disposable_writer()
        {
            let backend: Arc<dyn computer_use::ComputerBackend> = match local.computer_use_backend {
                crate::config::ComputerUseBackend::Native => computer_use::native_backend()
                    .map_err(|error| {
                        Error::Unsupported(format!("computer use is unavailable: {error}"))
                    })?,
                crate::config::ComputerUseBackend::Simulated => {
                    Arc::new(computer_use::SimulatedComputerBackend::new())
                }
            };
            registry.enable_computer_use(backend);
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
        let session_mode = options.mode.clone();
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
                exec_sandbox::SandboxPreset::for_session_mode(session_mode.as_deref())
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
        self.session_mode = session_mode.clone();
        let sandbox = Arc::new(sandbox);

        // Configured MCP stdio servers are trusted connector processes. Their
        // individual tool calls remain permission-gated as External, but a
        // network connector itself cannot work inside the offline project
        // sandbox. Keep remote connectors on the remote host; locally, launch
        // the configured server on the host without widening the session's
        // normal file/shell executor.
        if !config.mcp_servers.is_empty() && self.mcp_status.is_empty() {
            let registry = self
                .registry
                .as_mut()
                .and_then(Arc::get_mut)
                .ok_or_else(|| Error::Other("tool registry is already shared".to_string()))?;
            let mcp_executor: Arc<dyn Executor> = if config.remote.is_some() {
                self.executor.clone()
            } else {
                Arc::new(LocalExecutor)
            };
            self.mcp_status = registry
                .connect_mcp(&config.mcp_servers, mcp_executor.as_ref(), sandbox.root())
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

        // Project-scoped config (`.clark/settings.json`) mirrors Claude Code's
        // configurable commit attribution and also layers permissions/hooks
        // beneath the global UI-driven config.
        let project = if self.isolation.disposable_writer() {
            crate::project_settings::ProjectSettings::default()
        } else {
            crate::project_settings::load(self.executor.as_ref(), sandbox.root()).await
        };

        let available_tools = self
            .registry
            .as_ref()
            .map(|registry| registry.tool_names())
            .unwrap_or_default();
        let skill_project_root = self
            .executor
            .canonicalize(sandbox.root())
            .await
            .unwrap_or_else(|_| sandbox.root().to_path_buf());
        let skill_environment_id = crate::skills::skill_environment_id(
            &skill_project_root,
            config.remote.as_ref().map(|remote| remote.ws_url.as_str()),
        );
        let skills = if self.isolation.disposable_writer() {
            Arc::new(crate::skills::SkillCatalog::default())
        } else {
            self.skill_catalogs
                .refresh_for_provider(
                    self.executor.as_ref(),
                    &skill_project_root,
                    &skill_environment_id,
                    &available_tools,
                    &project.skills.disabled,
                )
                .await
        };
        let registry = self
            .registry
            .as_mut()
            .and_then(Arc::get_mut)
            .ok_or_else(|| Error::Other("tool registry is already shared".to_string()))?;
        if skills.enabled().next().is_some() {
            registry.enable_skills(skills.clone());
        } else {
            registry.disable_skills();
        }
        self.skills = skills;
        self.skill_environment_id = Some(skill_environment_id);
        self.skill_disabled_names = project.skills.disabled.clone();

        let commit_attribution = project
            .include_git_instructions()
            .then(|| project.commit_attribution());
        let mut prompt = system_prompt(
            &sandbox,
            config.clark.is_some(),
            config.remote.is_some(),
            commit_attribution,
        );
        if let Some(docs) = sandbox.docs_root() {
            prompt.push_str(&crate::workspace::prompt_section(docs));
        }
        if let Some(catalog) = crate::skills::render_catalog(&self.skills) {
            prompt.push_str(&catalog);
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
        {
            let mut s = self.session.lock().await;
            s.system_prompt = prompt;
            s.transcript = resumed_transcript;
            s.planning = crate::planning::PlanningState::default();
            s.planning.mode = collaboration_mode;
            s.planning.proposed_plan = restored_proposed_plan;
            s.deferred_tools.clear();
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
        self.llm = self.llm.take().map(|llm| llm.with_session_id(id.as_str()));
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
            mode: session_mode,
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
        if self.manual_compacting.load(Ordering::Acquire) {
            return Err(Error::Unsupported(
                "wait for context compaction to finish before sending a message".into(),
            ));
        }
        let sandbox = self.sandbox.clone().ok_or(Error::NotConnected)?;
        if !self.isolation.disposable_writer() {
            let environment_id = self
                .skill_environment_id
                .as_deref()
                .ok_or_else(|| Error::Other("skill environment is not initialized".into()))?;
            let available_tools = self
                .registry
                .as_ref()
                .map(|registry| registry.tool_names())
                .unwrap_or_default();
            let skill_project_root = self
                .executor
                .canonicalize(sandbox.root())
                .await
                .unwrap_or_else(|_| sandbox.root().to_path_buf());
            let refreshed = self
                .skill_catalogs
                .refresh_for_provider(
                    self.executor.as_ref(),
                    &skill_project_root,
                    environment_id,
                    &available_tools,
                    &self.skill_disabled_names,
                )
                .await;
            let registry = self
                .registry
                .as_mut()
                .and_then(Arc::get_mut)
                .ok_or_else(|| Error::Other("tool registry is still in use".to_string()))?;
            if refreshed.enabled().next().is_some() {
                registry.enable_skills(refreshed.clone());
            } else {
                registry.disable_skills();
            }
            let rendered = crate::skills::render_catalog(&refreshed);
            crate::skills::replace_catalog_section(
                &mut self.session.lock().await.system_prompt,
                rendered.as_deref(),
            );
            self.skills = refreshed;
        }
        let llm = self.llm.clone().ok_or(Error::NotConnected)?;
        let registry = self.registry.clone().ok_or(Error::NotConnected)?;
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
        // One immutable catalog snapshot governs validation, injection, and
        // `read_skill` for this run. A refresh can affect the next run but
        // cannot change capability meaning while the model is acting.
        let run_skills = self.skills.clone();
        let selected_skill_sections = crate::skills::bound_skill_injections(
            self.executor.as_ref(),
            &run_skills,
            &input.blocks,
        )
        .await
        .map_err(Error::Other)?;
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
        context_sections.extend(selected_skill_sections);
        context_sections.extend(
            crate::skills::explicit_skill_injections(
                self.executor.as_ref(),
                &run_skills,
                &user_request,
            )
            .await,
        );
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
        let (approved_plan, collaboration_instruction) = {
            let mut s = self.session.lock().await;
            let style = crate::prompt::output_style_instructions(&s.output_style);
            if !style.is_empty() {
                context_sections.push(style.to_string());
            }
            if s.planning.plan_mode() {
                let instruction_kind = s.planning.next_plan_instruction_kind();
                let reminder = crate::planning::plan_mode_instruction_for(
                    config.planning_prompt_profile,
                    s.planning.proposed_plan.as_ref(),
                    instruction_kind,
                );
                (None, Some(reminder))
            } else if std::mem::take(&mut s.planning.exited) {
                let note = crate::planning::plan_mode_exit_note(s.planning.proposed_plan.as_ref());
                (s.planning.proposed_plan.clone(), Some(note))
            } else {
                (None, None)
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
                call_progress: None,
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
            developer_instructions: collaboration_instruction
                .into_iter()
                .map(crate::planning::developer_instruction_message)
                .collect(),
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

    async fn compact(&mut self, _session: &SessionId) -> Result<EventStream> {
        if self.run_cancellations.has_active() {
            return Err(Error::Unsupported(
                "wait for the active run to finish before compacting context".into(),
            ));
        }
        self.manual_compacting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Unsupported("context compaction is already running".into()))?;

        let llm = match self.llm.clone() {
            Some(llm) => llm,
            None => {
                self.manual_compacting.store(false, Ordering::Release);
                return Err(Error::NotConnected);
            }
        };
        let config = match self.config.as_ref() {
            Some(config) => config.compaction.clone(),
            None => {
                self.manual_compacting.store(false, Ordering::Release);
                return Err(Error::NotConnected);
            }
        };
        if self.session.lock().await.transcript.is_empty() {
            self.manual_compacting.store(false, Ordering::Release);
            return Err(Error::Unsupported(
                "this conversation has no model context to compact".into(),
            ));
        }

        let run = RunId::new(format!(
            "run-{}",
            self.run_counter.fetch_add(1, Ordering::SeqCst) + 1
        ));
        let cancel = CancellationToken::new();
        self.run_cancellations.register(&run, cancel.clone());
        let registration = ManualCompactionRegistration {
            registry: self.run_cancellations.clone(),
            run: run.clone(),
            latch: self.manual_compacting.clone(),
        };
        let session = self.session.clone();
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();
        tokio::spawn(async move {
            let _registration = registration;
            crate::compaction::run_manual_compaction(llm, config, session, tx, run, cancel).await;
        });
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
        let plan_mode = self.session.lock().await.planning.plan_mode();
        if !plan_mode {
            if let (Some(config), Some(sandbox)) = (self.config.as_ref(), self.sandbox.as_ref()) {
                if config.remote.is_none() {
                    let preset = exec_sandbox::SandboxPreset::for_session_mode(Some(&mode));
                    let (executor, sandbox_temp) = build_local_executor(config, sandbox, preset)?;
                    self.executor = executor;
                    self.sandbox_temp = sandbox_temp;
                }
            }
        }
        self.session_mode = Some(mode);
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
                    CollaborationMode::Default => {
                        exec_sandbox::SandboxPreset::for_session_mode(self.session_mode.as_deref())
                    }
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

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
