use std::collections::BTreeSet;
use std::sync::Arc;

use agent_core::domain::{ArtifactKind, ToolKind};
use agent_orchestration::{
    ScoutAdjudication, ScoutAssignment, ScoutCapabilities, ScoutCharter, ScoutClaimId,
    ScoutClaimProposal, ScoutLedger, ScoutLimits, ScoutPhase, ScoutRunId, ScoutWorkerAssignmentId,
    ScoutWorkerEnvelope, SealDisposition,
};
use async_trait::async_trait;
use base64::Engine;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::scope::census_scope_covers;
use super::ScoutToolState;
use crate::tools::{ProducedArtifact, ToolCtx, ToolExecutor, ToolOutcome};

pub(super) struct ScoutLedgerTool {
    pub state: Arc<ScoutToolState>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LedgerArgs {
    action: LedgerAction,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LedgerAction {
    Start,
    IssueAssignment,
    SubmitWorker,
    Advance,
    Adjudicate,
    Retract,
    Supersede,
    Status,
    Seal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArgs {
    census_id: String,
    objective: String,
    snapshot_id: String,
    scopes: BTreeSet<String>,
    #[serde(default)]
    exclusions: BTreeSet<String>,
    production_read_only: bool,
    network_allowed: bool,
    #[serde(default)]
    denied: BTreeSet<String>,
    #[serde(default = "default_minimum_power")]
    minimum_power: f64,
}

fn default_minimum_power() -> f64 {
    0.8
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdvanceArgs {
    to: ScoutPhase,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetractArgs {
    claim_id: ScoutClaimId,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupersedeArgs {
    claim_id: ScoutClaimId,
    replacement: ScoutClaimProposal,
    assignment_id: ScoutWorkerAssignmentId,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SealArgs {
    disposition: SealDisposition,
}

#[async_trait]
impl ToolExecutor for ScoutLedgerTool {
    fn name(&self) -> &str {
        "scout_ledger"
    }

    fn description(&self) -> &str {
        "Operate Scout's append-only, replayable evidence ledger. Call scout_capabilities first; start rejects unknown census ids. The host issues assignments and owns phase transitions/adjudication. Worker artifacts remain untrusted until a host probe verifies them. Use status to inspect the current snapshot and seal to emit the final report."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "start",
                        "issue_assignment",
                        "submit_worker",
                        "advance",
                        "adjudicate",
                        "retract",
                        "supersede",
                        "status",
                        "seal"
                    ],
                    "description": "Commit to the ledger operation before supplying its target."
                },
                "run_id": {
                    "type": "string",
                    "description": "Existing Scout run id. For start, provide the new id."
                },
                "data": {
                    "type": "object",
                    "description": "Typed operation payload. start: census_id, objective, snapshot_id, scopes, exclusions, production_read_only, network_allowed, denied, minimum_power. issue_assignment: a ScoutAssignment. submit_worker: a WorkerEnvelope. advance: {to}. adjudicate: an Adjudication. retract: {claim_id, reason}. supersede: {claim_id, replacement, assignment_id, reason}. seal: {disposition}. Omit for status."
                }
            },
            "required": ["action", "run_id"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let args: LedgerArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => {
                return ToolOutcome::error(format!("invalid Scout ledger request: {error}"))
            }
        };
        let Some(run_id) = args.run_id.filter(|run_id| !run_id.trim().is_empty()) else {
            return ToolOutcome::error("Scout ledger requests require run_id");
        };
        match args.action {
            LedgerAction::Start => self.start(run_id, args.data),
            action => self.with_ledger(&run_id, |ledger| apply_existing(action, args.data, ledger)),
        }
    }
}

impl ScoutLedgerTool {
    fn start(&self, run_id: String, data: Option<Value>) -> ToolOutcome {
        let data: StartArgs = match decode_data(data, "start") {
            Ok(data) => data,
            Err(error) => return ToolOutcome::error(error),
        };
        let census = {
            let censuses = self.state.censuses.lock().expect("Scout census lock");
            match censuses.get(&data.census_id) {
                Some(census) => census.clone(),
                None => {
                    return ToolOutcome::error(
                        "unknown capability census id; call scout_capabilities immediately before starting Scout",
                    )
                }
            }
        };
        let run_id = match ScoutRunId::new(&run_id) {
            Ok(run_id) => run_id,
            Err(error) => return ToolOutcome::error(error),
        };
        if let Some(scope) = data
            .scopes
            .iter()
            .find(|scope| !census_scope_covers(&census.scope, scope))
        {
            return ToolOutcome::error(format!(
                "Scout scope `{scope}` was not covered by capability census scope `{}`",
                census.scope
            ));
        }
        let limits = ScoutLimits {
            max_parallel_agents: self.state.max_parallel_agents,
            ..ScoutLimits::default()
        };
        let charter = ScoutCharter {
            run_id: run_id.clone(),
            objective: data.objective,
            snapshot_id: data.snapshot_id,
            capability_census_id: census.id,
            capability_fingerprint: census.fingerprint,
            scopes: data.scopes,
            exclusions: data.exclusions,
            capabilities: ScoutCapabilities {
                production_read_only: data.production_read_only,
                network_allowed: data.network_allowed,
                denied: data.denied,
            },
            limits,
            minimum_power: data.minimum_power,
        };
        let ledger = match ScoutLedger::new(charter) {
            Ok(ledger) => ledger,
            Err(error) => return ToolOutcome::error(error),
        };
        let details = ledger_details(&ledger, false);
        let mut ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        if ledgers.contains_key(run_id.as_str()) {
            return ToolOutcome::error(format!("Scout run {run_id} already exists"));
        }
        ledgers.insert(run_id.to_string(), ledger);
        ToolOutcome::ok(format!(
            "Scout run `{run_id}` started from census `{}`. Advance to map, issue bounded assignments, and submit worker envelopes.",
            data.census_id
        ))
        .with_details(details)
    }

    fn with_ledger(
        &self,
        run_id: &str,
        operation: impl FnOnce(&mut ScoutLedger) -> Result<LedgerResponse, String>,
    ) -> ToolOutcome {
        let mut ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let Some(ledger) = ledgers.get_mut(run_id) else {
            return ToolOutcome::error(format!("unknown Scout run {run_id}"));
        };
        let response = match operation(ledger) {
            Ok(response) => response,
            Err(error) => return ToolOutcome::error(error),
        };
        let details = ledger_details(ledger, response.full_details);
        let mut outcome = ToolOutcome::ok(response.message).with_details(details);
        if response.emit_report {
            let markdown = ledger.report_markdown();
            let encoded = base64::engine::general_purpose::STANDARD.encode(markdown.as_bytes());
            outcome = outcome.with_artifact(ProducedArtifact {
                id: format!("scout-report-{run_id}"),
                title: format!("Scout report: {run_id}"),
                kind: ArtifactKind::File,
                mime_type: Some("text/markdown".into()),
                uri: Some(format!("data:text/markdown;base64,{encoded}")),
            });
        }
        outcome
    }
}

struct LedgerResponse {
    message: String,
    full_details: bool,
    emit_report: bool,
}

fn apply_existing(
    action: LedgerAction,
    data: Option<Value>,
    ledger: &mut ScoutLedger,
) -> Result<LedgerResponse, String> {
    let message = match action {
        LedgerAction::IssueAssignment => {
            let assignment: ScoutAssignment = decode_data(data, "issue_assignment")?;
            let id = assignment.id.to_string();
            ledger.issue_assignment(assignment)?;
            format!("Scout assignment `{id}` issued.")
        }
        LedgerAction::SubmitWorker => {
            let envelope: ScoutWorkerEnvelope = decode_data(data, "submit_worker")?;
            let id = envelope.assignment_id.to_string();
            ledger.submit(envelope)?;
            format!(
                "Worker envelope `{id}` appended. Candidate evidence is not trusted until host verification."
            )
        }
        LedgerAction::Advance => {
            let request: AdvanceArgs = decode_data(data, "advance")?;
            ledger.advance(request.to)?;
            format!("Scout advanced to `{:?}`.", request.to)
        }
        LedgerAction::Adjudicate => {
            let decision: ScoutAdjudication = decode_data(data, "adjudicate")?;
            let claim_id = decision.claim_id.to_string();
            ledger.adjudicate(decision)?;
            format!("Claim `{claim_id}` adjudicated.")
        }
        LedgerAction::Retract => {
            let request: RetractArgs = decode_data(data, "retract")?;
            let claim_id = request.claim_id.to_string();
            ledger.retract(request.claim_id, request.reason)?;
            format!("Claim `{claim_id}` retracted with an append-only reason.")
        }
        LedgerAction::Supersede => {
            let request: SupersedeArgs = decode_data(data, "supersede")?;
            let claim_id = request.claim_id.to_string();
            ledger.supersede(
                request.claim_id,
                request.replacement,
                request.assignment_id,
                request.reason,
            )?;
            format!("Claim `{claim_id}` superseded with an append-only correction.")
        }
        LedgerAction::Status => {
            if data.is_some() {
                return Err("status does not accept data".into());
            }
            return Ok(LedgerResponse {
                message: format!(
                    "Scout run is in `{:?}` with {} claims and {} evidence artifacts.",
                    ledger.snapshot().phase,
                    ledger.snapshot().claims.len(),
                    ledger.snapshot().evidence.len()
                ),
                full_details: true,
                emit_report: false,
            });
        }
        LedgerAction::Seal => {
            let request: SealArgs = decode_data(data, "seal")?;
            ledger.seal(request.disposition)?;
            return Ok(LedgerResponse {
                message: format!(
                    "Scout run sealed as `{:?}`; report and ledger receipts are attached.",
                    request.disposition
                ),
                full_details: true,
                emit_report: true,
            });
        }
        LedgerAction::Start => return Err("start requires a new Scout run".into()),
    };
    Ok(LedgerResponse {
        message,
        full_details: false,
        emit_report: false,
    })
}

fn decode_data<T: DeserializeOwned>(data: Option<Value>, action: &str) -> Result<T, String> {
    serde_json::from_value(data.ok_or_else(|| format!("{action} requires data"))?)
        .map_err(|error| format!("invalid {action} data: {error}"))
}

fn ledger_details(ledger: &ScoutLedger, include_events: bool) -> Value {
    let fingerprint = ledger.fingerprint().unwrap_or_default();
    if include_events {
        json!({
            "snapshot": ledger.snapshot(),
            "events": ledger.events(),
            "fingerprint": fingerprint,
            "report_markdown": ledger.report_markdown(),
        })
    } else {
        json!({
            "phase": ledger.snapshot().phase,
            "event_count": ledger.snapshot().event_count,
            "fingerprint": fingerprint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::tool::scout::capabilities::{CapabilityReport, CensusTruncation};
    use std::collections::BTreeMap;

    fn state() -> Arc<ScoutToolState> {
        let census = CapabilityReport {
            id: "census-test".into(),
            schema_version: "v1".into(),
            platform: "linux".into(),
            architecture: "x86_64".into(),
            scope: ".".into(),
            adapter_executable_names: Vec::new(),
            path_executable_count: 0,
            path_executable_names_sha256: "b".repeat(64),
            environment: Vec::new(),
            environment_name_count: 0,
            environment_names_sha256: "c".repeat(64),
            dotenv_files: Vec::new(),
            credential_surfaces: Vec::new(),
            routing: BTreeMap::new(),
            fallbacks: Vec::new(),
            truncated: CensusTruncation {
                executables: false,
                environment_names: false,
                dotenv_files: false,
            },
            fingerprint: "a".repeat(64),
        };
        Arc::new(ScoutToolState {
            censuses: std::sync::Mutex::new(std::collections::HashMap::from([(
                census.id.clone(),
                census,
            )])),
            ledgers: std::sync::Mutex::new(std::collections::HashMap::new()),
            target: std::sync::Mutex::new(None),
            max_parallel_agents: 3,
        })
    }

    #[test]
    fn start_requires_a_host_recorded_capability_census() {
        let tool = ScoutLedgerTool { state: state() };
        let outcome = tool.start(
            "run-1".into(),
            Some(json!({
                "census_id": "forged",
                "objective": "map fixture",
                "snapshot_id": "snapshot-1",
                "scopes": ["repo"],
                "production_read_only": true,
                "network_allowed": false
            })),
        );
        assert!(outcome.is_error);
        assert!(outcome.content.contains("unknown capability census"));
    }

    #[test]
    fn start_pins_census_fingerprint_into_the_charter() {
        let state = state();
        let tool = ScoutLedgerTool {
            state: state.clone(),
        };
        let outcome = tool.start(
            "run-1".into(),
            Some(json!({
                "census_id": "census-test",
                "objective": "map fixture",
                "snapshot_id": "snapshot-1",
                "scopes": ["repo"],
                "production_read_only": true,
                "network_allowed": false
            })),
        );
        assert!(!outcome.is_error, "{}", outcome.content);
        let ledgers = state.ledgers.lock().unwrap();
        let charter = &ledgers["run-1"].snapshot().charter;
        assert_eq!(charter.capability_census_id, "census-test");
        assert_eq!(charter.capability_fingerprint, "a".repeat(64));
        assert_eq!(charter.limits.max_parallel_agents, 3);
    }

    #[test]
    fn start_rejects_scopes_outside_the_pinned_census() {
        let state = state();
        state
            .censuses
            .lock()
            .unwrap()
            .get_mut("census-test")
            .unwrap()
            .scope = "services/api".into();
        let tool = ScoutLedgerTool { state };
        let outcome = tool.start(
            "run-1".into(),
            Some(json!({
                "census_id": "census-test",
                "objective": "map fixture",
                "snapshot_id": "snapshot-1",
                "scopes": ["services/web"],
                "production_read_only": true,
                "network_allowed": false
            })),
        );
        assert!(outcome.is_error);
        assert!(outcome.content.contains("not covered"));
    }
}
