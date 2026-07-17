//! The provider abstraction — the heart of the app. One trait fronts every
//! agentic backend (a local ACP CLI over stdio, the Clark runtime over a
//! WebSocket, an in-WASM client). The engine and all UI are provider-agnostic.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::domain::{
    AgentEvent, ContentBlock, FsLocation, PendingUpload, Role, ToolKind, ToolStatus,
};
use crate::error::Result;
use crate::ids::{PermissionRequestId, ProviderId, RunId, SessionId};

/// A stream of normalized events for one prompt/run. Boxed so the trait stays
/// object-safe and adapters can return any concrete stream.
pub type EventStream = BoxStream<'static, AgentEvent>;

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
    /// Named operating modes (ACP modes / Clark tiers).
    pub modes: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Typed transcript of the conversation being reopened. Providers that
    /// cannot resume server-side replay this into their canonical history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeTranscript>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResumeTranscript {
    pub items: Vec<ResumeItem>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// Authoritative filesystem binding for this conversation. UI actions must
    /// use this instead of a global project-folder preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<SessionEnvironment>,
}

/// A user turn to send to the agent: text/content plus any attached files. Each
/// provider ingests `attachments` its own way (ACP → content blocks, Clark → an
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
}

/// The provider abstraction.
///
/// Adapters translate their wire protocol into [`AgentEvent`]s and accept
/// [`ClientResponse`]s for host-served requests (permissions, fs, terminal).
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;

    fn capabilities(&self) -> ProviderCapabilities;

    /// Establish the connection / spawn the agent. Idempotent.
    async fn connect(&mut self, config: ProviderConfig) -> Result<()>;

    async fn new_session(&mut self, options: SessionOptions) -> Result<Session>;

    /// Resume a prior session (capability-gated).
    async fn load_session(&mut self, id: SessionId) -> Result<Session>;

    /// Send a user turn; returns the run's normalized event stream.
    async fn prompt(&mut self, session: &SessionId, input: PromptInput) -> Result<EventStream>;

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

    /// Switch the session's output style/persona (e.g. `"terse"`). A separate
    /// axis from `set_mode` — kept as its own method rather than overloading
    /// mode, since plan-mode (a boolean-ish gate) and output style (a
    /// prompt-tone choice) are independent and shouldn't be conflated.
    /// Best-effort: providers with no notion of output style leave this a no-op.
    async fn set_output_style(&mut self, _session: &SessionId, _style: String) -> Result<()> {
        Ok(())
    }
}
