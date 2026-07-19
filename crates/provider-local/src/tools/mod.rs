//! The tool layer the model drives.
//!
//! Every tool — whether it edits a local file, runs a local shell command, or
//! delegates research to Clark's sandbox — implements one [`ToolExecutor`]
//! trait. The model sees a single flat tool list ([`ToolRegistry::schemas`]);
//! execution routes to the right backend behind the trait. Local executors hold
//! a [`Sandbox`]; remote executors carry their own client. This is the seam that
//! lets coding stay local while research runs in Clark's sandbox.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use agent_core::domain::{FanOutAgent, FsLocation, ToolKind};
use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::AgenticClarkConfig;
use crate::sandbox::Sandbox;

pub mod android_emulator;
pub mod apply_patch;
pub mod browser;
pub mod clark;
pub mod diagnostics;
pub mod fs;
pub mod goal;
pub mod grep;
#[cfg(target_os = "macos")]
pub mod ios_simulator;
pub mod memory;
pub mod mobile;
pub mod organization_knowledge;
pub mod plan;
pub mod shell;
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
#[derive(Clone)]
pub struct ToolCtx {
    /// Project-root containment for file tools.
    pub sandbox: Arc<Sandbox>,
    /// The execution backend the tools perform their I/O through — local today,
    /// or a remote host (over the exec-server) for a remote project.
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
    pub session: Arc<tokio::sync::Mutex<crate::loop_state::SessionState>>,
    /// Per-call live-progress sink: long tools (shell, grep) push text deltas
    /// here and they stream to the UI's tool row while the call runs. `None`
    /// outside a run (tests, session-setup helpers).
    pub progress: Option<ProgressFn>,
    /// Typed child-lifecycle progress for orchestration tools. This stays
    /// separate from textual tool output so presentation never has to infer
    /// agent identity or status from log strings.
    pub agent_progress: Option<AgentProgressFn>,
}

/// A tool's live-progress callback — each call appends a text delta to the
/// in-flight tool call in the UI.
pub type ProgressFn = Arc<dyn Fn(String) + Send + Sync>;

/// A typed child-agent update projected into the conversation's parallel-work
/// surface by the desktop adapter.
pub type AgentProgressFn = Arc<dyn Fn(FanOutAgent) + Send + Sync>;

impl ToolCtx {
    /// Stream a live-progress text delta to the UI for the in-flight call.
    pub(crate) fn report(&self, delta: impl Into<String>) {
        if let Some(progress) = &self.progress {
            progress(delta.into());
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
    pub details: Value,
}

/// An image a tool wants to attach to its result — both shown to the model
/// (as a synthetic follow-up multimodal turn, since tool-role messages can't
/// carry image content-parts on the OpenAI-compatible wire format) and, when
/// it also arrives via `with_location`, rendered as an Artifact card.
#[derive(Clone, Debug)]
pub struct ImageAttachment {
    pub mime_type: String,
    pub data_base64: String,
    pub alt: Option<String>,
}

impl ToolOutcome {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            locations: Vec::new(),
            images: Vec::new(),
            details: Value::Null,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: format!("Error: {}", message.into()),
            is_error: true,
            locations: Vec::new(),
            images: Vec::new(),
            details: Value::Null,
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
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
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
    /// A read-only preview of what `invoke` would change, shown in the permission
    /// gate so the user reviews edits *before* they touch disk. Default: none.
    fn preview(&self, _args: &Value, _ctx: &ToolCtx) -> Option<String> {
        None
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome;
}

/// The ordered set of tools advertised to the model.
pub struct ToolRegistry {
    tools: Vec<Arc<dyn ToolExecutor>>,
    /// Live MCP server connections, kept alive for the registry's lifetime.
    _mcp_clients: Vec<Arc<crate::mcp::McpClient>>,
}

impl ToolRegistry {
    /// The standard local coding tools, plus the Clark research tool when a
    /// research endpoint is configured, plus the `memory` tool when memories are
    /// enabled (`memory` is `Some` with the local global dir + optional Clark
    /// personal-recall config).
    pub fn new(clark: Option<AgenticClarkConfig>, memory: Option<memory::MemoryConfig>) -> Self {
        let mut tools: Vec<Arc<dyn ToolExecutor>> = vec![
            Arc::new(fs::ReadFile),
            Arc::new(fs::ListDir),
            Arc::new(fs::Glob),
            Arc::new(grep::Grep),
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
            Arc::new(goal::CreateGoal),
            Arc::new(goal::UpdateGoal),
            Arc::new(goal::GetGoal),
            Arc::new(web_fetch::WebFetchTool::new(clark.clone())),
            Arc::new(diagnostics::CheckDiagnostics),
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
        ];
        #[cfg(target_os = "macos")]
        {
            tools.push(Arc::new(ios_simulator::ListSimulators));
            tools.push(Arc::new(ios_simulator::BootSimulator));
            tools.push(Arc::new(ios_simulator::ShutdownSimulator));
            tools.push(Arc::new(ios_simulator::InstallApp));
            tools.push(Arc::new(ios_simulator::UninstallApp));
            tools.push(Arc::new(ios_simulator::LaunchApp));
            tools.push(Arc::new(ios_simulator::Screenshot));
            tools.push(Arc::new(ios_simulator::Tap));
            tools.push(Arc::new(ios_simulator::Swipe));
            tools.push(Arc::new(ios_simulator::TypeText));
            tools.push(Arc::new(ios_simulator::PressButton));
        }
        if let Some(cfg) = clark {
            tools.push(Arc::new(clark::ClarkResearchTool::new(cfg)));
        }
        if let Some(cfg) = memory {
            tools.push(Arc::new(memory::MemoryTool::new(
                cfg.global_dir,
                cfg.personal,
            )));
        }
        Self {
            tools,
            _mcp_clients: Vec::new(),
        }
    }

    /// Register the opt-in, experimental `browser` tool (clark-browser,
    /// downloaded on first use). Called separately from `new()`, gated by the
    /// user's Settings toggle (off by default) — the tool isn't even
    /// advertised to the model unless enabled.
    pub fn enable_browser(&mut self) {
        self.tools.push(Arc::new(browser::BrowserTool::new()));
    }

    /// Remove device-control capabilities that are physically attached to the
    /// Clark Desktop machine. Remote sessions must discover and operate any
    /// mobile SDK or emulator on their SSH host through the remote shell.
    pub fn disable_desktop_mobile_tools(&mut self) {
        self.tools.retain(|tool| {
            !tool.name().starts_with("android_") && !tool.name().starts_with("ios_")
        });
    }

    /// Register the bounded orchestration tools. Explicitly disabled and
    /// fail-closed child configurations never advertise them.
    pub fn enable_orchestration(&mut self, config: crate::orchestration::OrchestrationToolsConfig) {
        self.tools
            .extend(crate::orchestration::orchestration_tools(config));
    }

    /// Register organization recall independently of the optional research
    /// agent. A Platform key is sufficient; authorization is rechecked by the
    /// service for every read.
    pub fn enable_organization_knowledge(
        &mut self,
        config: organization_knowledge::OrganizationKnowledgeConfig,
    ) {
        self.tools.push(Arc::new(
            organization_knowledge::OrganizationKnowledgeTool::new(config),
        ));
    }

    /// Connect the configured MCP servers and register their tools. A server
    /// that fails to start is skipped (not fatal); the returned statuses let the
    /// UI show what connected. Tool-name collisions are dropped (first wins).
    pub async fn connect_mcp(
        &mut self,
        servers: &[crate::mcp::McpServerConfig],
    ) -> Vec<crate::mcp::McpStatus> {
        let mut statuses = Vec::new();
        for cfg in servers {
            match crate::mcp::McpClient::connect(cfg).await {
                Ok(client) => {
                    let client = Arc::new(client);
                    let mut added = Vec::new();
                    for exec in client.executors() {
                        let name = exec.name().to_string();
                        if self.tools.iter().any(|t| t.name() == name) {
                            continue; // keep the first registration of a name
                        }
                        added.push(name);
                        self.tools.push(exec);
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

    /// Tool schemas in declaration order, for the request `tools` array.
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
mod tests {
    use super::*;

    #[test]
    fn read_tracker_distinguishes_fresh_notread_and_stale() {
        use std::time::{Duration, SystemTime};
        let mut t = ReadTracker::default();
        let path = Path::new("/proj/a.rs");
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        // Never recorded → NotRead.
        assert_eq!(t.check(path, t0), ReadCheck::NotRead);
        // Recorded at t0, unchanged → Fresh.
        t.record(path, t0);
        assert_eq!(t.check(path, t0), ReadCheck::Fresh);
        // File now newer than the recorded read → Stale.
        let t1 = t0 + Duration::from_secs(5);
        assert_eq!(t.check(path, t1), ReadCheck::Stale);
    }

    #[test]
    fn permission_mode_parses_synonyms() {
        assert_eq!(PermissionMode::parse("allow"), Some(PermissionMode::Allow));
        assert_eq!(PermissionMode::parse("ASK"), Some(PermissionMode::Ask));
        assert_eq!(PermissionMode::parse(" deny "), Some(PermissionMode::Deny));
        assert_eq!(PermissionMode::parse("maybe"), None);
    }

    #[test]
    fn schema_property_order_survives_serialization() {
        // Tool schemas are autoregressive prompts: the model emits arguments
        // in the property order it sees, so authored order must reach the
        // wire. Without serde_json's `preserve_order` feature the json!{}
        // maps alphabetize (new_string before path) — this test pins the
        // feature and the intended orders.
        fn wire_order(registry: &ToolRegistry, tool: &str, props: &[&str]) {
            let schema = registry
                .schemas()
                .into_iter()
                .find(|s| s.function.name == tool)
                .unwrap_or_else(|| panic!("{tool} not registered"));
            let wire = serde_json::to_string(&schema.function.parameters).unwrap();
            let positions: Vec<usize> = props
                .iter()
                .map(|p| {
                    wire.find(&format!("\"{p}\""))
                        .unwrap_or_else(|| panic!("{tool}: {p} missing from schema"))
                })
                .collect();
            assert!(
                positions.windows(2).all(|w| w[0] < w[1]),
                "{tool}: properties out of order on the wire: {wire}"
            );
        }
        let reg = ToolRegistry::new(None, Some(memory::MemoryConfig::default()));
        // Locate before payload: the model must commit to where/what it is
        // replacing before it generates the replacement.
        wire_order(&reg, "edit_file", &["path", "old_string", "new_string"]);
        wire_order(&reg, "write_file", &["path", "content"]);
        wire_order(&reg, "read_file", &["path", "offset", "limit"]);
        // Commit to the command and its location/mode before tuning timeout.
        wire_order(
            &reg,
            "bash",
            &["command", "workdir", "run_in_background", "timeout_ms"],
        );
        wire_order(
            &reg,
            "bash_wait",
            &[
                "task_id",
                "output_contains",
                "timeout_ms",
                "poll_interval_ms",
            ],
        );
        wire_order(&reg, "bash_input", &["task_id", "text", "close"]);
        // Decide the action, scope, and provenance before the fact being saved.
        wire_order(
            &reg,
            "memory",
            &["action", "scope", "source", "title", "content"],
        );
        // Rationale first: explanation tokens condition the plan steps.
        wire_order(&reg, "update_plan", &["explanation", "plan"]);
        wire_order(&reg, "grep", &["pattern", "path"]);
    }

    #[test]
    fn registry_lists_local_tools_and_optionally_clark() {
        let local = ToolRegistry::new(None, None);
        let names: Vec<_> = local
            .schemas()
            .iter()
            .map(|s| s.function.name.clone())
            .collect();
        assert!(names.contains(&"read_file".to_string()));
        assert!(names.contains(&"edit_file".to_string()));
        assert!(names.contains(&"bash".to_string()));
        assert!(!names.contains(&"clark_research".to_string()));
        assert!(!names.contains(&"memory".to_string()));
        assert!(local.get("read_file").is_some());
        assert!(local.get("nope").is_none());

        let with_clark = ToolRegistry::new(
            Some(AgenticClarkConfig {
                base_url: "https://api.clarkslabs.com/v1".into(),
                api_key: Some("ck_live_x".into()),
                model: "clark".into(),
            }),
            None,
        );
        let names: Vec<_> = with_clark
            .schemas()
            .iter()
            .map(|s| s.function.name.clone())
            .collect();
        assert!(names.contains(&"clark_research".to_string()));
    }

    #[test]
    fn memory_tool_registered_only_when_enabled() {
        let off = ToolRegistry::new(None, None);
        assert!(off.get("memory").is_none());
        let on = ToolRegistry::new(None, Some(memory::MemoryConfig::default()));
        assert!(on.get("memory").is_some());
        // Memory writes are curated + path-constrained, so they don't gate.
        assert!(!on.get("memory").unwrap().mutating());
    }

    #[test]
    fn organization_knowledge_is_an_explicit_read_only_registry_plugin() {
        let mut registry = ToolRegistry::new(None, None);
        assert!(registry.get("organization_knowledge").is_none());
        registry.enable_organization_knowledge(
            organization_knowledge::OrganizationKnowledgeConfig {
                base_url: "https://api.clarkslabs.com/v1".into(),
                api_key: "ck_live_test".into(),
            },
        );
        let tool = registry.get("organization_knowledge").unwrap();
        assert!(!tool.mutating());
        assert_eq!(tool.kind(), ToolKind::Research);
    }

    #[test]
    fn mutating_tools_are_flagged() {
        let reg = ToolRegistry::new(None, None);
        assert!(reg.get("write_file").unwrap().mutating());
        assert!(reg.get("edit_file").unwrap().mutating());
        assert!(reg.get("bash").unwrap().mutating());
        assert!(!reg.get("read_file").unwrap().mutating());
        assert!(!reg.get("grep").unwrap().mutating());
    }

    #[test]
    fn plan_tools_are_registered_with_correct_mutating_flags() {
        let reg = ToolRegistry::new(None, None);
        assert!(reg.get("propose_plan").unwrap().mutating());
        assert!(!reg.get("update_plan").unwrap().mutating());
    }

    #[test]
    fn web_fetch_is_always_registered_and_non_mutating() {
        let reg = ToolRegistry::new(None, None);
        let t = reg.get("web_fetch").unwrap();
        assert!(!t.mutating());
    }

    #[test]
    fn android_tools_are_registered_with_correct_mutating_flags() {
        let reg = ToolRegistry::new(None, None);
        // Read-only: never gate the user.
        assert!(!reg.get("android_list_devices").unwrap().mutating());
        assert!(!reg.get("android_screenshot").unwrap().mutating());
        // Mutating: one "always allow" confirm each.
        for name in [
            "android_boot_emulator",
            "android_shutdown_emulator",
            "android_install_app",
            "android_uninstall_app",
            "android_launch_app",
            "android_tap",
            "android_swipe",
            "android_type_text",
            "android_press_button",
        ] {
            assert!(
                reg.get(name).unwrap().mutating(),
                "{name} should be mutating"
            );
        }
    }

    #[test]
    fn remote_registry_does_not_expose_desktop_mobile_tools() {
        let mut reg = ToolRegistry::new(None, None);
        reg.disable_desktop_mobile_tools();
        let names: Vec<_> = reg
            .schemas()
            .into_iter()
            .map(|tool| tool.function.name)
            .collect();
        assert!(names.iter().all(|name| !name.starts_with("android_")));
        assert!(names.iter().all(|name| !name.starts_with("ios_")));
        assert!(names.iter().any(|name| name == "bash"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn ios_tools_are_registered_with_correct_mutating_flags() {
        let reg = ToolRegistry::new(None, None);
        // Read-only: never gate the user.
        assert!(!reg.get("ios_list_simulators").unwrap().mutating());
        assert!(!reg.get("ios_screenshot").unwrap().mutating());
        // Mutating: one "always allow" confirm each.
        for name in [
            "ios_boot_simulator",
            "ios_shutdown_simulator",
            "ios_install_app",
            "ios_uninstall_app",
            "ios_launch_app",
            "ios_tap",
            "ios_swipe",
            "ios_type_text",
            "ios_press_button",
        ] {
            assert!(
                reg.get(name).unwrap().mutating(),
                "{name} should be mutating"
            );
        }
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn ios_tools_are_absent_on_non_macos() {
        let reg = ToolRegistry::new(None, None);
        assert!(reg.get("ios_list_simulators").is_none());
    }
}
