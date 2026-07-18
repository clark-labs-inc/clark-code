use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::budget::UsageCharge;
use crate::contract::{AgentPath, HarnessKind, OrchestrationId, ReadOnlyTask, StructuredReport};

#[derive(Clone, Debug)]
pub struct AttemptContext {
    pub orchestration_id: OrchestrationId,
    pub agent_path: AgentPath,
    pub task: ReadOnlyTask,
    pub attempt: u32,
    pub parent_context: String,
    pub feedback: Option<String>,
    pub cancel: CancellationToken,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HarnessEvent {
    Progress { message: String },
    Tool { name: String, summary: String },
    Warning { message: String },
}

pub type HarnessEventSink = Arc<dyn Fn(HarnessEvent) + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessAttempt {
    pub provider: String,
    pub model: String,
    pub final_message: String,
    pub report: Option<StructuredReport>,
    pub usage: UsageCharge,
    /// Defense in depth for harnesses that can independently detect a write.
    pub observed_write: bool,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HarnessError {
    #[error("harness is not configured: {0}")]
    NotConfigured(String),
    #[error("harness rejected the task: {0}")]
    Rejected(String),
    #[error("harness timed out: {0}")]
    TimedOut(String),
    #[error("harness failed: {0}")]
    Failed(String),
    #[error("harness was cancelled")]
    Cancelled,
}

#[async_trait]
pub trait ReadOnlyHarness: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> HarnessKind;

    /// Run one attempt. Implementations must enforce read-only execution at the
    /// provider or OS boundary; a prompt instruction alone is insufficient.
    async fn run(
        &self,
        context: AttemptContext,
        events: HarnessEventSink,
    ) -> Result<HarnessAttempt, HarnessError>;
}
