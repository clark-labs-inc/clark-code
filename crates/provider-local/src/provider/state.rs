//! Construction and local state access for the provider.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64};

pub struct LocalAgentProvider {
    pub(super) isolation: ProviderIsolation,
    pub(super) config: Option<LocalConfig>,
    pub(super) llm: Option<LlmClient>,
    pub(super) registry: Option<Arc<ToolRegistry>>,
    pub(super) sandbox: Option<Arc<Sandbox>>,
    pub(super) session_id: Option<SessionId>,
    pub(super) session: Arc<Mutex<SessionState>>,
    pub(super) control: Arc<Mutex<RunControl>>,
    /// Session-scoped read tracker (read-before-edit/write invariant).
    pub(super) reads: Arc<std::sync::Mutex<ReadTracker>>,
    /// Session-scoped `bash(run_in_background: true)` task registry.
    pub(super) background: Arc<crate::background::BackgroundTasks>,
    /// Cancellation token for prompt setup and the newest run. Exact run
    /// cancellation uses `run_cancellations` below.
    pub(super) cancel: CancellationToken,
    pub(super) run_cancellations: RunCancellationRegistry,
    /// Manual compaction is a standalone, non-steerable run. This latch closes
    /// the gap before its RunStarted event reaches the frontend and prevents a
    /// normal prompt from racing the history replacement.
    pub(super) manual_compacting: Arc<AtomicBool>,
    /// Where this provider process performs tool I/O. A remote session runs the
    /// whole provider inside its worker, so this remains process-local.
    pub(super) executor: Arc<dyn crate::exec::Executor>,
    pub(super) run_counter: AtomicU64,
    /// Per-provider-instance namespace. Resuming or editing a conversation
    /// constructs a new provider, so the sequence alone is not a durable run
    /// identity across the merged history prefix.
    pub(super) run_namespace: String,
    /// Last MCP connection result, surfaced to the settings UI.
    pub(super) mcp_status: Vec<crate::mcp::McpStatus>,
    /// Stable identity for the active project, when private project knowledge
    /// is enabled and the selected root is a Git repository.
    pub(super) repository_fingerprint: Option<String>,
    /// Session catalog for progressive skill disclosure and explicit `$skill`
    /// injection. Rebuilt for each project root in `new_session`.
    pub(super) skills: Arc<crate::skills::SkillCatalog>,
    /// Process-wide catalog authority shared with the native composer API.
    pub(super) skill_catalogs: Arc<crate::skills::SkillCatalogService>,
    pub(super) skill_environment_id: Option<String>,
    pub(super) skill_disabled_names: Vec<String>,
    /// Named approval preset selected for this session. This is stored
    /// separately from collaboration mode so Plan can temporarily enforce
    /// read-only execution, then restore the selected sandbox when it exits.
    pub(super) session_mode: Option<String>,
    /// Owns the narrow temporary write root exposed to sandboxed children.
    pub(super) sandbox_temp: Option<tempfile::TempDir>,
    /// Product-owned capabilities installed after the neutral core registry.
    pub(super) tool_packs: Vec<Arc<dyn crate::tools::ToolPack>>,
    /// Product-owned mailbox/event hooks attached only to root agent runs.
    pub(super) runtime_plugin_packs: Vec<Arc<dyn crate::runtime_plugins::RuntimePluginPack>>,
    /// Optional product transport for personal, repository, and organization
    /// context. The neutral provider never owns cloud routes or credentials.
    pub(super) context_provider: Option<Arc<dyn crate::platform::PlatformContextProvider>>,
}

impl LocalAgentProvider {
    pub fn new() -> Self {
        Self {
            isolation: ProviderIsolation::default(),
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
            run_cancellations: RunCancellationRegistry::default(),
            manual_compacting: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            executor: Arc::new(crate::exec::LocalExecutor),
            run_counter: AtomicU64::new(0),
            run_namespace: uuid::Uuid::new_v4().simple().to_string(),
            mcp_status: Vec::new(),
            repository_fingerprint: None,
            skills: Arc::new(crate::skills::SkillCatalog::default()),
            skill_catalogs: Arc::new(crate::skills::SkillCatalogService::new()),
            skill_environment_id: None,
            skill_disabled_names: Vec::new(),
            session_mode: None,
            sandbox_temp: None,
            tool_packs: Vec::new(),
            runtime_plugin_packs: Vec::new(),
            context_provider: None,
        }
    }

    /// MCP connection statuses from the last `connect`, for the settings UI.
    pub fn mcp_status(&self) -> &[crate::mcp::McpStatus] {
        &self.mcp_status
    }

    /// Names of every tool currently registered for this session. Used by tests
    /// to assert which capabilities (e.g. the Scout toolchain) are exposed for a
    /// given target without paying for a model call.
    pub fn tool_names(&self) -> Vec<String> {
        self.registry
            .as_ref()
            .map(|registry| registry.tool_names().into_iter().collect())
            .unwrap_or_default()
    }

    /// The executor this provider process uses for session tool I/O.
    pub fn session_executor(&self) -> Arc<dyn crate::exec::Executor> {
        self.executor.clone()
    }

    /// Look up a registered tool by name. Combined with [`Self::tool_ctx`], this
    /// lets tests drive a tool directly through a live session
    /// without paying for a model call.
    pub fn tool(&self, name: &str) -> Option<Arc<dyn crate::tools::ToolExecutor>> {
        self.registry.as_ref().and_then(|r| r.get(name))
    }

    /// A `ToolCtx` bound to this session's sandbox, executor, read-tracker, and
    /// session state — the same wiring a real tool call gets. `None` until a
    /// session has been started (`new_session`).
    pub fn tool_ctx(&self) -> Option<crate::tools::ToolCtx> {
        let sandbox = self.sandbox.clone()?;
        Some(crate::tools::ToolCtx {
            sandbox,
            executor: self.executor.clone(),
            reads: self.reads.clone(),
            cancel: CancellationToken::new(),
            background: self.background.clone(),
            session: self.session.clone(),
            progress: None,
            agent_progress: None,
            call_progress: None,
            model_override: None,
        })
    }

    pub fn with_skill_catalog_service(
        mut self,
        service: Arc<crate::skills::SkillCatalogService>,
    ) -> Self {
        self.skill_catalogs = service;
        self
    }

    pub fn with_tool_pack(mut self, pack: Arc<dyn crate::tools::ToolPack>) -> Self {
        self.tool_packs.push(pack);
        self
    }

    pub fn with_runtime_plugin_pack(
        mut self,
        pack: Arc<dyn crate::runtime_plugins::RuntimePluginPack>,
    ) -> Self {
        self.runtime_plugin_packs.push(pack);
        self
    }

    pub fn with_context_provider(
        mut self,
        provider: Arc<dyn crate::platform::PlatformContextProvider>,
    ) -> Self {
        self.context_provider = Some(provider);
        self
    }

    /// Mutate the registry used by future runs without requiring every owner
    /// of the previous run's immutable registry snapshot to have dropped.
    pub(super) fn next_run_registry_mut(&mut self) -> Result<&mut ToolRegistry> {
        self.registry
            .as_mut()
            .map(Arc::make_mut)
            .ok_or(Error::NotConnected)
    }

    pub(super) fn next_run_id(&self) -> RunId {
        let sequence = self.run_counter.fetch_add(1, Ordering::SeqCst) + 1;
        RunId::new(format!("run-{}-{sequence}", self.run_namespace))
    }

    /// A new user turn supersedes any still-parked previous turn.
    ///
    /// A run parked on a permission answer owns the session's single armed
    /// permission request. If its host abandoned it (remote stream timeout,
    /// restart, retry), that request must not poison every later turn: the
    /// next `arm()` would refuse and the new run would die as `tool_fatal`.
    /// Cancel the leftover run task(s) and drop the armed request; the parked
    /// waiter observes cancellation (or its closed response channel) and ends
    /// as `Cancelled` instead of leaking.
    pub(super) async fn supersede_parked_runs(&mut self) -> bool {
        let superseded = self.run_cancellations.has_active();
        self.run_cancellations.cancel_all();
        self.control.lock().await.clear();
        superseded
    }

    pub(super) fn config(&self) -> Result<&LocalConfig> {
        self.config.as_ref().ok_or(Error::NotConnected)
    }

    pub(super) fn replace_read_roots(
        &mut self,
        roots: Vec<std::path::PathBuf>,
        plan_mode: bool,
    ) -> Result<()> {
        let current = self.sandbox.as_ref().ok_or(Error::NotConnected)?;
        let mut sandbox = current.as_ref().clone();
        sandbox = sandbox.replacing_read_roots(roots);
        let mut config = self.config()?.clone();
        config.sandbox_read_roots = sandbox.read_roots().to_vec();
        let preset = if plan_mode {
            exec_sandbox::SandboxPreset::ReadOnly
        } else {
            exec_sandbox::SandboxPreset::for_session_mode(self.session_mode.as_deref())
        };
        let (executor, sandbox_temp) = build_local_executor(&config, &mut sandbox, preset)?;
        self.executor = executor;
        self.sandbox_temp = sandbox_temp;
        self.sandbox = Some(Arc::new(sandbox));
        self.config = Some(config);
        Ok(())
    }
}

impl Default for LocalAgentProvider {
    fn default() -> Self {
        Self::new()
    }
}
