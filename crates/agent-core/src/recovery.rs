use serde::{Deserialize, Serialize};

/// The failing boundary. Keeping this separate from the category prevents a
/// dropped event stream or provider process from being mislabeled as a model
/// generation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderIncidentScope {
    ModelRequest,
    ProviderEventStream,
    ProviderProcess,
    CloudHistorySync,
    ToolExecutionHost,
}

/// User-facing classification chosen at the failing transport boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderIncidentCategory {
    Timeout,
    RateLimit,
    UpstreamUnavailable,
    ConnectionLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderIncidentStatus {
    /// Failure observed; the runtime has not yet chosen a recovery action.
    Observed,
    Retrying,
    Recovered,
    Failed,
    /// The host stopped before a truthful terminal provider outcome arrived.
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFailureClass {
    TransientTransport,
    RateLimited,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRetryCounts {
    pub transient: u32,
    pub rate_limit: u32,
    pub authentication: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRequestDiagnostics {
    /// Provider-generated key reused across request-local retries.
    pub idempotency_key: String,
    /// Provider/gateway-generated request identifier, when returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    /// Number of network attempts already made for this logical request.
    pub attempts: u32,
    pub max_attempts: u32,
    #[serde(default)]
    pub retries: ProviderRetryCounts,
    /// True once any assistant content, reasoning, or tool-call delta arrived.
    pub output_started: bool,
    pub started_at_ms: u64,
}

/// Exact durable boundary used when the orchestration layer elects to recover.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBoundaryReceipt {
    pub execution_id: String,
    pub attempt_sequence: u32,
    pub event_sequence: u64,
    pub transcript_commit_id: String,
    pub completed_tools: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_checkpoint_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRecovery {
    pub attempt: u32,
    pub boundary: ExecutionBoundaryReceipt,
    pub started_at_ms: u64,
}

/// Durable, redacted lifecycle for one provider incident. Incident observation
/// is independent from whether an execution retry is safe or even possible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderIncident {
    pub id: String,
    pub status: ProviderIncidentStatus,
    pub scope: ProviderIncidentScope,
    pub failure_class: ProviderFailureClass,
    pub category: ProviderIncidentCategory,
    /// Safe summary for the collapsed card.
    pub message: String,
    /// Redacted, bounded provider detail available only in diagnostics.
    pub detail: String,
    pub model: String,
    pub provider_route: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_error_type: Option<String>,
    pub request: ProviderRequestDiagnostics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_recovery: Option<ExecutionRecovery>,
    pub observed_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
}
