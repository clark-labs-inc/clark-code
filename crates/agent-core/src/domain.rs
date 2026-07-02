//! The normalized domain vocabulary.
//!
//! Every provider adapter translates its wire format into these types, and the
//! projection layer consumes them. This is the superset normalization of ACP's
//! `session/update` variants and Clark's envelope `kind`s.

use serde::{Deserialize, Serialize};

use crate::ids::{PermissionRequestId, RunId, SessionId, ToolCallId};

/// Who produced a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Agent,
    System,
}

/// A piece of message or tool content. Mirrors ACP content blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        mime_type: String,
        /// Base64-encoded bytes.
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    Audio {
        mime_type: String,
        data: String,
    },
    Resource {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }
}

/// What a tool call is doing — used for iconography and grouping. Not a routing
/// signal; purely presentational.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    Research,
    #[default]
    Other,
}

/// Lifecycle of a tool call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// A file (and optional 1-based line) a tool touched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsLocation {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// A single tool invocation surfaced to the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub title: String,
    #[serde(default)]
    pub kind: ToolKind,
    pub status: ToolStatus,
    #[serde(default)]
    pub locations: Vec<FsLocation>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    /// Opaque, provider-specific raw input for debugging/inspection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<serde_json::Value>,
}

/// A partial update to an existing [`ToolCall`]. Fields left `None` are unchanged.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCallPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ToolKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<FsLocation>>,
    /// Content blocks to append (streaming tool output).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub append_content: Vec<ContentBlock>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPhaseStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanPhase {
    pub title: String,
    pub status: PlanPhaseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// The agent's current plan (ACP `plan` update / Clark plan phases).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub phases: Vec<PlanPhase>,
}

/// How a permission option resolves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionOption {
    pub id: String,
    pub label: String,
    pub kind: PermissionOptionKind,
}

/// A human-in-the-loop authorization request (ACP `session/request_permission`,
/// Clark confirmation gate). The UI must resolve it before the agent proceeds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: PermissionRequestId,
    pub session: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallId>,
    pub title: String,
    pub options: Vec<PermissionOption>,
    /// What the action will do, shown verbatim for review (a shell command, a
    /// file path, or a diff). Lets the user see *exactly* what they're approving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Risk class for shell commands: "safe" | "caution" | "danger". Drives the
    /// gate's styling and the auto-approve policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    /// Short, user-facing reason the action was flagged ("recursive delete").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The agent workspace sub-surface a focus event points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSurfaceKind {
    Browser,
    Terminal,
    Files,
    Website,
}

/// Where the agent wants the "computer" surface focused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceFocus {
    pub surface: WorkspaceSurfaceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_dir: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    File,
    Image,
    Pdf,
    Office,
    Slides,
    Media,
    Video,
    Website,
    Diff,
    SearchResults,
    Other,
}

/// A durable output the user can open/preview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub title: String,
    pub kind: ArtifactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallId>,
}

/// Lifecycle of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    AwaitingInput,
    Done,
    Cancelled,
    Failed,
}

/// Terminal result of a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Token/cost accounting summed over the run's model calls, when the
    /// provider surfaces it (the local coding loop does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunUsage>,
}

/// Aggregated model usage for one run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunUsage {
    /// Total prompt tokens across the run's model calls.
    pub input_tokens: u64,
    /// Total completion tokens across the run's model calls.
    pub output_tokens: u64,
    /// Prompt size of the LAST model call — the conversation's live context
    /// footprint (roughly what the next turn starts from).
    pub context_tokens: u64,
    /// Upstream USD cost summed across calls, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// A file the user attached but a provider hasn't ingested yet. The bytes ride
/// base64-encoded; each provider decides how to make it available to the agent
/// (ACP → content blocks, Clark → an attachment record, …).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUpload {
    pub filename: String,
    pub content_type: String,
    /// File bytes, base64-encoded.
    pub data_base64: String,
}

impl PendingUpload {
    pub fn is_image(&self) -> bool {
        self.content_type.starts_with("image/")
    }
    pub fn is_text(&self) -> bool {
        self.content_type.starts_with("text/")
            || matches!(
                self.content_type.as_str(),
                "application/json" | "application/xml" | "application/javascript"
            )
    }
}

/// Lifecycle of one child agent in a parallel `subagent_map` fan-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FanOutStatus {
    Queued,
    Running,
    Done,
    Failed,
}

/// A single agent tile in the fan-out surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanOutAgent {
    pub id: String,
    pub label: String,
    pub status: FanOutStatus,
}

/// Aggregate state of a live parallel fan-out (one `subagent_map` split across
/// many child agents), projected from per-child `subagent_event` telemetry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanOut {
    pub title: String,
    pub total: usize,
    pub done: usize,
    pub running: usize,
    pub agents: Vec<FanOutAgent>,
}

/// The single normalized event every provider emits. Projection consumes these.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    RunStarted {
        run: RunId,
    },
    /// A working-tree snapshot was taken before this run's edits, so the UI can
    /// offer "undo this run". `id` is an opaque restore handle (a git SHA).
    Checkpoint {
        run: RunId,
        id: String,
    },
    MessageChunk {
        run: RunId,
        role: Role,
        delta: ContentBlock,
    },
    ToolCall {
        run: RunId,
        call: ToolCall,
    },
    ToolCallUpdate {
        run: RunId,
        id: ToolCallId,
        patch: ToolCallPatch,
    },
    Plan {
        run: RunId,
        plan: Plan,
    },
    PermissionRequest {
        request: PermissionRequest,
    },
    Artifact {
        run: RunId,
        artifact: Artifact,
    },
    Surface {
        focus: WorkspaceFocus,
    },
    /// A live parallel fan-out update: one child agent of a `subagent_map`
    /// reported progress. Projection accumulates these into `Snapshot::fan_out`.
    FanOut {
        run: RunId,
        parent: ToolCallId,
        agent: FanOutAgent,
    },
    ModeChanged {
        session: SessionId,
        mode: String,
    },
    RunFinished {
        run: RunId,
        outcome: RunOutcome,
    },
    Error {
        code: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<RunId>,
    },
}
