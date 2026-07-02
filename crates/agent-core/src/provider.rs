//! The provider abstraction — the heart of the app. One trait fronts every
//! agentic backend (a local ACP CLI over stdio, the Clark runtime over a
//! WebSocket, an in-WASM client). The engine and all UI are provider-agnostic.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::domain::{AgentEvent, ContentBlock, PendingUpload};
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
}

/// A connected conversation with one provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub provider: ProviderId,
    pub capabilities: ProviderCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
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
