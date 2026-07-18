//! Construction and local state access for the provider.

use super::*;

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
            executor: Arc::new(crate::exec::LocalExecutor),
            run_counter: AtomicU64::new(0),
            mcp_status: Vec::new(),
            repository_fingerprint: None,
            instruction_snapshot: None,
        }
    }

    /// MCP connection statuses from the last `connect`, for the settings UI.
    pub fn mcp_status(&self) -> &[crate::mcp::McpStatus] {
        &self.mcp_status
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
