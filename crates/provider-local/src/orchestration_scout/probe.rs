use std::sync::Arc;

use agent_core::domain::ToolKind;
use agent_orchestration::{
    ScoutEvidenceArtifact, ScoutEvidenceCheck, ScoutEvidenceId, ScoutEvidenceKind,
    VerificationOutcome,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ScoutToolState;
use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};

const MAX_SOURCE_LINES: u64 = 400;

#[path = "probe_runner.rs"]
mod runner;
use runner::{
    execute_recipe, now_ms, receipt_digest, runner_id, source_label, validate_path_scope,
};
#[cfg(test)]
use runner::{project_relative_path, redact_source_line};
pub(super) use runner::{resolve_probe_path, sensitive_path};

pub(super) struct ScoutProbeTool {
    pub state: Arc<ScoutToolState>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProbeAction {
    Record,
    Verify,
    Reproduce,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProbeOperation {
    SourceSlice,
    TextCount,
    JsonArrayCount,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeArgs {
    action: ProbeAction,
    run_id: String,
    evidence_id: String,
    #[serde(default)]
    target_evidence_id: Option<String>,
    #[serde(default)]
    operation: Option<ProbeOperation>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    line_start: Option<u64>,
    #[serde(default)]
    line_end: Option<u64>,
    #[serde(default)]
    needle: Option<String>,
    #[serde(default)]
    json_pointer: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ProbeRecipe {
    schema_version: String,
    operation: ProbeOperation,
    path: String,
    scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line_start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line_end: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    needle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    json_pointer: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ProbeReceipt {
    schema_version: &'static str,
    operation: ProbeOperation,
    path: String,
    input_sha256: String,
    selector: Value,
    result: Value,
}

#[async_trait]
impl ToolExecutor for ScoutProbeTool {
    fn name(&self) -> &str {
        "scout_probe"
    }

    fn description(&self) -> &str {
        "Run a bounded, read-only Rust evidence probe without shell, network, or writes. record creates host-owned source/census evidence; verify replays a Scout-owned recipe and checks worker evidence; reproduce independently reruns an existing recipe. Secret-bearing paths such as .env, credentials, private keys, and secret files are refused—use scout_capabilities for names-only discovery."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["record", "verify", "reproduce"],
                    "description": "Choose the trust transition first."
                },
                "run_id": {"type": "string"},
                "evidence_id": {
                    "type": "string",
                    "description": "New id for record/reproduce; existing id for verify."
                },
                "target_evidence_id": {
                    "type": "string",
                    "description": "Existing target id, required only for reproduce."
                },
                "operation": {
                    "type": "string",
                    "enum": ["source_slice", "text_count", "json_array_count"],
                    "description": "Required only for record."
                },
                "path": {
                    "type": "string",
                    "description": "Project-relative file path, required only for record."
                },
                "scope": {
                    "type": "string",
                    "description": "Exact charter scope containing path, required only for record."
                },
                "line_start": {"type": "integer", "minimum": 1},
                "line_end": {"type": "integer", "minimum": 1},
                "needle": {"type": "string"},
                "json_pointer": {"type": "string"}
            },
            "required": ["action", "run_id", "evidence_id"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ProbeArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::error(format!("invalid Scout probe: {error}")),
        };
        let evidence_id = match ScoutEvidenceId::new(&args.evidence_id) {
            Ok(id) => id,
            Err(error) => return ToolOutcome::error(error),
        };
        match args.action {
            ProbeAction::Record => self.record(args, evidence_id, ctx).await,
            ProbeAction::Verify => self.verify(&args.run_id, evidence_id, ctx).await,
            ProbeAction::Reproduce => self.reproduce(args, evidence_id, ctx).await,
        }
    }
}

impl ScoutProbeTool {
    async fn record(
        &self,
        args: ProbeArgs,
        evidence_id: ScoutEvidenceId,
        ctx: &ToolCtx,
    ) -> ToolOutcome {
        let recipe = match recipe_from_args(&args) {
            Ok(recipe) => recipe,
            Err(error) => return ToolOutcome::error(error),
        };
        let snapshot_id = match self.validate_recipe(&args.run_id, &recipe, ctx) {
            Ok(snapshot_id) => snapshot_id,
            Err(error) => return ToolOutcome::error(error),
        };
        let receipt = match execute_recipe(&recipe, ctx).await {
            Ok(receipt) => receipt,
            Err(error) => return ToolOutcome::error(error),
        };
        let digest = receipt_digest(&receipt);
        let kind = evidence_kind_for_operation(recipe.operation);
        let artifact = ScoutEvidenceArtifact {
            id: evidence_id,
            kind,
            source: source_label(&recipe),
            content_sha256: digest,
            observed_at_ms: now_ms(),
            snapshot_id,
            scope: recipe.scope.clone(),
            recipe: Some(serde_json::to_string(&recipe).unwrap_or_default()),
            proof_tier: Some(agent_orchestration::ProofTier::T1Source),
            measurement: None,
            offline_poc_controls: None,
            reproduces: None,
        };
        let runner = runner_id("probe");
        let mut ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let Some(ledger) = ledgers.get_mut(&args.run_id) else {
            return ToolOutcome::error(format!("unknown Scout run {}", args.run_id));
        };
        if let Err(error) = ledger.record_evidence(artifact, runner) {
            return ToolOutcome::error(error);
        }
        ToolOutcome::ok(format!(
            "Host-owned evidence `{}` recorded with a replayable Rust recipe.",
            args.evidence_id
        ))
        .with_details(json!({"receipt": receipt, "content_sha256": receipt_digest(&receipt)}))
    }

    async fn verify(
        &self,
        run_id: &str,
        evidence_id: ScoutEvidenceId,
        ctx: &ToolCtx,
    ) -> ToolOutcome {
        let recipe = match self.recipe_for(run_id, &evidence_id) {
            Ok(recipe) => recipe,
            Err(error) => return ToolOutcome::error(error),
        };
        if let Err(error) = self.validate_recipe(run_id, &recipe, ctx) {
            return ToolOutcome::error(error);
        }
        let receipt = match execute_recipe(&recipe, ctx).await {
            Ok(receipt) => receipt,
            Err(error) => {
                return self.record_failed_check(run_id, evidence_id, error, "probe replay failed")
            }
        };
        let digest = receipt_digest(&receipt);
        let expected = match self.expected_digest(run_id, &evidence_id) {
            Ok(expected) => expected,
            Err(error) => return ToolOutcome::error(error),
        };
        let outcome = if digest == expected {
            VerificationOutcome::Exact
        } else {
            VerificationOutcome::Changed
        };
        let check = ScoutEvidenceCheck {
            evidence_id,
            verifier: runner_id("verifier"),
            outcome,
            observed_sha256: Some(digest.clone()),
            checked_at_ms: now_ms(),
            recipe: serde_json::to_string(&recipe).unwrap_or_default(),
            reason: if outcome == VerificationOutcome::Exact {
                "fresh host replay matched the candidate receipt"
            } else {
                "fresh host replay differed from the candidate receipt"
            }
            .into(),
        };
        let mut ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let ledger = ledgers
            .get_mut(run_id)
            .expect("Scout ledger was present before probe replay");
        if let Err(error) = ledger.check_evidence(check) {
            return ToolOutcome::error(error);
        }
        ToolOutcome::ok(format!(
            "Evidence replay outcome: `{outcome:?}`. Changed evidence is not trusted."
        ))
        .with_details(json!({"outcome": outcome, "observed_sha256": digest, "receipt": receipt}))
    }

    async fn reproduce(
        &self,
        args: ProbeArgs,
        evidence_id: ScoutEvidenceId,
        ctx: &ToolCtx,
    ) -> ToolOutcome {
        let Some(target) = args.target_evidence_id.as_deref() else {
            return ToolOutcome::error("reproduce requires target_evidence_id");
        };
        let target_id = match ScoutEvidenceId::new(target) {
            Ok(id) => id,
            Err(error) => return ToolOutcome::error(error),
        };
        let recipe = match self.recipe_for(&args.run_id, &target_id) {
            Ok(recipe) => recipe,
            Err(error) => return ToolOutcome::error(error),
        };
        let snapshot_id = match self.validate_recipe(&args.run_id, &recipe, ctx) {
            Ok(snapshot_id) => snapshot_id,
            Err(error) => return ToolOutcome::error(error),
        };
        let receipt = match execute_recipe(&recipe, ctx).await {
            Ok(receipt) => receipt,
            Err(error) => return ToolOutcome::error(error),
        };
        let digest = receipt_digest(&receipt);
        let artifact = ScoutEvidenceArtifact {
            id: evidence_id.clone(),
            kind: ScoutEvidenceKind::Reproduction,
            source: source_label(&recipe),
            content_sha256: digest.clone(),
            observed_at_ms: now_ms(),
            snapshot_id,
            scope: recipe.scope.clone(),
            recipe: Some(serde_json::to_string(&recipe).unwrap_or_default()),
            proof_tier: None,
            measurement: None,
            offline_poc_controls: None,
            reproduces: Some(target_id),
        };
        let mut ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let Some(ledger) = ledgers.get_mut(&args.run_id) else {
            return ToolOutcome::error(format!("unknown Scout run {}", args.run_id));
        };
        if let Err(error) = ledger.record_evidence(artifact, runner_id("reproducer")) {
            return ToolOutcome::error(error);
        }
        let check = ScoutEvidenceCheck {
            evidence_id,
            verifier: runner_id("reproduction-verifier"),
            outcome: VerificationOutcome::Exact,
            observed_sha256: Some(digest.clone()),
            checked_at_ms: now_ms(),
            recipe: serde_json::to_string(&recipe).unwrap_or_default(),
            reason: "independent fresh replay produced the recorded reproduction receipt".into(),
        };
        if let Err(error) = ledger.check_evidence(check) {
            return ToolOutcome::error(error);
        }
        ToolOutcome::ok(format!(
            "Independent reproduction `{}` recorded for `{target}`.",
            args.evidence_id
        ))
        .with_details(json!({"receipt": receipt, "content_sha256": digest}))
    }

    fn recipe_for(
        &self,
        run_id: &str,
        evidence_id: &ScoutEvidenceId,
    ) -> Result<ProbeRecipe, String> {
        let ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let ledger = ledgers
            .get(run_id)
            .ok_or_else(|| format!("unknown Scout run {run_id}"))?;
        let record = ledger
            .snapshot()
            .evidence
            .get(evidence_id)
            .ok_or_else(|| format!("unknown evidence {evidence_id}"))?;
        let recipe = record
            .artifact
            .recipe
            .as_deref()
            .ok_or_else(|| "evidence has no replay recipe".to_string())?;
        let recipe: ProbeRecipe = serde_json::from_str(recipe)
            .map_err(|_| "evidence recipe is not owned by scout_probe".to_string())?;
        if !probe_can_verify_kind(record.artifact.kind, recipe.operation) {
            return Err(format!(
                "scout_probe cannot verify {:?} evidence; use a host runner for that evidence kind",
                record.artifact.kind
            ));
        }
        Ok(recipe)
    }

    fn expected_digest(
        &self,
        run_id: &str,
        evidence_id: &ScoutEvidenceId,
    ) -> Result<String, String> {
        let ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let ledger = ledgers
            .get(run_id)
            .ok_or_else(|| format!("unknown Scout run {run_id}"))?;
        ledger
            .snapshot()
            .evidence
            .get(evidence_id)
            .map(|record| record.artifact.content_sha256.clone())
            .ok_or_else(|| format!("unknown evidence {evidence_id}"))
    }

    fn validate_recipe(
        &self,
        run_id: &str,
        recipe: &ProbeRecipe,
        ctx: &ToolCtx,
    ) -> Result<String, String> {
        if recipe.schema_version != "scout-probe-v1" {
            return Err("unsupported Scout probe recipe version".into());
        }
        let path = resolve_probe_path(ctx, &recipe.path)?;
        if sensitive_path(&path) {
            return Err(
                "scout_probe refuses secret-bearing paths; use scout_capabilities for names-only discovery"
                    .into(),
            );
        }
        let ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let ledger = ledgers
            .get(run_id)
            .ok_or_else(|| format!("unknown Scout run {run_id}"))?;
        if !ledger.snapshot().charter.scopes.contains(&recipe.scope) {
            return Err(format!("undeclared Scout scope {}", recipe.scope));
        }
        validate_path_scope(ctx, &path, &recipe.scope)?;
        Ok(ledger.snapshot().charter.snapshot_id.clone())
    }

    fn record_failed_check(
        &self,
        run_id: &str,
        evidence_id: ScoutEvidenceId,
        error: String,
        reason: &str,
    ) -> ToolOutcome {
        let check = ScoutEvidenceCheck {
            evidence_id,
            verifier: runner_id("verifier"),
            outcome: VerificationOutcome::Failed,
            observed_sha256: None,
            checked_at_ms: now_ms(),
            recipe: "scout-probe-v1 replay".into(),
            reason: format!("{reason}: {error}"),
        };
        let mut ledgers = self.state.ledgers.lock().expect("Scout ledger lock");
        let Some(ledger) = ledgers.get_mut(run_id) else {
            return ToolOutcome::error(format!("unknown Scout run {run_id}"));
        };
        match ledger.check_evidence(check) {
            Ok(()) => ToolOutcome::error(format!("evidence replay failed: {error}")),
            Err(check_error) => ToolOutcome::error(format!("{error}; {check_error}")),
        }
    }
}

fn recipe_from_args(args: &ProbeArgs) -> Result<ProbeRecipe, String> {
    let operation = args
        .operation
        .ok_or_else(|| "record requires operation".to_string())?;
    let path = args
        .path
        .clone()
        .ok_or_else(|| "record requires path".to_string())?;
    let scope = args
        .scope
        .clone()
        .ok_or_else(|| "record requires scope".to_string())?;
    let recipe = ProbeRecipe {
        schema_version: "scout-probe-v1".into(),
        operation,
        path,
        scope,
        line_start: args.line_start,
        line_end: args.line_end,
        needle: args.needle.clone(),
        json_pointer: args.json_pointer.clone(),
    };
    match operation {
        ProbeOperation::SourceSlice => {
            let start = recipe.line_start.unwrap_or(1);
            let end = recipe.line_end.unwrap_or(start);
            if end < start || end - start + 1 > MAX_SOURCE_LINES {
                return Err(format!(
                    "source_slice requires an ordered range of at most {MAX_SOURCE_LINES} lines"
                ));
            }
        }
        ProbeOperation::TextCount if recipe.needle.as_deref().is_none_or(str::is_empty) => {
            return Err("text_count requires a non-empty needle".into())
        }
        ProbeOperation::JsonArrayCount if recipe.json_pointer.is_none() => {
            return Err("json_array_count requires json_pointer".into())
        }
        _ => {}
    }
    Ok(recipe)
}

fn evidence_kind_for_operation(operation: ProbeOperation) -> ScoutEvidenceKind {
    match operation {
        ProbeOperation::SourceSlice => ScoutEvidenceKind::SourceTrace,
        ProbeOperation::TextCount | ProbeOperation::JsonArrayCount => ScoutEvidenceKind::Census,
    }
}

fn probe_can_verify_kind(kind: ScoutEvidenceKind, operation: ProbeOperation) -> bool {
    kind == evidence_kind_for_operation(operation) || kind == ScoutEvidenceKind::Reproduction
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
