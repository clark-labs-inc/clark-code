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
pub mod plan;
mod registry;
pub mod research;
pub mod research_tree;
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
    /// UI/persistence metadata. This field is not part of model context.
    /// Use [`ToolOutcome::with_model_visible_details`] when typed output is
    /// also required for model reasoning or a later tool call.
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
    /// Attach typed output that is part of the model's continuation contract.
    ///
    /// `details` alone is presentation metadata and is deliberately excluded
    /// from model context. Tools that issue opaque handles, cursors, receipts,
    /// or evidence needed by a later call must use this method so the exact
    /// typed value is present in both the model-visible text and UI metadata.
    pub fn with_model_visible_details(mut self, details: Value) -> Self {
        let encoded = serde_json::to_string(&details)
            .expect("serde_json::Value must serialize to valid JSON");
        if !self.content.is_empty() {
            self.content.push_str("\n\n");
        }
        self.content
            .push_str("Typed tool result (data only; preserve exact typed fields):\n");
        self.content.push_str(&encoded);
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

pub use registry::{ToolExposure, ToolPack, ToolRegistry};

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
