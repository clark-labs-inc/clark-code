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
    /// Where this session's tool I/O runs — local today, remote (over the
    /// exec-server) once a remote project is selected. Chosen in `new_session`.
    pub(super) executor: Arc<dyn crate::exec::Executor>,
    pub(super) run_counter: AtomicU64,
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
            mcp_status: Vec::new(),
            repository_fingerprint: None,
            skills: Arc::new(crate::skills::SkillCatalog::default()),
            skill_catalogs: Arc::new(crate::skills::SkillCatalogService::new()),
            skill_environment_id: None,
            skill_disabled_names: Vec::new(),
            session_mode: None,
            sandbox_temp: None,
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

    /// The executor this session's tool I/O runs through — local, or a remote
    /// host (over the exec-server) for a remote project. Tests use it to prove a
    /// remote session's reads resolve to the remote host, not the local disk.
    pub fn session_executor(&self) -> Arc<dyn crate::exec::Executor> {
        self.executor.clone()
    }

    /// Look up a registered tool by name. Combined with [`Self::tool_ctx`], this
    /// lets tests drive a tool directly through a live session (local or remote)
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

    pub(super) fn config(&self) -> Result<&LocalConfig> {
        self.config.as_ref().ok_or(Error::NotConnected)
    }
}

impl Default for LocalAgentProvider {
    fn default() -> Self {
        Self::new()
    }
}
