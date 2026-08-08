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
        "Operate Scout's append-only, replayable evidence ledger. Call scout_capabilities first; start rejects unknown census ids. Phases advance serially: charter -> map -> measure -> check -> prove -> adjudicate -> synthesize -> sealed. Only the root issues assignments, advances phases, and adjudicates. Submit worker envelopes (which register claims and evidence) only during the phase matching the worker's role, and always BEFORE advancing past it: mapper submits during map, measurer during measure, reproducer during check, prover/red_team/reproducer during prove. Envelopes submitted at the wrong phase are rejected, and claims cannot be registered after the phase leaves the worker's role. Worker artifacts remain untrusted until a host probe verifies them. Use status to inspect the current snapshot and seal to emit the final report."
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
                    "description": "Typed operation payload (all snake_case). Required fields per action:\nstart: census_id, objective, snapshot_id, scopes[], exclusions[], production_read_only, network_allowed, denied[], minimum_power.\nissue_assignment: id, role(mapper|measurer|prover|red_team|reproducer), objective, snapshot_id, scopes[]; a role may only be issued during the phase it submits in (mapper=map, measurer=measure, reproducer=check, prover/red_team/reproducer=prove).\nsubmit_worker: a worker envelope with REQUIRED assignment_id, role, snapshot_id, coverage, plus optional artifacts[], claims[], claim_updates[], limitations[], requested_followups[]. coverage is the required top-level string naming how much of the assignment scope this envelope covers. Each claims entry is {id, text, headline, quantitative, required_tier?, evidence[], counterevidence[], assumptions[], missing_instrument?}. Each artifacts entry is {id, kind(source_trace|live_state|census|measurement|offline_poc|benign_reachability|reproduction|counterexample|assumption), source, content_sha256, observed_at_ms, snapshot_id, scope, recipe?, proof_tier(t1_source|t2_live_state|t3_offline_poc|t4_benign_reachability)?, measurement?, reproduces?}.\nadvance: to(charter|map|measure|check|prove|adjudicate|synthesize|sealed).\nadjudicate: claim_id, verdict(supported|unsupported|unfalsifiable), test, reason, proof_tier?, addressed_counterevidence[], instrument_needed?.\nretract: claim_id, reason.\nsupersede: claim_id, replacement<a claim proposal>, assignment_id, reason.\nseal: disposition(complete|partial).\nOmit data for status."
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
            ledger.submit(envelope).map_err(|error| {
                format!(
                    "{error}; ledger phase is `{:?}` and worker envelopes are only accepted while the envelope role is allowed in the current phase (mapper=map, measurer=measure, reproducer=check, prover/red_team/reproducer=prove); register all envelopes before advancing into adjudicate",
                    ledger.snapshot().phase
                )
            })?;
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
    serde_json::from_value(data.ok_or_else(|| format!("{action} requires data"))?).map_err(
        |error| {
            let hint = expected_payload_shape(action);
            if hint.is_empty() {
                format!("invalid {action} data: {error}")
            } else {
                format!("invalid {action} data: {error}; {hint}")
            }
        },
    )
}

/// Model recovery hint appended to parse failures so the caller can fix the
/// payload instead of guessing the schema again.
fn expected_payload_shape(action: &str) -> &'static str {
    match action {
        "start" => "expected fields: census_id, objective, snapshot_id, scopes[], exclusions[], production_read_only, network_allowed, denied[], minimum_power",
        "issue_assignment" => "expected fields: id, role(mapper|measurer|prover|red_team|reproducer), objective, snapshot_id, scopes[]",
        "submit_worker" => "expected envelope fields: assignment_id, role, snapshot_id, artifacts[], claims[], claim_updates[], coverage, limitations[], requested_followups[]",
        "advance" => "expected fields: to(charter|map|measure|check|prove|adjudicate|synthesize|sealed)",
        "adjudicate" => "expected fields: claim_id, verdict(supported|unsupported|unfalsifiable), test, reason, proof_tier?, addressed_counterevidence[], instrument_needed?",
        "retract" => "expected fields: claim_id, reason",
        "supersede" => "expected fields: claim_id, replacement<a claim proposal>, assignment_id, reason",
        "seal" => "expected fields: disposition(complete|partial)",
        _ => "",
    }
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
    use crate::background::BackgroundTasks;
    use crate::exec::LocalExecutor;
    use crate::loop_state::SessionState;
    use crate::orchestration::tool::scout::capabilities::{CapabilityReport, CensusTruncation};
    use crate::sandbox::Sandbox;
    use crate::tools::ReadTracker;
    use std::collections::BTreeMap;
    use std::sync::Mutex as StdMutex;
    use tokio_util::sync::CancellationToken;

    fn ctx() -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(std::env::temp_dir()).unwrap()),
            executor: Arc::new(LocalExecutor),
            reads: Arc::new(StdMutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
            background: Arc::new(BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
            progress: None,
            agent_progress: None,
            call_progress: None,
            model_override: None,
        }
    }

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
            adapter_gate: tokio::sync::Mutex::new(()),
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

    #[test]
    fn submit_worker_parse_errors_name_the_expected_envelope_fields() {
        // A worker envelope missing the required top-level `coverage` must be
        // rejected with a recovery hint, not a bare serde message the model
        // has to guess against.
        let error = decode_data::<ScoutWorkerEnvelope>(
            Some(json!({
                "assignment_id": "assignment-1",
                "role": "mapper",
                "snapshot_id": "snapshot-1"
            })),
            "submit_worker",
        )
        .unwrap_err();
        assert!(error.contains("missing field `coverage`"), "{error}");
        assert!(
            error.contains("expected envelope fields: assignment_id, role, snapshot_id"),
            "{error}"
        );
        assert!(error.contains("coverage"), "{error}");
    }

    #[test]
    fn submit_worker_accepts_a_complete_envelope() {
        let envelope = decode_data::<ScoutWorkerEnvelope>(
            Some(json!({
                "assignment_id": "assignment-1",
                "role": "mapper",
                "snapshot_id": "snapshot-1",
                "coverage": "fully mapped src/services",
                "claims": [{"id": "claim-1", "text": "fixture claim", "headline": true}],
                "limitations": ["n/a"]
            })),
            "submit_worker",
        )
        .unwrap();
        assert_eq!(envelope.assignment_id.as_str(), "assignment-1");
        assert_eq!(envelope.coverage, "fully mapped src/services");
        assert_eq!(envelope.claims.len(), 1);
    }

    #[test]
    fn ledger_tool_contract_documents_required_envelope_fields_and_phase_gating() {
        let tool = ScoutLedgerTool { state: state() };
        let parameters = tool.parameters();
        let data_description = parameters["properties"]["data"]["description"]
            .as_str()
            .unwrap();
        assert!(data_description.contains("REQUIRED assignment_id, role, snapshot_id, coverage"));
        assert!(data_description.contains("coverage is the required top-level string"));
        assert!(data_description.contains("|measure|"));
        assert!(data_description.contains("adjudicate"));
        assert!(
            data_description.contains("a role may only be issued during the phase it submits in")
        );
        assert!(tool.description().contains("charter -> map -> measure"));
        assert!(tool.description().contains("mapper submits during map"));
        assert!(tool.description().contains("BEFORE advancing"));
    }

    #[tokio::test]
    async fn submit_during_a_mismatched_phase_names_the_current_phase() {
        // A measurer cannot submit once the ledger has advanced; the surfaced
        // error must state the current phase and the allowed window.
        let state = state();
        let tool = ScoutLedgerTool {
            state: state.clone(),
        };
        assert!(
            !tool
                .start(
                    "run-1".into(),
                    Some(json!({
                        "census_id": "census-test",
                        "objective": "map fixture",
                        "snapshot_id": "snapshot-1",
                        "scopes": ["repo"],
                        "production_read_only": true,
                        "network_allowed": false
                    })),
                )
                .is_error
        );
        // A role may only be issued during the phase it submits in: advance to
        // map, issue a mapper, submit it, then measure (issue a measurer), and
        // advance into adjudicate so the late measurer submission is out of phase.
        let outcome = tool
            .invoke(
                json!({ "action": "advance", "run_id": "run-1", "data": { "to": "map" } }),
                &ctx(),
            )
            .await;
        assert!(!outcome.is_error, "{}", outcome.content);
        let outcome = tool
            .invoke(
                json!({
                    "action": "issue_assignment",
                    "run_id": "run-1",
                    "data": {
                        "id": "assignment-map",
                        "role": "mapper",
                        "objective": "map the fixture",
                        "snapshot_id": "snapshot-1",
                        "scopes": ["repo"]
                    }
                }),
                &ctx(),
            )
            .await;
        assert!(!outcome.is_error, "{}", outcome.content);
        let outcome = tool
            .invoke(
                json!({
                    "action": "submit_worker",
                    "run_id": "run-1",
                    "data": {
                        "assignment_id": "assignment-map",
                        "role": "mapper",
                        "snapshot_id": "snapshot-1",
                        "coverage": "mapped everything"
                    }
                }),
                &ctx(),
            )
            .await;
        assert!(!outcome.is_error, "{}", outcome.content);
        let outcome = tool
            .invoke(
                json!({ "action": "advance", "run_id": "run-1", "data": { "to": "measure" } }),
                &ctx(),
            )
            .await;
        assert!(!outcome.is_error, "{}", outcome.content);
        let outcome = tool
            .invoke(
                json!({
                    "action": "issue_assignment",
                    "run_id": "run-1",
                    "data": {
                        "id": "assignment-measure",
                        "role": "measurer",
                        "objective": "measure the fixture",
                        "snapshot_id": "snapshot-1",
                        "scopes": ["repo"]
                    }
                }),
                &ctx(),
            )
            .await;
        assert!(!outcome.is_error, "{}", outcome.content);
        for to in ["check", "prove", "adjudicate"] {
            let outcome = tool
                .invoke(
                    json!({ "action": "advance", "run_id": "run-1", "data": { "to": to } }),
                    &ctx(),
                )
                .await;
            assert!(!outcome.is_error, "{}", outcome.content);
        }
        let outcome = tool
            .invoke(
                json!({
                    "action": "submit_worker",
                    "run_id": "run-1",
                    "data": {
                        "assignment_id": "assignment-measure",
                        "role": "measurer",
                        "snapshot_id": "snapshot-1",
                        "coverage": "measured everything"
                    }
                }),
                &ctx(),
            )
            .await;
        assert!(outcome.is_error);
        assert!(
            outcome.content.contains("ledger phase is `Adjudicate`"),
            "{}",
            outcome.content
        );
        assert!(outcome
            .content
            .contains("register all envelopes before advancing"));
    }
}
