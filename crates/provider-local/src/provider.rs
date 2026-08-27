//! [`LocalAgentProvider`] — the local coding agent behind the `agent_core`
//! `Provider` trait. Connect sets the model endpoint + tool registry; each
//! session is bound to a project root; each prompt drives a local tool-calling
//! loop ([`crate::engine`]) whose normalized events stream back to the UI.

mod background_runtime;
mod cancellation;
mod configuration_runtime;
mod goal_runtime;
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
    AttachmentKind, BackgroundTask, ClientResponse, CollaborationMode, EventStream, PlanDecision,
    PromptInput, Provider, ProviderCapabilities, ProviderConfig, ProviderConfiguration,
    ProviderConfigurationChange, ResumeItem, ResumeTranscript, Session, SessionEnvironment,
    SessionOptions,
};
use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::config::LocalConfig;
use crate::engine::{run_turn, TurnContext};
use crate::exec::{Executor, LocalExecutor};
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
        let mut attachment_kinds = vec![
            AttachmentKind::Text,
            AttachmentKind::Pdf,
            AttachmentKind::Docx,
        ];
        let image_support = self.config.as_ref().is_none_or(|config| {
            crate::config::model_supports_images(&config.model) || config.vision.is_some()
        });
        if image_support {
            attachment_kinds.push(AttachmentKind::Image);
        }
        ProviderCapabilities {
            streaming: true,
            permissions: true,
            fs: true,
            terminal: true,
            load_session: false,
            attachment_kinds,
            modes: vec!["ask".to_string(), "auto".to_string(), "full".to_string()],
            collaboration_modes: vec![CollaborationMode::Default, CollaborationMode::Plan],
        }
    }

    async fn configuration(&self, _session: &SessionId) -> Result<ProviderConfiguration> {
        self.current_configuration().await
    }

    async fn configure(
        &mut self,
        session: &SessionId,
        change: ProviderConfigurationChange,
    ) -> Result<ProviderConfiguration> {
        self.apply_configuration_change(session, change).await
    }

    async fn background_tasks(&self, session: &SessionId) -> Result<Vec<BackgroundTask>> {
        self.list_background_tasks(session).await
    }

    async fn stop_background_task(
        &mut self,
        session: &SessionId,
        task: &str,
    ) -> Result<BackgroundTask> {
        self.stop_background(session, task).await
    }

    async fn clean_background_tasks(&mut self, session: &SessionId) -> Result<Vec<BackgroundTask>> {
        self.clean_background(session).await
    }

    async fn goal_state(&self, session: &SessionId) -> Result<Option<agent_core::GoalState>> {
        self.current_goal(session).await
    }

    async fn resume_goal(&mut self, session: &SessionId) -> Result<agent_core::GoalState> {
        self.resume_session_goal(session).await
    }

    async fn clear_goal(&mut self, session: &SessionId) -> Result<()> {
        self.clear_session_goal(session).await
    }

    async fn add_read_roots(&mut self, session: &SessionId, roots: Vec<String>) -> Result<()> {
        if self.session_id.as_ref() != Some(session) {
            return Err(Error::SessionNotFound(session.to_string()));
        }
        let state = self.session.lock().await;
        if state.active_execution.is_some() {
            return Err(Error::Unsupported(
                "finish the active run before attaching repository context".into(),
            ));
        }
        let plan_mode = state.planning.plan_mode();
        drop(state);
        let roots = roots
            .into_iter()
            .map(|root| {
                let path = std::path::PathBuf::from(&root);
                let canonical = path
                    .canonicalize()
                    .map_err(|error| Error::Other(format!("read-only root {root}: {error}")))?;
                if !canonical.is_dir() {
                    return Err(Error::Other(format!(
                        "read-only root {} is not a directory",
                        canonical.display()
                    )));
                }
                Ok(canonical)
            })
            .collect::<Result<Vec<_>>>()?;
        let current = self.sandbox.as_ref().ok_or(Error::NotConnected)?;
        let mut combined = current.read_roots().to_vec();
        combined.extend(roots);
        self.replace_read_roots(combined, plan_mode)
    }

    async fn remove_read_roots(&mut self, session: &SessionId, roots: Vec<String>) -> Result<()> {
        if self.session_id.as_ref() != Some(session) {
            return Err(Error::SessionNotFound(session.to_string()));
        }
        let state = self.session.lock().await;
        if state.active_execution.is_some() {
            return Err(Error::Unsupported(
                "finish the active run before removing repository context".into(),
            ));
        }
        let plan_mode = state.planning.plan_mode();
        drop(state);
        let remove = roots
            .into_iter()
            .map(|root| {
                std::path::PathBuf::from(&root)
                    .canonicalize()
                    .map_err(|error| Error::Other(format!("read-only root {root}: {error}")))
            })
            .collect::<Result<std::collections::HashSet<_>>>()?;
        let current = self.sandbox.as_ref().ok_or(Error::NotConnected)?;
        let remaining = current
            .read_roots()
            .iter()
            .filter(|root| !remove.contains(*root))
            .cloned()
            .collect();
        self.replace_read_roots(remaining, plan_mode)
    }

    async fn connect(&mut self, config: ProviderConfig) -> Result<()> {
        self.isolation = ProviderIsolation::from_provider_config(&config);
        let local = LocalConfig::from_provider_config(&config);
        let llm = LlmClient::new(&local).map_err(Error::Other)?;
        let memory =
            (local.tools_enabled && local.memories_enabled).then(|| self.memory_config(&local));
        let mut registry = if local.tools_enabled {
            ToolRegistry::new(memory)
        } else {
            ToolRegistry::empty()
        };
        if local.tools_enabled {
            if let Some(api_key) = local.api_key.clone().filter(|_| {
                !local
                    .image_generation_excluded_models
                    .contains(&local.model)
            }) {
                registry.enable_image_generation(crate::tools::image::ImageGenerationConfig {
                    base_url: local.base_url.clone(),
                    api_key,
                });
            }
            if !self.isolation.disposable_writer() {
                if let Some(provider) = self.context_provider.clone() {
                    registry.enable_organization_knowledge(provider);
                }
            }
            if local.browser_enabled {
                let browser = local.browser_binary.clone().ok_or_else(|| {
                    Error::Unsupported("browser is enabled without a host binary policy".into())
                })?;
                registry.enable_browser(browser);
            }
            if local.computer_use_enabled && !self.isolation.disposable_writer() {
                let backend: Arc<dyn computer_use::ComputerBackend> =
                    match local.computer_use_backend {
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
            if !self.isolation.disposable_writer() {
                registry.enable_orchestration(
                    crate::orchestration::OrchestrationToolsConfig::from_local(&local),
                );
            }
            if !self.isolation.disposable_writer() {
                for pack in &self.tool_packs {
                    registry
                        .install_tool_pack(pack.as_ref())
                        .map_err(Error::Other)?;
                }
            }
        }
        self.llm = Some(llm);
        self.registry = Some(Arc::new(registry));
        self.config = Some(local);
        Ok(())
    }

    async fn new_session(&mut self, options: SessionOptions) -> Result<Session> {
        let mut config = self.config()?.clone();
        let scout_full_access = config.scout_cartography.is_some();
        let collaboration_mode = if scout_full_access {
            CollaborationMode::Default
        } else {
            options.collaboration_mode.unwrap_or_default()
        };
        // A Scout-bound provider spans the human-selected organization and
        // workspace. Its Full Access authority is fixed by the product
        // contract and cannot be weakened by stale or modified client state.
        let session_mode = if scout_full_access {
            Some("full".to_string())
        } else {
            options.mode.clone()
        };
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
        let restored_context_revisions = restored_proposed_plan
            .as_ref()
            .map(|plan| plan.context_revisions.clone())
            .unwrap_or_default();

        let cache_session_id = (!config.tools_enabled && config.response_format.is_some())
            .then(|| config.cache_session_id.clone())
            .flatten();
        let id = options.session_id.clone().unwrap_or_else(|| {
            if !config.tools_enabled && config.response_format.is_some() {
                config
                    .cache_session_id
                    .as_deref()
                    .map(SessionId::new)
                    .unwrap_or_else(|| SessionId::new(uuid::Uuid::new_v4().to_string()))
            } else {
                SessionId::new(uuid::Uuid::new_v4().to_string())
            }
        });
        let sandbox_preset = match collaboration_mode {
            CollaborationMode::Plan => exec_sandbox::SandboxPreset::ReadOnly,
            CollaborationMode::Default => {
                exec_sandbox::SandboxPreset::for_session_mode(session_mode.as_deref())
            }
        };

        // Resolve every writable root before selecting the executor so one
        // immutable policy reaches both direct filesystem operations and
        // child-process compilation. Remote sessions run this same provider
        // inside their durable worker, so its executor remains process-local.
        let cwd = options
            .cwd
            .or(config.cwd.clone())
            .ok_or_else(|| Error::Unsupported("local provider requires a project `cwd`".into()))?;

        // Project-scoped config (`.agent/settings.json`) loads before executor
        // construction: `sandbox_write_roots` must reach the session's initial
        // sandbox policy, which cannot exist yet. Read directly rather than
        // through a session executor; disposable-writer isolation never honors it.
        let project = if self.isolation.disposable_writer() {
            crate::project_settings::ProjectSettings::default()
        } else {
            crate::project_settings::load(&LocalExecutor, std::path::Path::new(&cwd)).await
        };
        if !self.isolation.disposable_writer() {
            let (roots, rejected) =
                crate::project_settings::validated_write_roots(&project.sandbox_write_roots);
            for entry in rejected {
                tracing::warn!(entry, "ignoring non-absolute sandbox_write_roots entry");
            }
            config.sandbox_write_roots.extend(roots);
            self.config = Some(config.clone());
        }
        let (sandbox, executor, sandbox_temp): (
            Sandbox,
            Arc<dyn Executor>,
            Option<tempfile::TempDir>,
        ) = {
            let mut sandbox = Sandbox::new(&cwd)
                .map_err(Error::Io)?
                .with_read_roots(config.sandbox_read_roots.clone());
            if !self.isolation.disposable_writer() {
                if crate::workspace::is_quick_chat_workspace(std::path::Path::new(&cwd)) {
                    sandbox = sandbox.with_docs(std::path::PathBuf::from(&cwd));
                } else if let Some(workspace) = crate::workspace::session_workspace(id.as_str()) {
                    if std::fs::create_dir_all(&workspace).is_ok() {
                        sandbox = sandbox.with_docs(workspace);
                    }
                }
            }

            let (executor, sandbox_temp) =
                build_local_executor(&config, &mut sandbox, sandbox_preset)?;
            (sandbox, executor, sandbox_temp)
        };
        self.executor = executor;
        self.sandbox_temp = sandbox_temp;
        self.session_mode = session_mode.clone();
        let sandbox = Arc::new(sandbox);

        // Compatible desktop memories are host-owned startup state, not an
        // agent filesystem action. Import them before rendering the memory
        // prompt so Plan Mode receives the same migrated context as Full
        // Access, while remote workers never read the desktop user's home.
        if config.tools_enabled
            && config.memories_enabled
            && config.compatible_memory_import_enabled
            && !config.remote_worker
            && !self.isolation.disposable_writer()
        {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from);
            let global_dir = config
                .memory_scope
                .as_deref()
                .and_then(crate::memory::global_memory_dir_for_scope);
            let report = crate::external_import::migrate_memories(
                &LocalExecutor,
                sandbox.root(),
                home.as_deref(),
                global_dir.as_deref(),
            )
            .await;
            if report.discovered() > 0 || !report.failures.is_empty() {
                tracing::info!(
                    created = report.created,
                    updated = report.updated,
                    unchanged = report.unchanged,
                    failures = report.failures.len(),
                    "compatible desktop memory migration completed"
                );
            }
            for error in report.failures {
                tracing::warn!(error, "compatible desktop memory migration was incomplete");
            }
        }

        // Configured MCP stdio servers are trusted connector processes. Their
        // individual tool calls remain permission-gated as External, but a
        // network connector itself cannot work inside the offline project
        // sandbox. Keep remote connectors on the remote host; locally, launch
        // the configured server on the host without widening the session's
        // normal file/shell executor.
        if config.tools_enabled && !config.mcp_servers.is_empty() && self.mcp_status.is_empty() {
            let registry = self
                .registry
                .as_mut()
                .and_then(Arc::get_mut)
                .ok_or_else(|| Error::Other("tool registry is already shared".to_string()))?;
            let mcp_executor: Arc<dyn Executor> = Arc::new(LocalExecutor);
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

        // Project-scoped config (`.agent/settings.json`) was loaded before the
        // executor above so its `sandbox_write_roots` could shape sandbox policy.
        let available_tools = self
            .registry
            .as_ref()
            .map(|registry| registry.tool_names())
            .unwrap_or_default();
        let planning_eval_preactivated_tools = config
            .planning_eval_preactivated_tools
            .iter()
            .filter(|name| available_tools.contains(name.as_str()))
            .cloned()
            .collect();
        let skill_project_root = self
            .executor
            .canonicalize(sandbox.root())
            .await
            .unwrap_or_else(|_| sandbox.root().to_path_buf());
        let skill_environment_id = crate::skills::skill_environment_id(&skill_project_root, None);
        let skills = if !config.tools_enabled || self.isolation.disposable_writer() {
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
        let feature_context_registration = if config.tools_enabled
            && !self.isolation.disposable_writer()
        {
            self.context_provider.clone().map(|provider| {
                let scout = config.scout_cartography.as_ref();
                (
                    provider,
                    crate::tools::feature_context::FeatureContextBinding {
                        repository_fingerprint: self.repository_fingerprint.clone(),
                        organization_id: scout.map(|binding| binding.organization_id.to_string()),
                        workspace_id: scout.map(|binding| binding.workspace_id.to_string()),
                    },
                )
            })
        } else {
            None
        };
        let registry = self.next_run_registry_mut()?;
        if config.tools_enabled && skills.enabled().next().is_some() {
            registry.enable_skills(skills.clone());
        } else {
            registry.disable_skills();
        }
        if let Some((provider, binding)) = feature_context_registration {
            registry.enable_feature_context(provider, binding);
        }
        self.skills = skills;
        self.skill_environment_id = Some(skill_environment_id);
        self.skill_disabled_names = project.skills.disabled.clone();

        let commit_attribution = project
            .include_git_instructions()
            .then(|| project.commit_attribution_or(&config.default_commit_attribution));
        let pr_body_attribution = project
            .include_git_instructions()
            .then(|| project.pr_body_attribution_or(&config.default_pr_body_attribution));
        let brokered_research_available = self
            .registry
            .as_ref()
            .is_some_and(|registry| registry.has_brokered_research());
        let mut prompt = config.system_prompt_override.clone().unwrap_or_else(|| {
            system_prompt(
                &sandbox,
                brokered_research_available,
                false,
                commit_attribution,
                pr_body_attribution,
            )
        });
        if let Some(preamble) = crate::hard_constraints::prompt_preamble(&config.hard_constraints) {
            prompt.insert_str(0, &preamble);
        }
        if config.tools_enabled {
            if let Some(docs) = sandbox.docs_root() {
                prompt.push_str(&crate::workspace::prompt_section(docs));
            }
        }
        if let Some(catalog) = crate::skills::render_catalog(&self.skills) {
            prompt.push_str(&catalog);
        }
        if config.tools_enabled && config.memories_enabled {
            prompt.push_str(&self.render_memory_section(&config, &sandbox).await?);
        }
        let mut hydrated_resume = options.resume.clone();
        if let Some(resume) = hydrated_resume.as_mut() {
            crate::attachments::hydrate_resume_attachments(resume).await;
        }
        let resumed_transcript = crate::resume::to_agent_messages(
            hydrated_resume.as_ref(),
            crate::config::model_supports_images(&config.model),
        );
        {
            let mut s = self.session.lock().await;
            s.system_prompt = prompt;
            s.transcript = resumed_transcript;
            s.planning = crate::planning::PlanningState::default();
            s.planning.mode = collaboration_mode;
            s.planning.proposed_plan = restored_proposed_plan;
            s.planning.context_revisions = restored_context_revisions;
            s.deferred_tools = planning_eval_preactivated_tools;
            s.planning_research_autoactivate = config.planning_research_autoactivate;
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
            s.hard_constraints = config.hard_constraints.clone();
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
        self.llm = self
            .llm
            .take()
            .map(|llm| llm.with_session_id(cache_session_id.as_deref().unwrap_or(id.as_str())));
        self.session_id = Some(id.clone());
        let sandbox = self.sandbox.as_ref().expect("sandbox was just installed");
        let checkout_root = sandbox.root().to_string_lossy().into_owned();
        let docs_root = sandbox
            .docs_root()
            .map(|root| root.to_string_lossy().into_owned());
        let mut workspace_roots = vec![checkout_root.clone()];
        if let Some(docs_root) = docs_root.as_ref().filter(|root| *root != &checkout_root) {
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
                remote: config.remote_worker,
            }),
        })
    }

    async fn load_session(&mut self, _id: SessionId) -> Result<Session> {
        Err(Error::Unsupported(
            "local provider does not support resuming sessions".into(),
        ))
    }

    async fn session_transcript(&self, session: &SessionId) -> Result<ResumeTranscript> {
        if self.session_id.as_ref() != Some(session) {
            return Err(Error::SessionNotFound(session.to_string()));
        }
        let state = self.session.lock().await;
        let mut transcript = crate::resume::from_agent_messages(&state.transcript);
        if let Some(plan) = state.planning.proposed_plan.as_ref() {
            transcript
                .items
                .push(ResumeItem::ProposedPlan { plan: plan.clone() });
        }
        if let Some(goal) = state.goal.as_ref() {
            transcript.items.push(ResumeItem::Goal {
                goal: goal.state(None),
            });
        }
        Ok(transcript)
    }

    async fn validate_prompt(&self, _session: &SessionId, input: &PromptInput) -> Result<()> {
        let supported = self.capabilities().attachment_kinds;
        if let Some(attachment) = input
            .attachments
            .iter()
            .find(|attachment| !supported.contains(&attachment.kind()))
        {
            return Err(Error::Unsupported(format!(
                "{} attachment `{}` is not supported by this model configuration",
                attachment.content_type, attachment.filename
            )));
        }
        if self.manual_compacting.load(Ordering::Acquire) {
            return Err(Error::Unsupported(
                "wait for context compaction to finish before sending a message".into(),
            ));
        }
        self.sandbox.as_ref().ok_or(Error::NotConnected)?;
        self.config()?;
        self.llm.as_ref().ok_or(Error::NotConnected)?;
        self.registry.as_ref().ok_or(Error::NotConnected)?;

        let user_request = prompt_parts(input).user_request;
        if let Some(objective) = goal_command_objective(&user_request) {
            let session = self.session.lock().await;
            crate::tools::goal::validate_goal_command(&session, &objective)
                .map_err(Error::Other)?;
        }
        Ok(())
    }

    async fn prompt(&mut self, session: &SessionId, input: PromptInput) -> Result<EventStream> {
        self.validate_prompt(session, &input).await?;
        // The accepted turn supersedes any previous turn still parked on a
        // permission answer — its armed request can never be resolved by this
        // new run and would otherwise poison the session (see
        // `supersede_parked_runs`).
        self.supersede_parked_runs().await;
        let sandbox = self.sandbox.clone().ok_or(Error::NotConnected)?;
        let config = self.config()?.clone();
        if config.tools_enabled && !self.isolation.disposable_writer() {
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
            // A just-finished run may still be dropping its event sink and
            // tool adapters after RunFinished reaches the UI. Fork the next
            // run's registry snapshot instead of rejecting an accepted user
            // turn merely because the prior immutable snapshot is still held.
            let registry = self.next_run_registry_mut()?;
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
        {
            let available_tools = registry.tool_names();
            let mut session = self.session.lock().await;
            if session.planning.plan_mode() && session.planning_research_autoactivate {
                session
                    .deferred_tools
                    .extend(crate::planning::available_source_tools(&available_tools));
            }
        }
        let session_id = self.session_id.clone().ok_or(Error::NotConnected)?;
        // Fresh cancellation scope for this run — created early so it can also
        // gate the attachment pre-processing below (vision call / doc parsing).
        let cancel = CancellationToken::new();
        self.cancel = cancel.clone();

        let parts = prompt_parts(&input);
        let knowledge_query = prompt_text(&input);
        let user_request = parts.user_request;
        let sandbox = explicit_task_scope(&sandbox, &user_request)
            .and_then(|scope| (*sandbox).clone().with_task_scope(&scope).ok())
            .map(Arc::new)
            .unwrap_or(sandbox);
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
        let scout_turn =
            crate::skills::invokes_skill(&run_skills, &input.blocks, &user_request, "scout:scout");
        if scout_turn {
            // An explicit Scout invocation is already a capability selection.
            // Pre-activate its typed dependencies so the model does not have
            // to rediscover them through `tool_search` before every bounded
            // evidence action. This is especially important for remote
            // sessions, where an unnecessary discovery turn can outlive the
            // interactive run window.
            let mut session = self.session.lock().await;
            session.deferred_tools.extend(
                crate::scout_policy::EVIDENCE_TOOLS
                    .iter()
                    .map(|name| (*name).to_string()),
            );
        }
        let security_deep_turn = crate::skills::invokes_skill(
            &run_skills,
            &input.blocks,
            &user_request,
            "security:security-deep",
        );
        let security_turn = security_deep_turn
            || ["security:security-scan", "security:security-diff"]
                .into_iter()
                .any(|skill| {
                    crate::skills::invokes_skill(&run_skills, &input.blocks, &user_request, skill)
                });
        // The conversation picker is not an entitlement source. Scout and
        // Security pin their own execution routes below; Clark Code's gateway then
        // admits those routes against the API key's current personal/workspace
        // coverage. Rejecting here merely because the base conversation uses
        // the included lane blocks paid workspace members before the
        // authoritative billing boundary can run.
        let model_override = if scout_turn {
            config.skill_model_overrides.get("scout")
        } else if security_turn {
            config.skill_model_overrides.get("security")
        } else {
            None
        }
        .map(|policy| crate::tools::TurnModelOverride {
            model: policy.model.clone(),
            reasoning_effort: policy.reasoning_effort.clone(),
        });
        let effective_model = model_override
            .as_ref()
            .map(|policy| policy.model.clone())
            .unwrap_or_else(|| config.model.clone());
        let llm = match model_override.as_ref() {
            Some(policy) => llm
                .with_model(&policy.model)
                .with_reasoning_effort(policy.reasoning_effort.as_deref()),
            None => llm,
        };
        if security_turn {
            let mut session = self.session.lock().await;
            session
                .deferred_tools
                .insert("security_scan_contract".into());
            session.deferred_tools.insert("security_poc_execute".into());
            if security_deep_turn {
                session.deferred_tools.insert("delegate_read_only".into());
                session.deferred_tools.insert("resolve_delegation".into());
            }
        }
        let goal_command = goal_command_objective(&user_request);
        if let Some(objective) = goal_command.as_ref() {
            let mut session = self.session.lock().await;
            crate::tools::goal::apply_goal_command(&mut session, objective)
                .map_err(Error::Other)?;
        } else if explicitly_requests_goal_lifecycle(&user_request) {
            let mut session = self.session.lock().await;
            session.deferred_tools.extend(
                ["create_goal", "update_goal", "get_goal"]
                    .into_iter()
                    .map(str::to_string),
            );
        }
        let native_image_support = crate::config::model_supports_images(&effective_model);
        let mut context_sections = Vec::new();
        if !self.isolation.disposable_writer() {
            context_sections.push(crate::orchestration::turn_policy_section().to_string());
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
        context_sections.push(environment_context(&sandbox, config.remote_worker));
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
            let provider = self.context_provider.as_ref()?;
            let fingerprint = self.repository_fingerprint.as_deref()?;
            let context = provider
                .repository_context(fingerprint, &knowledge_query)
                .await
                .ok()?;
            crate::platform::repository_context_section(&context)
        };
        let approved_feature_pin = {
            let session = self.session.lock().await;
            session
                .planning
                .proposed_plan
                .as_ref()
                .filter(|plan| plan.status == agent_core::domain::ProposedPlanStatus::Approved)
                .and_then(|plan| {
                    plan.context_revisions
                        .iter()
                        .find(|revision| revision.context_kind == "enterprise_feature_context")
                        .cloned()
                })
        };
        let feature_context = async {
            let provider = self.context_provider.as_ref()?;
            let scout_binding = config.scout_cartography.as_ref();
            if !config.project_knowledge_enabled
                && self.repository_fingerprint.is_none()
                && scout_binding.is_none()
            {
                return None;
            }
            let pinned_revision = approved_feature_pin.as_ref().map(|revision| {
                crate::platform::FeatureContextRevision {
                    effective_at_ms: revision.effective_at_ms,
                    known_at_ms: revision.known_at_ms,
                    selector_sha256: revision.selector_sha256.clone(),
                }
            });
            let request = crate::platform::FeatureContextRequest {
                action: crate::platform::FeatureContextQueryKind::Task,
                query: approved_feature_pin
                    .as_ref()
                    .map(|revision| revision.query.clone())
                    .unwrap_or_else(|| knowledge_query.clone()),
                repository_fingerprint: self.repository_fingerprint.clone(),
                organization_id: approved_feature_pin
                    .as_ref()
                    .and_then(|revision| revision.organization_id.clone())
                    .or_else(|| scout_binding.map(|binding| binding.organization_id.to_string())),
                workspace_id: approved_feature_pin
                    .as_ref()
                    .and_then(|revision| revision.workspace_id.clone())
                    .or_else(|| scout_binding.map(|binding| binding.workspace_id.to_string())),
                object_ids: Vec::new(),
                target_object_ids: Vec::new(),
                changed_since_ms: None,
                max_depth: 2,
                pinned_revision,
                max_objects: 96,
            };
            provider.feature_context(&request).await.ok()
        };

        // The tree may be shared with other agents, so git state is re-taken
        // per turn (a session-start snapshot would go stale) and lands in the
        // turn message, keeping the cached system-prompt prefix stable.
        let git_snapshot =
            crate::repository::working_tree_snapshot(self.executor.as_ref(), sandbox.root());

        // Attachment extraction/vision, repository recall, enterprise feature
        // context, and the git snapshot are independent read-only preflight
        // work. Overlap them so
        // first-token latency is bounded by the slowest branch instead of
        // adding the durations together.
        let (attachment_context, repository_context, feature_context, git_snapshot) = tokio::join!(
            attachment_context,
            repository_context,
            feature_context,
            git_snapshot
        );
        if let Some(section) = repository_context {
            context_sections.push(section);
        }
        if let Some(response) = feature_context {
            let revisions = response
                .packets
                .iter()
                .map(|packet| agent_core::domain::PlanContextRevision {
                    context_kind: "enterprise_feature_context".into(),
                    organization_id: Some(packet.organization_id.clone()),
                    workspace_id: Some(packet.workspace_id.clone()),
                    query: packet.query.clone(),
                    effective_at_ms: packet.revision.effective_at_ms,
                    known_at_ms: packet.revision.known_at_ms,
                    selector_sha256: packet.revision.selector_sha256.clone(),
                })
                .collect();
            self.session
                .lock()
                .await
                .planning
                .set_context_revisions(revisions);
            if let Some(section) = crate::platform::feature_context_section(&response) {
                context_sections.push(section);
            }
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
            } else if config.plan_execution_reminders {
                let reminder = crate::planning::execution_continuation_note(
                    s.planning.proposed_plan.as_ref(),
                    s.planning.execution_checklist.as_ref(),
                );
                (None, reminder)
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

        let run = self.next_run_id();
        self.run_cancellations.register(&run, cancel.clone());
        let (tx, rx) = async_channel::unbounded::<AgentEvent>();

        // Post-turn durable-fact extraction (structural memory proactivity):
        // only when memories are on, and always off the turn's latency path.
        // Extraction quality may use a host-pinned model instead of inheriting
        // the active conversation model.
        let memory_extraction = (!scout_turn && config.memories_enabled).then(|| {
            let extraction_llm = config
                .memory_extraction_model
                .as_deref()
                .map_or_else(|| llm.clone(), |model| llm.clone().with_model(model));
            crate::memory_extraction::ExtractionCtx {
                llm: extraction_llm,
                executor: self.executor.clone(),
                project_root: sandbox.root().to_path_buf(),
                global_dir: config
                    .memory_scope
                    .as_deref()
                    .and_then(crate::memory::global_memory_dir_for_scope),
            }
        });
        let turn_system_prompt = if scout_turn {
            let session = self.session.lock().await;
            Some(
                crate::provider::configuration_runtime::without_memory_section(
                    &session.system_prompt,
                ),
            )
        } else {
            None
        };

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
                model_override,
            },
            session: self.session.clone(),
            control: self.control.clone(),
            session_id,
            compaction: config.compaction,
            plan_execution_reminders: config.plan_execution_reminders,
            hidden_plan_protocol: config.hidden_plan_protocol,
            scout_turn,
            turn_system_prompt,
            model: effective_model,
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
            run_cancellations: self.run_cancellations.clone(),
            tool_image_policy: crate::agent_adapter::ToolImagePolicy {
                native_image_support,
                vision: config.vision.clone(),
            },
            runtime_plugin_packs: self.runtime_plugin_packs.clone(),
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

        let run = self.next_run_id();
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
            return Err(Error::RunNotActive(run.clone()));
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
                        session.planning.approve_execution();
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
                        let mut sandbox = sandbox.as_ref().clone();
                        let (executor, sandbox_temp) = build_local_executor(
                            config,
                            &mut sandbox,
                            exec_sandbox::SandboxPreset::WorkspaceWrite,
                        )?;
                        self.executor = executor;
                        self.sandbox_temp = sandbox_temp;
                        self.sandbox = Some(Arc::new(sandbox));
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
        let mode = if self
            .config
            .as_ref()
            .is_some_and(|config| config.scout_cartography.is_some())
        {
            "full".to_string()
        } else {
            mode
        };
        let plan_mode = self.session.lock().await.planning.plan_mode();
        if !plan_mode {
            if let (Some(config), Some(sandbox)) = (self.config.as_ref(), self.sandbox.as_ref()) {
                let preset = exec_sandbox::SandboxPreset::for_session_mode(Some(&mode));
                let mut sandbox = sandbox.as_ref().clone();
                let (executor, sandbox_temp) = build_local_executor(config, &mut sandbox, preset)?;
                self.executor = executor;
                self.sandbox_temp = sandbox_temp;
                self.sandbox = Some(Arc::new(sandbox));
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
        let mode = if self
            .config
            .as_ref()
            .is_some_and(|config| config.scout_cartography.is_some())
        {
            CollaborationMode::Default
        } else {
            mode
        };
        if let (Some(config), Some(sandbox)) = (self.config.as_ref(), self.sandbox.as_ref()) {
            let preset = match mode {
                CollaborationMode::Plan => exec_sandbox::SandboxPreset::ReadOnly,
                CollaborationMode::Default => {
                    exec_sandbox::SandboxPreset::for_session_mode(self.session_mode.as_deref())
                }
            };
            let mut sandbox = sandbox.as_ref().clone();
            let (executor, sandbox_temp) = build_local_executor(config, &mut sandbox, preset)?;
            self.executor = executor;
            self.sandbox_temp = sandbox_temp;
            self.sandbox = Some(Arc::new(sandbox));
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

    fn start_side_question(
        &self,
        _session: &SessionId,
        question: &str,
    ) -> agent_core::SideQuestionFuture {
        self.side_question_future(question)
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
