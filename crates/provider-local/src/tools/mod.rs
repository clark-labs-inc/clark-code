//! The tool layer the model drives.
//!
//! Every tool — whether it edits a local file, runs a local shell command, or
//! delegates work to a product capability — implements one [`ToolExecutor`]
//! trait. Execution uses one flat registry while the model initially sees only
//! core schemas and discovers deferred capabilities through `tool_search`.
//! Local executors hold
//! a [`Sandbox`]; remote executors carry their own client. This is the seam that
//! lets coding stay local while optional product tools run behind their own boundary.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use agent_core::domain::{
    ArtifactKind, ExecutionChecklist, FanOutAgent, FsLocation, GoalState, ProposedPlan,
    ToolCallProgress, ToolKind,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::AuxiliaryModelConfig;
use crate::sandbox::Sandbox;

pub mod android_emulator;
pub mod apply_patch;
pub mod browser;
pub mod computer_use;
mod deferred;
pub mod diagnostics;
pub mod document;
pub mod effect;
pub mod final_answer;
pub mod fs;
pub mod goal;
pub mod grep;
pub mod image;
#[cfg(target_os = "macos")]
pub mod ios_simulator;
pub mod memory;
pub mod mobile;
pub mod organization_knowledge;
pub mod plan;
pub mod security_poc_execute;
pub mod security_scan_contract;
pub mod shell;
pub mod skill;
pub mod web_fetch;

/// Tracks which files the model has read this session, and their modification
/// time at read. This enforces the read-before-edit/write invariant (a Claude
/// Code best practice): the model must see a file's current contents before
/// changing it, and an edit/write fails if the file changed on disk since the
/// read — preventing blind or stale overwrites.
#[derive(Default)]
pub struct ReadTracker {
    seen: HashMap<PathBuf, SystemTime>,
}

/// Result of checking whether a path is safe to mutate.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadCheck {
    /// Read this session and unchanged since — safe to edit/write.
    Fresh,
    /// Never read this session.
    NotRead,
    /// Read, but the file changed on disk since — must be re-read.
    Stale,
}

impl ReadTracker {
    /// Record that `path` was read, capturing its current mtime.
    pub fn record(&mut self, path: &Path, mtime: SystemTime) {
        self.seen.insert(path.to_path_buf(), mtime);
    }

    /// Check whether `path` (currently at `current` mtime) may be mutated.
    pub fn check(&self, path: &Path, current: SystemTime) -> ReadCheck {
        match self.seen.get(path) {
            None => ReadCheck::NotRead,
            // Allow a small tolerance: only flag clearly newer mtimes as stale.
            Some(&seen) if current > seen => ReadCheck::Stale,
            Some(_) => ReadCheck::Fresh,
        }
    }
}

/// How a tool is gated before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionMode {
    /// Run without asking.
    Allow,
    /// Ask the user once; remember the answer if they choose "always".
    Ask,
    /// Never run; feed a denial back to the model.
    Deny,
}

/// Authorization class is independent of whether a tool mutates local state.
/// In particular, brokered cloud is a trusted brokered capability while direct
/// network access still needs consent even for an HTTP GET.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolPermissionClass {
    LocalRead,
    LocalMutation,
    External,
    BrokeredProduct,
}

/// Invocation-specific permission identity. Most tools are gated by their
/// static tool name; capabilities that cross a finer trust boundary can bind
/// remembered decisions to a narrower key (for example one target app).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionScope {
    pub key: String,
    pub title: Option<String>,
    pub always_label: Option<String>,
    pub reason: Option<String>,
    /// Optional request risk override. `"confirm"` is reserved for actions
    /// that must receive an explicit human answer even under Full access.
    pub risk: Option<String>,
    /// False for one-off or unusually sensitive actions. The permission UI
    /// omits its "always" choice and the gate refuses to persist one.
    pub remember: bool,
    /// Trusted invocation-specific authorization already exists. This is
    /// computed by the executor from native state, never accepted from model
    /// arguments, and bypasses the generic session policy prompt.
    pub preapproved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolPermissionDecision {
    AllowOnce,
    AllowAlways,
    Denied,
}

impl ToolPermissionClass {
    pub fn requires_gate(self) -> bool {
        matches!(self, Self::LocalMutation | Self::External)
    }
}

impl PermissionMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "allow" | "always" | "yes" => Some(Self::Allow),
            "ask" | "prompt" | "confirm" => Some(Self::Ask),
            "deny" | "never" | "no" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// Per-invocation context handed to every tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TurnModelOverride {
    pub model: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone)]
pub struct ToolCtx {
    /// Project-root containment for file tools.
    pub sandbox: Arc<Sandbox>,
    /// The process-local execution backend used by tools. Remote coding runs
    /// this provider and executor together inside the durable worker.
    pub executor: Arc<dyn crate::exec::Executor>,
    /// Session-scoped read tracker enforcing read-before-edit/write.
    pub reads: Arc<Mutex<ReadTracker>>,
    /// Fires when the run is cancelled; long tools should bail on it.
    pub cancel: CancellationToken,
    /// Session-scoped registry of `bash(run_in_background: true)` tasks.
    pub background: Arc<crate::background::BackgroundTasks>,
    /// Session state — `check_diagnostics` reads `check_command` and the
    /// stored baseline from here. `tokio::sync::Mutex` (not the `std` one
    /// aliased above for `ReadTracker`) since it's held across `.await` points.
    #[allow(private_interfaces)]
    pub session: Arc<tokio::sync::Mutex<crate::loop_state::SessionState>>,
    /// Per-call live-progress sink: long tools (shell, grep) push text deltas
    /// here and they stream to the UI's tool row while the call runs. `None`
    /// outside a run (tests, session-setup helpers).
    pub progress: Option<ProgressFn>,
    /// Typed child-lifecycle progress for orchestration tools. This stays
    /// separate from textual tool output so presentation never has to infer
    /// agent identity or status from log strings.
    pub agent_progress: Option<AgentProgressFn>,
    /// Structured, presentation-safe progress for a long-running delegated
    /// tool call. Kept separate from text output so UI state never depends on
    /// parsing human narration.
    pub call_progress: Option<CallProgressFn>,
    /// Host-owned per-turn model policy. Skill and prompt arguments cannot
    /// supply or modify it; orchestration uses it to keep delegated model
    /// execution on the same fixed policy as the root.
    pub(crate) model_override: Option<TurnModelOverride>,
}

/// A tool's live-progress callback — each call appends a text delta to the
/// in-flight tool call in the UI.
pub type ProgressFn = Arc<dyn Fn(String) + Send + Sync>;

/// A typed child-agent update projected into the conversation's parallel-work
/// surface by the desktop adapter.
pub type AgentProgressFn = Arc<dyn Fn(FanOutAgent) + Send + Sync>;

/// A complete replacement snapshot of one tool call's public run outline.
pub type CallProgressFn = Arc<dyn Fn(ToolCallProgress) + Send + Sync>;

impl ToolCtx {
    /// Stream a live-progress text delta to the UI for the in-flight call.
    pub(crate) fn report(&self, delta: impl Into<String>) {
        if let Some(progress) = &self.progress {
            progress(delta.into());
        }
    }

    /// Replace the in-flight call's structured public progress snapshot.
    pub fn report_call_progress(&self, progress: ToolCallProgress) {
        if let Some(report) = &self.call_progress {
            report(progress);
        }
    }

    /// Record a successful read of `path` (canonical) at its current mtime.
    /// mtime comes from the executor, so the invariant holds for remote files too.
    pub(crate) async fn note_read(&self, path: &Path) {
        if let Some(mtime) = self.executor.mtime(path).await {
            if let Ok(mut reads) = self.reads.lock() {
                reads.record(path, mtime);
            }
        }
    }

    /// Verify `path` may be mutated, returning a model-facing error if not.
    /// `must_exist=false` (write) lets brand-new files through without a read.
    pub(crate) async fn guard_mutation(&self, path: &Path, must_exist: bool) -> Result<(), String> {
        let current = self.executor.mtime(path).await;
        if current.is_none() {
            // File doesn't exist: only writes (creating it) are allowed.
            return if must_exist {
                Err(format!("{} does not exist", path.display()))
            } else {
                Ok(())
            };
        }
        let check = self
            .reads
            .lock()
            .map(|r| r.check(path, current.unwrap()))
            .unwrap_or(ReadCheck::Fresh);
        match check {
            ReadCheck::Fresh => Ok(()),
            ReadCheck::NotRead => Err(format!(
                "{} has not been read yet — use read_file to read it before editing or overwriting it.",
                path.display()
            )),
            ReadCheck::Stale => Err(format!(
                "{} has changed on disk since it was last read — read_file it again before editing.",
                path.display()
            )),
        }
    }
}

/// The result of running a tool: text fed back to the model, plus presentational
/// hints for the UI.
#[derive(Clone, Debug, Default)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    pub locations: Vec<FsLocation>,
    pub images: Vec<ImageAttachment>,
    /// Typed, user-visible outputs made by the tool. These are projected into
    /// desktop artifact cards by the adapter rather than inferred from text or
    /// a generic file location.
    pub artifacts: Vec<ProducedArtifact>,
    pub details: Value,
    /// Typed state changes emitted by tools. The desktop adapter attaches the
    /// active run id and forwards them without switching on tool names.
    pub signals: Vec<ToolSignal>,
}

#[derive(Clone, Debug)]
pub enum ToolSignal {
    ExecutionChecklist {
        checklist: ExecutionChecklist,
        explanation: Option<String>,
    },
    ProposedPlan(ProposedPlan),
    Goal(GoalState),
}

/// An image a tool wants to attach to its result. The adapter either forwards
/// it to a model with native image support or derives a bounded vision
/// description, while always preserving the typed bytes for the UI. Tools that
/// create a durable output additionally emit a [`ProducedArtifact`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageAttachment {
    pub mime_type: String,
    pub data_base64: String,
    pub alt: Option<String>,
}

/// A durable user-facing result emitted by a tool.
///
/// The URI may be a `data:` URL when the image was produced on a remote
/// executor: Clark Code cannot safely read arbitrary remote paths, but
/// it can render bytes the tool just received from the trusted platform relay.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProducedArtifact {
    pub id: String,
    pub title: String,
    pub kind: ArtifactKind,
    pub mime_type: Option<String>,
    pub uri: Option<String>,
}

impl ToolOutcome {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            locations: Vec::new(),
            images: Vec::new(),
            artifacts: Vec::new(),
            details: Value::Null,
            signals: Vec::new(),
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: format!("Error: {}", message.into()),
            is_error: true,
            locations: Vec::new(),
            images: Vec::new(),
            artifacts: Vec::new(),
            details: Value::Null,
            signals: Vec::new(),
        }
    }
    pub fn with_location(mut self, path: impl Into<String>, line: Option<u32>) -> Self {
        self.locations.push(FsLocation {
            path: path.into(),
            line,
        });
        self
    }
    pub fn with_image(
        mut self,
        mime_type: impl Into<String>,
        data_base64: impl Into<String>,
        alt: Option<String>,
    ) -> Self {
        self.images.push(ImageAttachment {
            mime_type: mime_type.into(),
            data_base64: data_base64.into(),
            alt,
        });
        self
    }
    pub fn with_artifact(mut self, artifact: ProducedArtifact) -> Self {
        self.artifacts.push(artifact);
        self
    }
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
    pub fn with_signal(mut self, signal: ToolSignal) -> Self {
        self.signals.push(signal);
        self
    }
}

/// A tool the model can call. Object-safe so the registry can hold a mix of
/// local and remote executors.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON-Schema for the tool's arguments.
    fn parameters(&self) -> Value;
    /// Presentational classification for the UI; never a routing signal.
    fn kind(&self) -> ToolKind;
    /// Whether the call mutates state and must pass the permission gate.
    fn mutating(&self) -> bool {
        false
    }
    /// Whether a successful result is the typed final delivery boundary.
    fn terminates_run(&self) -> bool {
        false
    }
    /// Invocation-level mutation classification for mixed read/write schemas.
    /// The static flag continues to control ordinary permission prompting;
    /// Plan Mode uses this exact argument-aware contract to keep its read-only
    /// boundary intact.
    fn mutating_for_args(&self, _args: &Value) -> bool {
        self.mutating()
    }
    fn permission_class(&self) -> ToolPermissionClass {
        if self.mutating() {
            ToolPermissionClass::LocalMutation
        } else {
            ToolPermissionClass::LocalRead
        }
    }
    /// Optional invocation-specific permission scope. The key is internal
    /// session policy state; title/labels/reason are presentation only.
    fn permission_scope(&self, _args: &Value) -> Option<PermissionScope> {
        None
    }
    /// Validate safety-critical arguments before the permission gate can
    /// display or remember an approval. Tool bodies must validate again.
    fn permission_preflight(&self, _args: &Value) -> Result<(), String> {
        Ok(())
    }
    /// A durable or externally visible effect this invocation may produce.
    /// Authorization and effect verification are deliberately separate: a
    /// user can approve an action without asserting that its final state is
    /// correct.
    #[allow(private_interfaces)]
    fn effect_intent(&self, _args: &Value) -> Option<crate::effects::EffectIntent> {
        None
    }
    /// A read-only preview of what `invoke` would change, shown in the permission
    /// gate so the user reviews edits *before* they touch disk. Default: none.
    fn preview(&self, _args: &Value, _ctx: &ToolCtx) -> Option<String> {
        None
    }
    /// Apply a permission answer to a capability whose authorization is owned
    /// by a trusted backend. Called while the desktop permission queue is
    /// still held, before a remembered generic policy is made visible.
    async fn permission_decision(
        &self,
        _args: &Value,
        _decision: ToolPermissionDecision,
        _ctx: &ToolCtx,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome;
}

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

    /// The standard local coding tools, plus Clark Code research tool when a
    /// research endpoint is configured, plus the `memory` tool when memories are
    /// enabled (`memory` is `Some` with the local global dir + optional Clark Code
    /// personal-recall config).
    pub fn new(
        research: Option<AuxiliaryModelConfig>,
        memory: Option<memory::MemoryConfig>,
    ) -> Self {
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
        let _ = research;
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

    /// Register the bounded orchestration tools. Explicitly disabled and
    /// fail-closed child configurations never advertise them.
    pub(crate) fn enable_orchestration(
        &mut self,
        config: crate::orchestration::OrchestrationToolsConfig,
    ) {
        for tool in crate::orchestration::orchestration_tools(config) {
            self.register_deferred(tool);
        }
    }

    /// Register only the target-local adapter census and signed capsule client.
    /// This remains available for remote execution targets without enabling
    /// nested child-process orchestration on those targets.
    pub(crate) fn enable_scout_capsules(
        &mut self,
        policy: crate::orchestration::ScoutCapsulePolicyConfig,
    ) {
        for tool in crate::orchestration::scout_capsule_tools(policy) {
            self.register_deferred(tool);
        }
    }

    /// Register organization recall independently of the optional research
    /// agent. A Platform key is sufficient; authorization is rechecked by the
    /// service for every read.
    pub fn enable_organization_knowledge(
        &mut self,
        provider: Arc<dyn crate::platform::PlatformContextProvider>,
    ) {
        self.register_deferred(Arc::new(
            organization_knowledge::OrganizationKnowledgeTool::new(provider),
        ));
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

/// Pull a required string argument, with a model-friendly error on miss.
pub(crate) fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing required string argument `{key}`"))
}

/// Pull an optional string argument.
pub(crate) fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Pull a required integer argument, with a model-friendly error on miss.
pub(crate) fn arg_i64(args: &Value, key: &str) -> Result<i64, String> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("missing required integer argument `{key}`"))
}

/// Pull an optional integer argument.
pub(crate) fn arg_i64_opt(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
