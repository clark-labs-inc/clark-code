use super::*;

/// Whether a product-supplied tool is visible in the initial model schema or
/// discovered later through `tool_search`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolExposure {
    Eager,
    Deferred,
}

/// Compile-time extension point for product-owned tool bundles. The open local
/// provider owns execution and safety; a branded product can add brokered
/// capabilities without teaching the provider their names or policies.
pub trait ToolPack: Send + Sync {
    fn id(&self) -> &str;
    fn install(&self, registry: &mut ToolRegistry) -> Result<(), String>;
}

/// The ordered executor registry plus its model-visible exposure catalog.
pub struct ToolRegistry {
    tools: Vec<Arc<dyn ToolExecutor>>,
    deferred_catalog: deferred::DeferredToolCatalog,
    /// Live MCP server connections, kept alive for the registry's lifetime.
    _mcp_clients: Vec<Arc<crate::mcp::McpClient>>,
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        let deferred_catalog = self.deferred_catalog.snapshot();
        let tools = self
            .tools
            .iter()
            .map(|tool| {
                if tool.name() == "tool_search" {
                    Arc::new(deferred::ToolSearch::new(deferred_catalog.clone()))
                        as Arc<dyn ToolExecutor>
                } else {
                    tool.clone()
                }
            })
            .collect();
        Self {
            tools,
            deferred_catalog,
            _mcp_clients: self._mcp_clients.clone(),
        }
    }
}

impl ToolRegistry {
    /// A deliberately empty registry for host-owned structured model turns.
    /// This is stronger than denying permissions: no tool schema or executor
    /// exists, so the model cannot request an effect at all.
    pub(crate) fn empty() -> Self {
        Self {
            tools: Vec::new(),
            deferred_catalog: deferred::DeferredToolCatalog::default(),
            _mcp_clients: Vec::new(),
        }
    }

    /// The standard local coding tools, plus the `memory` tool when memories
    /// are enabled. Product capabilities are installed only through
    /// [`ToolPack`].
    pub fn new(memory: Option<memory::MemoryConfig>) -> Self {
        let deferred_catalog = deferred::DeferredToolCatalog::default();
        let mut registry = Self {
            tools: Vec::new(),
            deferred_catalog: deferred_catalog.clone(),
            _mcp_clients: Vec::new(),
        };
        for tool in [
            Arc::new(fs::ReadFile) as Arc<dyn ToolExecutor>,
            Arc::new(fs::ListDir),
            Arc::new(fs::Glob),
            Arc::new(grep::Grep),
            Arc::new(image::ViewImage),
            Arc::new(fs::WriteFile),
            Arc::new(fs::EditFile),
            Arc::new(apply_patch::ApplyPatch),
            Arc::new(shell::Bash),
            Arc::new(shell::BashOutput),
            Arc::new(shell::BashWait),
            Arc::new(shell::BashInput),
            Arc::new(shell::BashKill),
            Arc::new(plan::ProposePlan),
            Arc::new(plan::EnterPlanMode),
            Arc::new(plan::UpdatePlan),
            Arc::new(diagnostics::CheckDiagnostics),
            Arc::new(final_answer::FinalAnswer),
            Arc::new(deferred::ToolSearch::new(deferred_catalog)),
        ] {
            registry.register_eager(tool);
        }
        for tool in [
            Arc::new(goal::CreateGoal) as Arc<dyn ToolExecutor>,
            Arc::new(goal::UpdateGoal),
            Arc::new(goal::GetGoal),
            Arc::new(effect::VerifyEffect),
            Arc::new(research::ResearchRank),
            Arc::new(research_tree::ResearchTreeTool),
            Arc::new(research_tree::ResearchTreeStatus),
            Arc::new(document::DocumentConvert),
            Arc::new(security_poc_execute::SecurityPocExecute),
            Arc::new(security_scan_contract::SecurityScanContract),
            Arc::new(web_fetch::WebFetchTool::new()),
            Arc::new(android_emulator::ListDevices),
            Arc::new(android_emulator::BootEmulator),
            Arc::new(android_emulator::ShutdownEmulator),
            Arc::new(android_emulator::InstallApp),
            Arc::new(android_emulator::UninstallApp),
            Arc::new(android_emulator::LaunchApp),
            Arc::new(android_emulator::Screenshot),
            Arc::new(android_emulator::Tap),
            Arc::new(android_emulator::Swipe),
            Arc::new(android_emulator::TypeText),
            Arc::new(android_emulator::PressButton),
        ] {
            registry.register_deferred(tool);
        }
        #[cfg(target_os = "macos")]
        {
            for tool in [
                Arc::new(ios_simulator::ListSimulators) as Arc<dyn ToolExecutor>,
                Arc::new(ios_simulator::BootSimulator),
                Arc::new(ios_simulator::ShutdownSimulator),
                Arc::new(ios_simulator::InstallApp),
                Arc::new(ios_simulator::UninstallApp),
                Arc::new(ios_simulator::LaunchApp),
                Arc::new(ios_simulator::Screenshot),
                Arc::new(ios_simulator::Tap),
                Arc::new(ios_simulator::Swipe),
                Arc::new(ios_simulator::TypeText),
                Arc::new(ios_simulator::PressButton),
            ] {
                registry.register_deferred(tool);
            }
        }
        if let Some(cfg) = memory {
            registry.register_deferred(Arc::new(memory::MemoryRecallTool::new(
                cfg.global_dir.clone(),
                cfg.personal.clone(),
            )));
            registry.register_deferred(Arc::new(memory::MemoryTool::new(
                cfg.global_dir,
                cfg.personal,
            )));
        }
        registry
    }

    fn register_eager(&mut self, tool: Arc<dyn ToolExecutor>) {
        self.deferred_catalog.register(
            tool.name(),
            tool.description(),
            deferred::ToolExposure::Eager,
        );
        self.tools.push(tool);
    }

    fn register_deferred(&mut self, tool: Arc<dyn ToolExecutor>) {
        self.deferred_catalog.register(
            tool.name(),
            tool.description(),
            deferred::ToolExposure::Deferred,
        );
        self.tools.push(tool);
    }

    /// Add one product-owned tool without allowing it to shadow a built-in or
    /// another extension. Registration order remains model-visible order.
    pub fn register_extension_tool(
        &mut self,
        exposure: ToolExposure,
        tool: Arc<dyn ToolExecutor>,
    ) -> Result<(), String> {
        let name = tool.name();
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("extension tool name is invalid".to_string());
        }
        if self
            .tools
            .iter()
            .any(|registered| registered.name() == name)
        {
            return Err(format!("tool `{name}` is already registered"));
        }
        match exposure {
            ToolExposure::Eager => self.register_eager(tool),
            ToolExposure::Deferred => self.register_deferred(tool),
        }
        Ok(())
    }

    pub fn install_tool_pack(&mut self, pack: &dyn ToolPack) -> Result<(), String> {
        let id = pack.id();
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            return Err("tool pack id is invalid".to_string());
        }
        pack.install(self)
            .map_err(|error| format!("tool pack `{id}`: {error}"))
    }

    /// Install the session's progressive-disclosure skill reader. Replacing a
    /// prior reader keeps repeated `new_session` calls bound to the new root.
    pub(crate) fn enable_skills(&mut self, catalog: Arc<crate::skills::SkillCatalog>) {
        self.disable_skills();
        self.register_eager(Arc::new(skill::ReadSkill::new(catalog)));
    }

    pub(crate) fn disable_skills(&mut self) {
        self.tools.retain(|tool| tool.name() != "read_skill");
        self.deferred_catalog.remove_name("read_skill");
    }

    pub(crate) fn tool_names(&self) -> std::collections::HashSet<String> {
        self.tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect()
    }

    /// Whether the installed tool set contains product-brokered external
    /// research. Prompt capability claims must come from this executable
    /// boundary, not from provider metadata.
    pub(crate) fn has_brokered_research(&self) -> bool {
        self.tools.iter().any(|tool| {
            tool.kind() == ToolKind::Research
                && tool.permission_class() == ToolPermissionClass::BrokeredProduct
        })
    }

    /// Register the opt-in, host-configured browser tool, downloaded on first
    /// use. Called separately from `new()`, gated by the
    /// user's Settings toggle (off by default) — the tool isn't even
    /// advertised to the model unless enabled.
    pub fn enable_browser(&mut self, config: crate::browser_binary::BrowserBinaryConfig) {
        self.disable_browser();
        self.register_deferred(Arc::new(browser::BrowserTool::new(config)));
    }

    pub(crate) fn disable_browser(&mut self) {
        self.tools.retain(|tool| tool.name() != "browser");
        self.deferred_catalog.remove_name("browser");
    }

    pub(crate) fn enable_memory(&mut self, config: memory::MemoryConfig) {
        self.disable_memory();
        self.register_deferred(Arc::new(memory::MemoryRecallTool::new(
            config.global_dir.clone(),
            config.personal.clone(),
        )));
        self.register_deferred(Arc::new(memory::MemoryTool::new(
            config.global_dir,
            config.personal,
        )));
    }

    pub(crate) fn disable_memory(&mut self) {
        self.tools
            .retain(|tool| !matches!(tool.name(), "memory" | "memory_recall"));
        self.deferred_catalog.remove_name("memory");
        self.deferred_catalog.remove_name("memory_recall");
    }

    /// Register the opt-in desktop observation and input tools. All executors
    /// share one backend so observation freshness, rate limits, and window
    /// identity are enforced across calls.
    pub fn enable_computer_use(&mut self, backend: Arc<dyn computer_use::ComputerBackend>) {
        for tool in computer_use::executors(backend) {
            // Perception is the entry point and recovery path for the whole
            // observe-before-act state machine. Keep it visible whenever the
            // user has enabled computer use so a resumed session cannot retain
            // action names from its transcript while losing the only safe way
            // to mint fresh observation capabilities. Mutating input tools
            // remain deferred until the model searches for the needed action.
            match tool.name() {
                "computer_permissions"
                | "computer_request_permissions"
                | "computer_list_windows"
                | "computer_open_app"
                | "computer_get_state"
                | "computer_commit_action" => self.register_eager(tool),
                _ => self.register_deferred(tool),
            }
        }
    }

    /// Register Clark Code-platform-backed image generation/editing when a signed-in
    /// session has a platform key. The key stays between Desktop and Clark Code;
    /// the relay owns provider credentials and billing.
    pub fn enable_image_generation(&mut self, config: image::ImageGenerationConfig) {
        self.disable_image_generation();
        self.register_deferred(Arc::new(image::GenerateImage::new(config)));
    }

    pub(crate) fn disable_image_generation(&mut self) {
        self.tools.retain(|tool| tool.name() != "generate_image");
        self.deferred_catalog.remove_name("generate_image");
    }

    /// Register orchestration tools for the root agent. Isolated child writers
    /// never advertise them, preventing recursive delegation.
    pub(crate) fn enable_orchestration(
        &mut self,
        config: crate::orchestration::OrchestrationToolsConfig,
    ) {
        for tool in crate::orchestration::orchestration_tools(config) {
            self.register_deferred(tool);
        }
    }

    /// Connect the configured MCP servers and register their tools. A server
    /// that fails to start is skipped (not fatal); the returned statuses let the
    /// UI show what connected. Tool-name collisions are dropped (first wins).
    pub async fn connect_mcp(
        &mut self,
        servers: &[crate::mcp::McpServerConfig],
        executor: &dyn crate::exec::Executor,
        cwd: &Path,
    ) -> Vec<crate::mcp::McpStatus> {
        let mut statuses = Vec::new();
        for cfg in servers {
            match crate::mcp::McpClient::connect(cfg, executor, cwd).await {
                Ok(client) => {
                    let client = Arc::new(client);
                    let mut added = Vec::new();
                    for exec in client.executors() {
                        let name = exec.name().to_string();
                        if self.tools.iter().any(|t| t.name() == name) {
                            continue; // keep the first registration of a name
                        }
                        added.push(name);
                        self.register_deferred(exec);
                    }
                    statuses.push(crate::mcp::McpStatus {
                        server: cfg.name.clone(),
                        connected: true,
                        tool_count: added.len(),
                        error: None,
                        tools: added,
                    });
                    self._mcp_clients.push(client);
                }
                Err(error) => statuses.push(crate::mcp::McpStatus {
                    server: cfg.name.clone(),
                    connected: false,
                    tool_count: 0,
                    error: Some(error),
                    tools: Vec::new(),
                }),
            }
        }
        statuses
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolExecutor>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    pub fn executors(&self) -> impl Iterator<Item = Arc<dyn ToolExecutor>> + '_ {
        self.tools.iter().cloned()
    }

    pub(crate) fn deferred_tool_gate(
        &self,
        session: Arc<tokio::sync::Mutex<crate::loop_state::SessionState>>,
    ) -> Arc<dyn agent_loop::plugin::ToolGate> {
        Arc::new(deferred::DeferredToolGate::new(
            self.deferred_catalog.clone(),
            session,
        ))
    }

    /// Every registered schema in declaration order. Runtime requests apply
    /// the deferred tool gate before producing their `tools` array.
    #[cfg(test)]
    pub fn schemas(&self) -> Vec<crate::llm::ToolSchema> {
        self.tools
            .iter()
            .map(|t| crate::llm::ToolSchema::function(t.name(), t.description(), t.parameters()))
            .collect()
    }
}
