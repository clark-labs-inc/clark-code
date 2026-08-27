//! The provider abstraction — the heart of the app. One trait fronts every
//! agentic backend (a local ACP CLI over stdio, a managed runtime over a
//! WebSocket, an in-WASM client). The engine and all UI are provider-agnostic.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::domain::{
    AgentEvent, ContentBlock, FsLocation, GoalState, PendingUpload, ProposedPlan, Role, ToolKind,
    ToolStatus,
};
use crate::error::Result;
use crate::ids::{PermissionRequestId, ProviderId, RunId, SessionId};

/// Attachment families a provider can ingest without discarding content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Text,
    Image,
    Audio,
    Pdf,
    Docx,
    Binary,
}

/// A stream of normalized events for one prompt/run. Boxed so the trait stays
/// object-safe and adapters can return any concrete stream.
pub type EventStream = BoxStream<'static, AgentEvent>;

/// A detached, one-off answer that owns everything it needs to finish. The
/// static lifetime lets clients poll it beside an active provider event stream
/// without holding a borrow of the primary session runtime.
pub type SideQuestionFuture = BoxFuture<'static, Result<String>>;

/// What a provider can do. Surfaces render only what is advertised here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    /// Can request human authorization for tool calls.
    pub permissions: bool,
    /// Can ask the client to read/write files.
    pub fs: bool,
    /// Can ask the client to run terminal commands.
    pub terminal: bool,
    /// Supports loading/resuming a prior session.
    pub load_session: bool,
    /// Raw attachment families accepted by this provider's prompt boundary.
    #[serde(default)]
    pub attachment_kinds: Vec<AttachmentKind>,
    /// Named operating modes (ACP modes / managed-provider tiers).
    pub modes: Vec<String>,
    /// Host collaboration modes supported independently of provider-native modes.
    #[serde(default)]
    pub collaboration_modes: Vec<CollaborationMode>,
}

/// One host-owned model choice advertised by a provider. The terminal and
/// desktop clients render this catalog; they do not maintain their own model
/// lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapability {
    pub id: String,
    pub label: String,
    pub description: String,
    /// Provider-selected reasoning policy for this model. `None` means the
    /// provider owns the default and exposes no client override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

/// One named reply style/personality implemented by a provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputStyleCapability {
    pub id: String,
    pub label: String,
    pub description: String,
}

/// One provider-owned experimental toggle. A client may only change entries
/// present in this list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentCapability {
    pub id: String,
    pub label: String,
    pub description: String,
    pub enabled: bool,
}

/// Effective, live configuration and the choices supported by one provider.
/// Empty fields mean the provider does not expose that control.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfiguration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,
    #[serde(default)]
    pub output_styles: Vec<OutputStyleCapability>,
    /// `None` means memory is not a user-configurable provider capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memories_enabled: Option<bool>,
    #[serde(default)]
    pub experiments: Vec<ExperimentCapability>,
}

/// A validated configuration mutation requested by a provider client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "setting", rename_all = "snake_case")]
pub enum ProviderConfigurationChange {
    Model { model: String },
    OutputStyle { style: String },
    Memories { enabled: bool },
    Experiment { id: String, enabled: bool },
}

/// Provider-owned state of a long-lived terminal task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BackgroundTaskState {
    Running,
    Stopping,
    Exited { code: Option<i32> },
    Failed { message: String },
}

/// One session-owned terminal task. Its stable id is also returned in the
/// originating tool result, keeping the terminal view associated with that
/// exact tool call without parsing prose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundTask {
    pub id: String,
    pub command: String,
    pub state: BackgroundTaskState,
    pub output: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationMode {
    #[default]
    Default,
    Plan,
}

/// How to reach a provider. Adapters read the fields they care about.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Network endpoint (e.g. `wss://host/ws`) for socket providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Command + args to spawn for stdio providers (ACP sidecar).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// Working directory for spawned providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// Bearer/bypass token for socket auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// Provider-specific extras.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub extra: serde_json::Value,
}

/// Options when opening a new session.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionOptions {
    /// Host-assigned identity for the conversation. Providers that allocate
    /// session-scoped filesystem state must use this identity before creating
    /// that state so native handles remain bound to the public conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_mode: Option<CollaborationMode>,
    /// Typed transcript of the conversation being reopened. Providers that
    /// cannot resume server-side replay this into their canonical history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeTranscript>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResumeTranscript {
    pub items: Vec<ResumeItem>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum ResumeItem {
    Message {
        role: Role,
        blocks: Vec<ContentBlock>,
    },
    ToolCall {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
        title: String,
        kind: ToolKind,
        status: ToolStatus,
        #[serde(default)]
        locations: Vec<FsLocation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<serde_json::Value>,
        #[serde(default)]
        content: Vec<ContentBlock>,
    },
    /// Standing goal state carried alongside transcript replay for providers
    /// without server-side session persistence.
    Goal { goal: GoalState },
    /// Latest typed plan proposal carried through history replay.
    ProposedPlan { plan: ProposedPlan },
}

/// A connected conversation with one provider.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionEnvironment {
    /// The checkout whose files and commands this session operates on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_root: Option<String>,
    /// The root of the main Git repository shared by linked worktrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_root: Option<String>,
    /// Every root the session may intentionally access.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_roots: Vec<String>,
    /// App-managed output root, when one is attached to the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_root: Option<String>,
    #[serde(default)]
    pub remote: bool,
}

/// A connected conversation with one provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub provider: ProviderId,
    pub capabilities: ProviderCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default)]
    pub collaboration_mode: CollaborationMode,
    /// Authoritative filesystem binding for this conversation. UI actions must
    /// use this instead of a global project-folder preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<SessionEnvironment>,
}

/// A user turn to send to the agent: text/content plus any attached files. Each
/// provider ingests `attachments` its own way (ACP → content blocks, managed providers → an
/// attachment record), keeping the UI provider-agnostic.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PromptInput {
    pub blocks: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<PendingUpload>,
}

impl PromptInput {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            blocks: vec![ContentBlock::text(s)],
            attachments: Vec::new(),
        }
    }
}

/// A client → provider reply that resolves a host-side request the agent made.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientResponse {
    /// Resolve a [`crate::domain::PermissionRequest`] by chosen option id.
    Permission {
        request: PermissionRequestId,
        option: String,
        /// Optional free-text the user attached to their choice — e.g. plan
        /// feedback on a "keep planning" rejection, delivered to the model as
        /// the rejection reason so the same run can revise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feedback: Option<String>,
    },
    PlanDecision {
        plan_id: String,
        decision: PlanDecision,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PlanDecision {
    Implement {
        context: PlanImplementationContext,
    },
    ContinuePlanning {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feedback: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanImplementationContext {
    Current,
    Fresh,
}

/// The provider abstraction.
///
/// Adapters translate their wire protocol into [`AgentEvent`]s and accept
/// [`ClientResponse`]s for host-served requests (permissions, fs, terminal).
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;

    fn capabilities(&self) -> ProviderCapabilities;

    /// Return live configuration plus the provider-owned choices that clients
    /// may render. Providers with no configurable session return an empty
    /// descriptor rather than making clients guess.
    async fn configuration(&self, _session: &SessionId) -> Result<ProviderConfiguration> {
        Ok(ProviderConfiguration::default())
    }

    /// Apply one provider-advertised change to the active session and return
    /// the resulting authoritative configuration.
    async fn configure(
        &mut self,
        _session: &SessionId,
        _change: ProviderConfigurationChange,
    ) -> Result<ProviderConfiguration> {
        Err(crate::error::Error::Unsupported(
            "this provider does not expose configurable session settings".into(),
        ))
    }

    /// List long-lived terminal tasks owned by this exact session.
    async fn background_tasks(&self, _session: &SessionId) -> Result<Vec<BackgroundTask>> {
        Err(crate::error::Error::Unsupported(
            "this provider does not expose background terminal tasks".into(),
        ))
    }

    /// Stop one session-owned terminal task by its stable provider id.
    async fn stop_background_task(
        &mut self,
        _session: &SessionId,
        _task: &str,
    ) -> Result<BackgroundTask> {
        Err(crate::error::Error::Unsupported(
            "this provider does not expose background terminal tasks".into(),
        ))
    }

    /// Remove completed terminal records and return the exact records removed.
    async fn clean_background_tasks(
        &mut self,
        _session: &SessionId,
    ) -> Result<Vec<BackgroundTask>> {
        Err(crate::error::Error::Unsupported(
            "this provider does not expose background terminal tasks".into(),
        ))
    }

    /// Return the provider-owned standing goal for this exact session. `None`
    /// means the session has no goal; clients must not infer one from prose.
    async fn goal_state(&self, _session: &SessionId) -> Result<Option<GoalState>> {
        Ok(None)
    }

    /// Resume a blocked goal.
    async fn resume_goal(&mut self, _session: &SessionId) -> Result<GoalState> {
        Err(crate::error::Error::Unsupported(
            "this provider does not expose durable goal controls".into(),
        ))
    }

    /// Remove the session's standing goal. User-initiated on the host: the
    /// goal stops continuing and the projected receipt is retired.
    async fn clear_goal(&mut self, _session: &SessionId) -> Result<()> {
        Err(crate::error::Error::Unsupported(
            "this provider does not expose durable goal controls".into(),
        ))
    }

    /// Admit additional host-approved read-only roots to a live session.
    /// Providers that cannot safely change their filesystem boundary keep the
    /// default rejection instead of presenting a UI-only attachment.
    async fn add_read_roots(&mut self, _session: &SessionId, _roots: Vec<String>) -> Result<()> {
        Err(crate::error::Error::Unsupported(
            "this provider cannot add read-only roots to a live session".into(),
        ))
    }

    /// Revoke host-approved read-only roots from a live session.
    async fn remove_read_roots(&mut self, _session: &SessionId, _roots: Vec<String>) -> Result<()> {
        Err(crate::error::Error::Unsupported(
            "this provider cannot remove read-only roots from a live session".into(),
        ))
    }

    /// Establish the connection / spawn the agent. Idempotent.
    async fn connect(&mut self, config: ProviderConfig) -> Result<()>;

    async fn new_session(&mut self, options: SessionOptions) -> Result<Session>;

    /// Resume a prior session (capability-gated).
    async fn load_session(&mut self, id: SessionId) -> Result<Session>;

    /// Export the provider's canonical model-visible history for a local
    /// resume or fork. Providers with server-side persistence may reject this;
    /// clients can then retain the remote session id without manufacturing a
    /// second transcript.
    async fn session_transcript(&self, _session: &SessionId) -> Result<ResumeTranscript> {
        Err(crate::error::Error::Unsupported(
            "this provider does not export session transcripts".into(),
        ))
    }

    /// Side-effect-free admission check for a user turn. Hosts that journal
    /// user messages before starting provider work must call this first so a
    /// rejected command never becomes durable conversation history.
    async fn validate_prompt(&self, _session: &SessionId, _input: &PromptInput) -> Result<()> {
        Ok(())
    }

    /// Send a user turn; returns the run's normalized event stream.
    async fn prompt(&mut self, session: &SessionId, input: PromptInput) -> Result<EventStream>;

    /// Compact the provider's model-visible conversation history without
    /// adding a user message to the visible transcript.
    async fn compact(&mut self, _session: &SessionId) -> Result<EventStream> {
        Err(crate::error::Error::Unsupported(
            "this provider does not support explicit context compaction".into(),
        ))
    }

    /// Inject a user message into the session's ACTIVE run — it lands between
    /// tool batches (steering) instead of waiting for the run to end. Errors
    /// with `Unsupported` when the provider has no steering or no live run;
    /// callers fall back to queueing the message as a normal follow-up turn.
    async fn steer(&mut self, _session: &SessionId, _input: PromptInput) -> Result<()> {
        Err(crate::error::Error::Unsupported(
            "this provider does not support mid-run steering".into(),
        ))
    }

    /// Request cancellation for one exact run.
    ///
    /// Return [`crate::Error::RunNotActive`] when the run has already left the
    /// provider's live execution set. Hosts treat that result as idempotent
    /// success while the terminal event finishes projecting.
    async fn cancel(&mut self, session: &SessionId, run: &RunId) -> Result<()>;

    /// Release session-owned resources before the host drops the provider.
    /// Providers without long-lived resources may keep the default no-op.
    async fn close_session(&mut self, _session: &SessionId) -> Result<()> {
        Ok(())
    }

    /// Resolve a host-side request the agent made.
    async fn respond(&mut self, session: &SessionId, response: ClientResponse) -> Result<()>;

    /// Switch the session's named operating mode (e.g. `"plan"`). Best-effort:
    /// providers that don't support server-side modes leave this a no-op.
    async fn set_mode(&mut self, _session: &SessionId, _mode: String) -> Result<()> {
        Ok(())
    }

    /// Switch the host-owned collaboration mode independently of provider-native modes.
    async fn set_collaboration_mode(
        &mut self,
        _session: &SessionId,
        _mode: CollaborationMode,
    ) -> Result<()> {
        Ok(())
    }

    /// Switch the session's output style/persona (e.g. `"terse"`). A separate
    /// axis from `set_mode` — kept as its own method rather than overloading
    /// mode, since plan-mode (a boolean-ish gate) and output style (a
    /// prompt-tone choice) are independent and shouldn't be conflated.
    /// Best-effort: providers with no notion of output style leave this a no-op.
    async fn set_output_style(&mut self, _session: &SessionId, _style: String) -> Result<()> {
        Ok(())
    }

    /// Answer a one-off "side question" against the session's current context
    /// WITHOUT interrupting the active run or mutating session state — a
    /// forked, single-turn, tool-less model call (the `/btw` feature, ported
    /// from Claude Code's `runSideQuestion`). Returns the answer text directly
    /// (no event stream). Errors with `Unsupported` on providers that can't
    /// fork the context; callers surface the failure in the UI but never touch
    /// the main run.
    async fn side_question(&mut self, _session: &SessionId, _question: &str) -> Result<String> {
        Err(crate::error::Error::Unsupported(
            "this provider does not support side questions".into(),
        ))
    }

    /// Begin a detached side question whose result cannot mutate or interrupt
    /// the primary conversation. Providers must snapshot any required context
    /// before the returned future performs its model call.
    fn start_side_question(&self, _session: &SessionId, _question: &str) -> SideQuestionFuture {
        Box::pin(async {
            Err(crate::error::Error::Unsupported(
                "this provider does not support detached side questions".into(),
            ))
        })
    }
}
