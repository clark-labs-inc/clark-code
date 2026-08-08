//! The normalized domain vocabulary.
//!
//! Every provider adapter translates its wire format into these types, and the
//! projection layer consumes them. This is the superset normalization of ACP's
//! `session/update` variants and managed-provider envelope `kind`s.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{PermissionRequestId, RunId, SessionId, ToolCallId};

/// Who produced a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Agent,
    System,
}

/// Whether assistant-authored text is an in-flight work update or the turn's
/// terminal answer. Providers that cannot distinguish the two may leave the
/// phase unset and let projection infer it from later tool/run events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagePhase {
    Commentary,
    FinalAnswer,
}

/// A piece of message or tool content. Mirrors ACP content blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    /// Hidden model reasoning (GLM `delta.reasoning` / a `<thinking>` block),
    /// surfaced as a collapsible Thinking row. Display-only — never sent back
    /// to the model on the next turn.
    Thinking {
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
        /// Base64-encoded original bytes when the resource came from an
        /// attachment and must survive restart/replay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<String>,
    },
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// A user-selected host-provided skill. Identity and content revision are
    /// carried separately from display text so the provider can reject stale
    /// or ambiguous bindings before a run starts.
    SkillReference {
        id: String,
        revision: String,
        name: String,
    },
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }
    pub fn thinking(s: impl Into<String>) -> Self {
        ContentBlock::Thinking { text: s.into() }
    }
    pub fn skill_reference(
        id: impl Into<String>,
        revision: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        ContentBlock::SkillReference {
            id: id.into(),
            revision: revision.into(),
            name: name.into(),
        }
    }
}

/// A specialist-owned result surface rendered inline in the conversation.
/// The presentation is structured so the client can expose maps, evidence,
/// metrics, and lifecycle stages without parsing assistant prose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecialistPresentationMetric {
    pub label: String,
    pub value: String,
    pub detail: String,
    pub progress: u8,
    pub tone: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecialistPresentationEvidence {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub source: String,
    pub freshness: String,
    pub confidence: u8,
    pub status: String,
    pub tone: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecialistPresentationStage {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecialistPresentation {
    pub id: String,
    pub kind: String,
    pub prompt: String,
    pub title: String,
    pub summary: String,
    pub takeaway: String,
    pub diagram: String,
    pub diagram_title: String,
    pub metrics: Vec<SpecialistPresentationMetric>,
    pub evidence: Vec<SpecialistPresentationEvidence>,
    pub stages: Vec<SpecialistPresentationStage>,
    pub limitation: String,
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
    /// Inspect an image in a safe workspace path.
    ViewImage,
    /// Create or edit an image and save the result as a workspace artifact.
    GenerateImage,
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
    /// The enclosing run ended before the tool produced a terminal receipt.
    /// Distinct from failure: cancellation may be an intentional user stop.
    Cancelled,
    Failed,
}

/// A file (and optional 1-based line) a tool touched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsLocation {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Structured public progress for a long-running delegated tool call.
///
/// This deliberately carries presentation-safe labels and summaries rather
/// than provider-native events, raw tool arguments, or model reasoning. A
/// provider may replace the complete value as newer progress arrives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallProgress {
    /// Monotonic presentation revision. Consumers can cheaply skip unchanged
    /// progress snapshots without deep-comparing a potentially large outline.
    #[serde(default)]
    pub revision: u64,
    pub status: ToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_activity: Option<String>,
    #[serde(default)]
    pub phases: Vec<ToolProgressPhase>,
    #[serde(default)]
    pub agents: Vec<ToolProgressAgent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolProgressPhase {
    pub id: String,
    pub title: String,
    pub status: ToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub steps: Vec<ToolProgressStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolProgressStep {
    pub id: String,
    pub title: String,
    pub status: ToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolProgressAgent {
    pub id: String,
    pub label: String,
    pub status: ToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// A single tool invocation surfaced to the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    /// Provider/tool-registry identifier. Kept out of user-facing labels but
    /// preserved for typed transcript replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
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
    /// Latest structured public progress for this tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ToolCallProgress>,
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
    /// Replace the call's content wholesale — the final result superseding any
    /// streamed partials, so progress lines don't linger (or duplicate the
    /// result) once the call completes. Applied before `append_content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace_content: Option<Vec<ContentBlock>>,
    /// Replace the call's structured progress wholesale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ToolCallProgress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChecklistStep {
    /// Stable step identity from an approved execution contract.
    ///
    /// `None` keeps ordinary ad-hoc checklists backward compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_step_id: Option<String>,
    pub title: String,
    pub status: ChecklistStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// The agent's live execution checklist (ACP `plan` update / managed-provider plan phases).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionChecklist {
    pub steps: Vec<ChecklistStep>,
    #[serde(default)]
    pub revision: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposedPlanStatus {
    AwaitingDecision,
    Approved,
    Superseded,
}

/// One planner-authored execution obligation.
///
/// Field order is intentional. Provider-facing serialization commits the
/// model to identity and objective before locations, completion evidence, and
/// reminders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanExecutionStep {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub done_when: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<String>,
}

/// A decision-complete implementation proposal produced in read-only Plan Mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposedPlan {
    pub id: String,
    pub revision: u32,
    pub markdown: String,
    pub status: ProposedPlanStatus,
    /// Cross-step invariants authored before the execution steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_reminders: Vec<String>,
    /// Typed obligations used to bind runtime checklists and reminders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub execution_contract: Vec<PlanExecutionStep>,
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
/// managed-provider confirmation gate). The UI must resolve it before the agent proceeds.
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

/// Machine-readable reason a run failed. Providers classify failures at their
/// transport boundary so presentation never has to infer auth, rate limits, or
/// provider state from human-readable error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunFailureKind {
    SessionExpired,
    PlatformKeyRejected,
    ProviderError,
    RateLimited,
    TransportError,
    ContextOverflow,
    InsufficientCredits,
    ToolFatal,
    LocalState,
    /// The provider exhausted an explicitly configured model/tool step budget
    /// before producing a terminal answer. The saved conversation can be
    /// continued; this is not corrupt local state.
    IterationLimit,
    /// The host process ended while a run was live. Distinct from an explicit
    /// user cancellation and safe for recovery surfaces to offer as resumable.
    RuntimeInterrupted,
    /// A user-visible final answer was committed, but one or more external
    /// effects from that run could not be independently verified afterward.
    VerificationIncomplete,
    EmptyResponse,
}

/// Terminal result of a run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunExecutionSummary {
    /// Stable identity for the root execution tree. A normal single-agent run
    /// has this root and no children.
    pub execution_id: String,
    pub root_path: String,
    pub attempts: u32,
    pub recoveries: u32,
    #[serde(default)]
    pub child_executions: u32,
    #[serde(default)]
    pub completed_children: u32,
    #[serde(default)]
    pub failed_children: u32,
    pub weighted_tokens: f64,
    pub cost_usd: f64,
    #[serde(default)]
    pub changed_paths: Vec<String>,
    #[serde(default)]
    pub completed_tools: Vec<String>,
    #[serde(default)]
    pub failed_tools: Vec<String>,
}

/// Terminal result of a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<RunFailureKind>,
    /// Token/cost accounting summed over the run's model calls, when the
    /// provider surfaces it (the local coding loop does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunUsage>,
    /// Runtime-derived execution receipt. Providers that do not expose a root
    /// lifecycle ledger leave this absent for backwards compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<RunExecutionSummary>,
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
    /// The engine's auto-compaction threshold in tokens, when known — the
    /// denominator for an honest UI context meter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
}

/// Lifecycle of a standing session goal. Unlike a run, a goal can span many
/// user turns and survives reopening the conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Blocked,
    BudgetLimited,
    Complete,
}

/// Authoritative, provider-owned receipt for a standing goal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalState {
    pub id: String,
    pub objective: String,
    pub status: GoalStatus,
    /// Most recent run that advanced or changed this goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub time_used_seconds: u64,
    pub continuations: u32,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker_reason: Option<String>,
}

/// A file the user attached but a provider hasn't ingested yet. The bytes ride
/// base64-encoded; each provider decides how to make it available to the agent
/// (ACP → content blocks, managed providers → an attachment record, …).
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

    pub fn kind(&self) -> crate::provider::AttachmentKind {
        use crate::provider::AttachmentKind;
        let content_type = self.content_type.to_ascii_lowercase();
        let filename = self.filename.to_ascii_lowercase();
        if self.is_image() {
            AttachmentKind::Image
        } else if content_type.starts_with("audio/") {
            AttachmentKind::Audio
        } else if content_type == "application/pdf" || filename.ends_with(".pdf") {
            AttachmentKind::Pdf
        } else if content_type
            == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            || filename.ends_with(".docx")
        {
            AttachmentKind::Docx
        } else if self.is_text() {
            AttachmentKind::Text
        } else {
            AttachmentKind::Binary
        }
    }

    /// The presentation echo for the conversation timeline. Images carry their
    /// (composer-downscaled) bytes so the UI can render a thumbnail; every
    /// other file becomes a data-less link chip, so a multi-MB binary never
    /// lands in the snapshot that hosts clone and re-emit per streamed token.
    pub fn echo_block(&self) -> ContentBlock {
        if self.is_image() {
            ContentBlock::Image {
                mime_type: self.content_type.clone(),
                data: self.data_base64.clone(),
                uri: None,
            }
        } else if self.content_type.starts_with("audio/") {
            ContentBlock::Audio {
                mime_type: self.content_type.clone(),
                data: self.data_base64.clone(),
            }
        } else {
            ContentBlock::Resource {
                uri: format!("attachment://{}", self.filename),
                mime_type: Some(self.content_type.clone()),
                text: None,
                data: Some(self.data_base64.clone()),
            }
        }
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
    /// Full backend-authored task objective. Presentation may use `label` as a
    /// compact selector while preserving this text in the inspector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    /// Latest public progress update. This must never contain hidden reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Final public result or failure summary, when the child has settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
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
    /// A working-tree snapshot was taken before this run for change tracking.
    /// `id` is the checkpoint commit SHA used as a diff baseline.
    Checkpoint {
        run: RunId,
        id: String,
    },
    MessageChunk {
        run: RunId,
        role: Role,
        delta: ContentBlock,
    },
    /// Classify the latest still-unphased assistant message for this run.
    /// Emitted after streaming when the provider learns whether more work
    /// follows (for example, a response that also contains tool calls).
    MessagePhase {
        run: RunId,
        phase: MessagePhase,
    },
    SpecialistPresentation {
        run: RunId,
        presentation: SpecialistPresentation,
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
    ExecutionChecklistUpdated {
        run: RunId,
        checklist: ExecutionChecklist,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        explanation: Option<String>,
    },
    ProposedPlanUpdated {
        run: RunId,
        plan: ProposedPlan,
    },
    /// A standing goal changed status or usage. Providers emit the complete
    /// state so replay and reconnect remain deterministic.
    GoalUpdated {
        run: RunId,
        goal: GoalState,
    },
    /// Cumulative token/cost accounting for a run so presentation can update
    /// after each completed model call instead of waiting for the terminal
    /// outcome. Providers emit the complete total to keep replay idempotent.
    RunUsageUpdated {
        run: RunId,
        usage: RunUsage,
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
    /// A provider incident changed state. Observation is independent from any
    /// optional execution recovery so failed incidents remain diagnosable.
    ProviderIncidentUpdated {
        run: RunId,
        incident: crate::recovery::ProviderIncident,
    },
    ModeChanged {
        session: SessionId,
        mode: String,
    },
    /// Durable replacement for the provider's model-visible history. The UI
    /// timeline stays intact; replay uses this checkpoint plus later items.
    ContextCompacted {
        run: RunId,
        transcript: crate::provider::ResumeTranscript,
    },
    /// Provider-native trajectory detail that does not participate in the
    /// presentation projection. Hosts persist this verbatim for replay and
    /// debugging (for example the full model-visible request and compaction
    /// before/after transcript emitted by the local agent loop).
    Trace {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<RunId>,
        source: String,
        payload: Value,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_tool_call_without_progress_deserializes() {
        let call: ToolCall = serde_json::from_value(serde_json::json!({
            "id": "call-1",
            "title": "Researching",
            "kind": "research",
            "status": "in_progress",
            "locations": [],
            "content": []
        }))
        .unwrap();
        assert!(call.progress.is_none());
    }

    #[test]
    fn run_failure_kind_is_machine_readable_and_backward_compatible() {
        let outcome = RunOutcome {
            status: RunStatus::Failed,
            stop_reason: None,
            error: Some("raw provider detail".into()),
            failure_kind: Some(RunFailureKind::PlatformKeyRejected),
            usage: None,
            execution: None,
        };
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["failure_kind"], "platform_key_rejected");

        let legacy: RunOutcome = serde_json::from_value(serde_json::json!({
            "status": "failed",
            "error": "old untyped error"
        }))
        .unwrap();
        assert_eq!(legacy.failure_kind, None);
        assert_eq!(legacy.execution, None);
    }

    #[test]
    fn pending_upload_echo_block_keeps_typed_attachment_bytes() {
        let image = PendingUpload {
            filename: "shot.webp".into(),
            content_type: "image/webp".into(),
            data_base64: "aGVsbG8".into(),
        };
        assert_eq!(
            image.echo_block(),
            ContentBlock::Image {
                mime_type: "image/webp".into(),
                data: "aGVsbG8".into(),
                uri: None,
            }
        );

        let pdf = PendingUpload {
            filename: "spec.pdf".into(),
            content_type: "application/pdf".into(),
            data_base64: "huge".into(),
        };
        assert_eq!(
            pdf.echo_block(),
            ContentBlock::Resource {
                uri: "attachment://spec.pdf".into(),
                mime_type: Some("application/pdf".into()),
                text: None,
                data: Some("huge".into()),
            }
        );
    }

    #[test]
    fn pending_upload_kind_uses_mime_and_document_extension_fallbacks() {
        use crate::provider::AttachmentKind;

        let upload = |filename: &str, content_type: &str| PendingUpload {
            filename: filename.into(),
            content_type: content_type.into(),
            data_base64: String::new(),
        };
        assert_eq!(
            upload("note.txt", "text/plain").kind(),
            AttachmentKind::Text
        );
        assert_eq!(
            upload("scan.PDF", "application/octet-stream").kind(),
            AttachmentKind::Pdf
        );
        assert_eq!(
            upload("report.docx", "application/zip").kind(),
            AttachmentKind::Docx
        );
        assert_eq!(
            upload("voice.wav", "audio/wav").kind(),
            AttachmentKind::Audio
        );
        assert_eq!(
            upload("archive.bin", "application/octet-stream").kind(),
            AttachmentKind::Binary
        );
    }
}
