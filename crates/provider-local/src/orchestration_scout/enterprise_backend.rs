use std::collections::BTreeSet;
use std::sync::Arc;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use scout_adapter_protocol::AdapterPageReceipt;
use scout_ingest_protocol::cartography::{
    ClaimedTask, GraphDeltaCursor, GraphObjectKind, GraphSnapshotCursor, SimulationOverlayCursor,
    TaskClaimResponse,
};
use scout_platform_client::{ScoutCartographySession, ScoutCartographySessionConfig};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::orchestration::{OrchestrationToolsConfig, ScoutCartographyHostConfig};
use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};

mod submit;

pub(super) struct CartographyBackendState {
    base_url: String,
    api_key: Option<String>,
    host: Option<ScoutCartographyHostConfig>,
    session: OnceCell<ScoutCartographySession>,
    pending: submit::PendingSubmissions,
    /// Enrollment, task leases, evidence submission, and graph reads share
    /// one session and must observe one ordering when the model emits a tool
    /// batch.  Do not let a query race session initialization or submission.
    operation_gate: tokio::sync::Mutex<()>,
}

impl CartographyBackendState {
    pub(super) fn new(config: OrchestrationToolsConfig) -> Self {
        Self {
            base_url: config.base_url,
            api_key: config.api_key,
            host: config.scout_cartography,
            session: OnceCell::new(),
            pending: submit::PendingSubmissions::default(),
            operation_gate: tokio::sync::Mutex::new(()),
        }
    }

    async fn enroll(&self) -> Result<&ScoutCartographySession, String> {
        let host = self.host.as_ref().ok_or_else(|| {
            "Scout enterprise backend is not host-configured; Agent Desktop must supply an exact organization, workspace, and private identity root".to_string()
        })?;
        let api_key = self.api_key.as_deref().ok_or_else(|| {
            "Scout enterprise backend requires the host-injected Agent Desktop Platform credential"
                .to_string()
        })?;
        self.session
            .get_or_try_init(|| async {
                let config = ScoutCartographySessionConfig::new(
                    &self.base_url,
                    api_key,
                    &host.route_prefix,
                    &host.identity_root,
                    host.organization_id,
                    host.workspace_id,
                    &host.platform,
                    &host.architecture,
                )?;
                ScoutCartographySession::enroll(config).await
            })
            .await
    }

    fn ready(&self) -> Result<&ScoutCartographySession, String> {
        self.session.get().ok_or_else(|| {
            "Scout collector is not enrolled in this session; call `scout_enterprise` with action `enroll` first".to_string()
        })
    }

    fn status(&self) -> Value {
        let binding = self.host.as_ref().map(|host| {
            json!({
                "organization_id": host.organization_id,
                "workspace_id": host.workspace_id,
                "platform": host.platform,
                "architecture": host.architecture,
            })
        });
        json!({
            "authority": "host_system_cartography_backend",
            "configured": self.host.is_some() && self.api_key.is_some(),
            "session_enrolled": self.session.get().is_some(),
            "binding": binding,
            "local_enterprise_authority": false,
        })
    }

    fn record_claim(&self, run_id: Uuid, task: ClaimedTask) -> Result<(), String> {
        self.pending.record_claim(run_id, task)
    }

    pub(super) fn record_receipt(&self, receipt: AdapterPageReceipt) -> Result<(), String> {
        self.pending.record_receipt(receipt)
    }
}

pub(super) struct ScoutEnterpriseBackendTool {
    pub state: Arc<CartographyBackendState>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EnterpriseAction {
    Enroll,
    ClaimTask,
    SubmitAdapterReceipt,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnterpriseArgs {
    action: EnterpriseAction,
    #[serde(default)]
    run_id: Option<Uuid>,
    #[serde(default)]
    lease_seconds: Option<i32>,
    #[serde(default)]
    task_id: Option<Uuid>,
    #[serde(default)]
    receipt_id: Option<String>,
}

impl EnterpriseArgs {
    fn validate(&self) -> Result<(), String> {
        match self.action {
            EnterpriseAction::Enroll
                if self.run_id.is_none()
                    && self.lease_seconds.is_none()
                    && self.task_id.is_none()
                    && self.receipt_id.is_none() =>
            {
                Ok(())
            }
            EnterpriseAction::ClaimTask
                if self.run_id.is_some()
                    && self.lease_seconds.is_some()
                    && self.task_id.is_none()
                    && self.receipt_id.is_none() =>
            {
                Ok(())
            }
            EnterpriseAction::SubmitAdapterReceipt
                if self.run_id.is_none()
                    && self.lease_seconds.is_none()
                    && self.task_id.is_some()
                    && self.receipt_id.is_some() =>
            {
                Ok(())
            }
            EnterpriseAction::Enroll => Err("enroll does not accept task or run arguments".into()),
            EnterpriseAction::ClaimTask => {
                Err("claim_task requires only a backend-issued run_id and lease_seconds".into())
            }
            EnterpriseAction::SubmitAdapterReceipt => Err(
                "submit_adapter_receipt requires only host-retained task_id and receipt_id".into(),
            ),
        }
    }
}

#[async_trait]
impl ToolExecutor for ScoutEnterpriseBackendTool {
    fn name(&self) -> &str {
        "scout_enterprise"
    }

    fn description(&self) -> &str {
        "Enroll this host's protected collector key with Agent Desktop's authoritative organization-scoped system-cartography backend, claim one fenced task from a backend-managed run, or submit a target-produced adapter receipt previously retained by this host. Submission uploads immutable evidence, translates the safe receipt under the backend-authored task scope, and ingests a signed batch. Organization, workspace, run binding, source, fence, Platform credential, signing key, and identity path are host-owned and cannot be supplied for submission."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["enroll", "claim_task", "submit_adapter_receipt"],
                    "description": "Choose the backend operation first."
                },
                "run_id": {
                    "type": "string",
                    "format": "uuid",
                    "description": "Backend-issued run id; required only for claim_task."
                },
                "lease_seconds": {
                    "type": "integer",
                    "minimum": 5,
                    "maximum": 3600,
                    "description": "Requested fenced lease duration; required only for claim_task."
                },
                "task_id": {
                    "type": "string",
                    "format": "uuid",
                    "description": "A task retained from this host's claim_task result; required only for submit_adapter_receipt."
                },
                "receipt_id": {
                    "type": "string",
                    "description": "A target-bound receipt id retained from this host's scout_adapter fetch_page result; required only for submit_adapter_receipt."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn mutating(&self) -> bool {
        true
    }

    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let _operation_gate = self.state.operation_gate.lock().await;
        let args: EnterpriseArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => {
                return ToolOutcome::error(format!(
                    "invalid Scout enterprise backend request: {error}"
                ))
            }
        };
        if let Err(error) = args.validate() {
            return ToolOutcome::error(error);
        }
        match args.action {
            EnterpriseAction::Enroll => match self.state.enroll().await {
                Ok(session) => {
                    let enrollment = session.enrollment();
                    ToolOutcome::ok(
                        "Scout collector is enrolled with Agent Desktop's authoritative backend.",
                    )
                    .with_details(json!({
                        "authority": "host_system_cartography_backend",
                        "organization_id": enrollment.organization_id,
                        "workspace_id": enrollment.workspace_id,
                        "machine_id": enrollment.id,
                        "signer_id": enrollment.signer_id,
                        "platform": enrollment.platform,
                        "architecture": enrollment.architecture,
                        "local_enterprise_authority": false,
                    }))
                }
                Err(error) => ToolOutcome::error(error),
            },
            EnterpriseAction::ClaimTask => {
                let Some(run_id) = args.run_id else {
                    return ToolOutcome::error("claim_task requires a backend-issued run_id");
                };
                let Some(lease_seconds) = args.lease_seconds else {
                    return ToolOutcome::error("claim_task requires lease_seconds");
                };
                let session = match self.state.ready() {
                    Ok(session) => session,
                    Err(error) => return ToolOutcome::error(error),
                };
                match session.claim_next_task(run_id, lease_seconds).await {
                    Ok(response) => {
                        if let Some(task) = &response.task {
                            if let Err(error) = self.state.record_claim(run_id, task.clone()) {
                                return ToolOutcome::error(error);
                            }
                        }
                        claim_task_outcome(response)
                    }
                    Err(error) => ToolOutcome::error(error),
                }
            }
            EnterpriseAction::SubmitAdapterReceipt => {
                let Some(task_id) = args.task_id else {
                    return ToolOutcome::error("submit_adapter_receipt requires task_id");
                };
                let Some(receipt_id) = args.receipt_id else {
                    return ToolOutcome::error("submit_adapter_receipt requires receipt_id");
                };
                match submit::submit_adapter_receipt(&self.state, task_id, &receipt_id).await {
                    Ok(submitted) => ToolOutcome::ok(
                        "Uploaded immutable adapter evidence and ingested its backend-fenced cartography batch.",
                    )
                    .with_details(json!({
                        "task_id": submitted.task_id,
                        "adapter_receipt_id": submitted.adapter_receipt_id,
                        "evidence_id": submitted.evidence.evidence_id,
                        "evidence_sha256": submitted.evidence.sha256,
                        "evidence_version_id": submitted.evidence.version_id,
                        "batch_receipt_id": submitted.acceptance.receipt.receipt_id,
                        "backend_sequence": submitted.acceptance.receipt.sequence,
                        "outcome": submitted.acceptance.outcome,
                        "inserted_events": submitted.acceptance.inserted_events,
                        "recorded_conflicts": submitted.acceptance.recorded_conflicts,
                        "authority": "host_system_cartography_backend",
                    })),
                    Err(error) => ToolOutcome::error(error),
                }
            }
        }
    }
}

fn claim_task_outcome(response: TaskClaimResponse) -> ToolOutcome {
    let content = match &response.task {
        Some(task) => match serde_json::to_string(task) {
            Ok(task) => format!(
                "Claimed one fenced Scout task from Agent Desktop. Copy this exact backend-issued task \
                 into the next adapter step; do not infer or replace any field:\n{task}"
            ),
            Err(_) => {
                return ToolOutcome::error(
                    "failed to encode the backend-issued Scout task for the model",
                )
            }
        },
        None => "Agent Desktop reports no claimable Scout task for this run.".to_string(),
    };
    ToolOutcome::ok(content).with_details(json!({
        "request_id": response.request_id,
        "task": response.task,
    }))
}

pub(super) struct ScoutEnterpriseBackendQueryTool {
    pub state: Arc<CartographyBackendState>,
}

impl ScoutEnterpriseBackendQueryTool {
    fn status_outcome(&self) -> ToolOutcome {
        let status = self.state.status();
        let configured = status["configured"].as_bool().unwrap_or(false);
        let enrolled = status["session_enrolled"].as_bool().unwrap_or(false);
        ToolOutcome::ok(format!(
            "Scout enterprise authority is Agent Desktop's organization-scoped backend; \
             configured={configured}, session_enrolled={enrolled}."
        ))
        .with_details(status)
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum QueryAction {
    Status,
    Snapshot,
    Delta,
    SimulationOverlay,
    Changes,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryArgs {
    action: QueryAction,
    #[serde(default)]
    from_effective_at_ms: Option<u64>,
    #[serde(default)]
    from_known_at_ms: Option<u64>,
    #[serde(default)]
    to_effective_at_ms: Option<u64>,
    #[serde(default)]
    to_known_at_ms: Option<u64>,
    #[serde(default)]
    effective_at_ms: Option<u64>,
    #[serde(default)]
    known_at_ms: Option<u64>,
    #[serde(default)]
    object_kinds: BTreeSet<GraphObjectKind>,
    #[serde(default = "default_snapshot_limit")]
    limit: u16,
    #[serde(default)]
    cursor: Option<GraphSnapshotCursor>,
    #[serde(default)]
    delta_cursor: Option<GraphDeltaCursor>,
    #[serde(default)]
    include_unchanged: bool,
    #[serde(default)]
    simulation_stable_key: Option<String>,
    #[serde(default)]
    simulation_version: Option<u64>,
    #[serde(default)]
    simulation_cursor: Option<SimulationOverlayCursor>,
    #[serde(default)]
    after_change_sequence: Option<u64>,
}

fn default_snapshot_limit() -> u16 {
    100
}

#[async_trait]
impl ToolExecutor for ScoutEnterpriseBackendQueryTool {
    fn name(&self) -> &str {
        "scout_enterprise_query"
    }

    fn description(&self) -> &str {
        "Read bounded current or bitemporal as-of graph snapshots, exact temporal deltas, versioned simulation coverage/result overlays, and the monotonic workspace change feed from Agent Desktop's authoritative system-cartography backend. Delta pages classify added, changed, removed, and optionally unchanged objects between two independently pinned bitemporal cuts, so they can show either business-system change or how Scout's knowledge grew over hours and days. Simulation overlays are immutable versions pinned to one exact graph snapshot. Change pages let clients refresh maps without rescanning. Tenant ids are fixed by trusted host configuration, never inferred from email domains or accepted from model arguments. This tool never reads a local enterprise database."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "snapshot", "delta", "simulation_overlay", "changes"],
                    "description": "Inspect the trusted backend binding, retrieve a snapshot, compare two cuts, page a simulation overlay, or poll the workspace change feed."
                },
                "from_effective_at_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Required for delta: the earlier business-effective cut."
                },
                "from_known_at_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional earlier transaction/knowledge cut; defaults to the later knowledge cut."
                },
                "to_effective_at_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional later delta cut; defaults to the pinned knowledge time."
                },
                "to_known_at_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional later transaction/knowledge cut; defaults to the backend database time."
                },
                "effective_at_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional business-effective as-of time."
                },
                "known_at_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional transaction/knowledge as-of time."
                },
                "object_kinds": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["entity", "edge", "claim", "coverage"]
                    },
                    "uniqueItems": true,
                    "description": "Empty means every graph object kind."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1000,
                    "description": "Maximum rows in this page."
                },
                "cursor": {
                    "type": "object",
                    "description": "Exact cursor returned by the previous snapshot page."
                },
                "delta_cursor": {
                    "type": "object",
                    "description": "Exact cursor returned by the previous delta page."
                },
                "include_unchanged": {
                    "type": "boolean",
                    "description": "For delta only, include objects whose selected event is identical at both cuts."
                },
                "simulation_stable_key": {
                    "type": "string",
                    "description": "Required for simulation_overlay: backend-owned stable simulation identity."
                },
                "simulation_version": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional immutable overlay version; defaults to latest."
                },
                "simulation_cursor": {
                    "type": "object",
                    "description": "Exact cursor returned by the previous simulation overlay page."
                },
                "after_change_sequence": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "For changes only, return workspace changes after this monotonic sequence."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let _operation_gate = self.state.operation_gate.lock().await;
        let args: QueryArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => {
                return ToolOutcome::error(format!("invalid Scout enterprise query: {error}"))
            }
        };
        match args.action {
            QueryAction::Status => self.status_outcome(),
            QueryAction::Snapshot => {
                let session = match self.state.ready() {
                    Ok(session) => session,
                    Err(error) => return ToolOutcome::error(error),
                };
                match session
                    .query_snapshot(
                        args.effective_at_ms,
                        args.known_at_ms,
                        args.object_kinds,
                        args.limit,
                        args.cursor,
                    )
                    .await
                {
                    Ok(page) => ToolOutcome::ok(format!(
                        "Read {} bounded graph rows from Agent Desktop.",
                        page.entries.len()
                    ))
                    .with_details(json!({
                        "organization_id": page.organization_id,
                        "workspace_id": page.workspace_id,
                        "effective_at_ms": page.effective_at_ms,
                        "known_at_ms": page.known_at_ms,
                        "entries": page.entries,
                        "next_cursor": page.next_cursor,
                        "authority": "host_system_cartography_backend",
                    })),
                    Err(error) => ToolOutcome::error(error),
                }
            }
            QueryAction::Delta => {
                let Some(from_effective_at_ms) = args.from_effective_at_ms else {
                    return ToolOutcome::error(
                        "delta requires the earlier from_effective_at_ms boundary",
                    );
                };
                if args.effective_at_ms.is_some() || args.cursor.is_some() {
                    return ToolOutcome::error(
                        "delta accepts from/to effective and knowledge boundaries plus delta_cursor; snapshot-only effective_at_ms and cursor are not allowed",
                    );
                }
                let session = match self.state.ready() {
                    Ok(session) => session,
                    Err(error) => return ToolOutcome::error(error),
                };
                match session
                    .query_delta(
                        from_effective_at_ms,
                        args.from_known_at_ms,
                        args.to_effective_at_ms,
                        args.to_known_at_ms,
                        args.object_kinds,
                        args.include_unchanged,
                        args.limit,
                        args.delta_cursor,
                    )
                    .await
                {
                    Ok(page) => ToolOutcome::ok(format!(
                        "Read {} temporal graph changes from Agent Desktop.",
                        page.entries.len()
                    ))
                    .with_details(json!({
                        "organization_id": page.organization_id,
                        "workspace_id": page.workspace_id,
                        "from_snapshot": page.from_snapshot,
                        "to_snapshot": page.to_snapshot,
                        "entries": page.entries,
                        "next_cursor": page.next_cursor,
                        "authority": "host_system_cartography_backend",
                    })),
                    Err(error) => ToolOutcome::error(error),
                }
            }
            QueryAction::SimulationOverlay => {
                let Some(stable_key) = args.simulation_stable_key else {
                    return ToolOutcome::error("simulation_overlay requires simulation_stable_key");
                };
                let session = match self.state.ready() {
                    Ok(session) => session,
                    Err(error) => return ToolOutcome::error(error),
                };
                match session
                    .query_simulation_overlay(
                        stable_key,
                        args.simulation_version,
                        args.limit,
                        args.simulation_cursor,
                    )
                    .await
                {
                    Ok(page) => ToolOutcome::ok(format!(
                        "Read {} simulation overlay memberships from Agent Desktop.",
                        page.memberships.len()
                    ))
                    .with_details(json!({
                        "overlay": page.overlay,
                        "memberships": page.memberships,
                        "next_cursor": page.next_cursor,
                        "authority": "host_system_cartography_backend",
                    })),
                    Err(error) => ToolOutcome::error(error),
                }
            }
            QueryAction::Changes => {
                let session = match self.state.ready() {
                    Ok(session) => session,
                    Err(error) => return ToolOutcome::error(error),
                };
                match session
                    .query_changes(args.after_change_sequence.unwrap_or_default(), args.limit)
                    .await
                {
                    Ok(page) => ToolOutcome::ok(format!(
                        "Read {} realtime cartography changes from Agent Desktop.",
                        page.changes.len()
                    ))
                    .with_details(json!({
                        "organization_id": page.organization_id,
                        "workspace_id": page.workspace_id,
                        "changes": page.changes,
                        "next_after_sequence": page.next_after_sequence,
                        "authority": "host_system_cartography_backend",
                    })),
                    Err(error) => ToolOutcome::error(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
