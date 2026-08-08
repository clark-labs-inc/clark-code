use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome};
use crate::security::{
    collect_security_diff_inventory, collect_security_inventory, finalize_security_diff,
    finalize_security_scan, SecurityDiffKind, SecurityScanBundle, SecurityScanMode,
    SECURITY_SCAN_CONTRACT_VERSION,
};

const MAX_BUNDLE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_PAGE_SIZE: usize = 200;
const MAX_PAGE_SIZE: usize = 500;

pub struct SecurityScanContract;

#[async_trait]
impl ToolExecutor for SecurityScanContract {
    fn name(&self) -> &str {
        "security_scan_contract"
    }

    fn description(&self) -> &str {
        "Deterministic Security scanner workbench. Use `schema` to get the canonical scan \
         shape, `inventory` to page through the exact target file set and obtain its \
         snapshot id, `diff_inventory` to bind changed files to an exact Git range or \
         working-tree object, `deep_begin`/`deep_checkpoint` to bind accepted \
         independent passes and saturation, and `finalize` to reject incomplete \
         coverage, stale targets, unvalidated candidates, or reportable findings \
         without attack-path evidence and host-issued positive/negative PoC receipts."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["schema", "inventory", "diff_inventory", "deep_begin", "deep_status", "deep_checkpoint", "finalize"],
                    "description": "Choose the contract operation before locating its target."
                },
                "scope": {
                    "type": "string",
                    "description": "Repository-relative directory to inventory. Defaults to the project root."
                },
                "diff_kind": {
                    "type": "string",
                    "enum": ["working_tree", "range"],
                    "description": "Exact Git target kind for diff_inventory. Defaults to working_tree."
                },
                "base": {
                    "type": "string",
                    "description": "Git base revision for diff_inventory. Defaults to HEAD."
                },
                "head": {
                    "type": "string",
                    "description": "Git head revision for range diff_inventory. Omit for working_tree."
                },
                "scan_id": {
                    "type": "string",
                    "description": "Canonical scan id used to start a deep run."
                },
                "deep_run_id": {
                    "type": "string",
                    "description": "Host-issued deep run id used for status and pass checkpoints."
                },
                "orchestration_id": {
                    "type": "string",
                    "description": "Accepted delegate_read_only orchestration receipt to checkpoint."
                },
                "candidate_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "uniqueItems": true,
                    "description": "Canonical candidate ids observed in this independent pass."
                },
                "cursor": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Zero-based inventory page cursor."
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_PAGE_SIZE,
                    "description": "Inventory paths per page. Defaults to 200."
                },
                "path": {
                    "type": "string",
                    "description": "Repository-relative canonical scan JSON path for finalize."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let action = match arg_str(&args, "action") {
            Ok(action) => action,
            Err(error) => return ToolOutcome::error(error),
        };
        match action.as_str() {
            "schema" => {
                let model = ctx
                    .model_override
                    .as_ref()
                    .map(|policy| policy.model.as_str())
                    .unwrap_or("conversation-model");
                ToolOutcome::ok(schema(model).to_string()).with_details(schema(model))
            }
            "inventory" => inventory(args, ctx).await,
            "diff_inventory" => diff_inventory(args, ctx).await,
            "deep_begin" => deep_begin(args, ctx).await,
            "deep_status" => deep_status(ctx).await,
            "deep_checkpoint" => deep_checkpoint(args, ctx).await,
            "finalize" => finalize(args, ctx).await,
            _ => ToolOutcome::error(
                "action must be `schema`, `inventory`, `diff_inventory`, `deep_begin`, \
                 `deep_status`, `deep_checkpoint`, or `finalize`",
            ),
        }
    }
}

async fn inventory(args: Value, ctx: &ToolCtx) -> ToolOutcome {
    let scope = arg_str_opt(&args, "scope").unwrap_or_else(|| ".".into());
    let resolved = match ctx.sandbox.resolve_existing(&scope) {
        Ok(path) => path,
        Err(error) => return ToolOutcome::error(error),
    };
    let inventory = match collect_security_inventory(
        ctx.executor.as_ref(),
        ctx.sandbox.root(),
        &resolved,
    )
    .await
    {
        Ok(inventory) => inventory,
        Err(error) => return ToolOutcome::error(error),
    };
    let cursor = args
        .get("cursor")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    let page_size = args
        .get("page_size")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_PAGE_SIZE as u64) as usize;
    if page_size == 0 || page_size > MAX_PAGE_SIZE {
        return ToolOutcome::error(format!("page_size must be between 1 and {MAX_PAGE_SIZE}"));
    }
    if cursor > inventory.paths.len() {
        return ToolOutcome::error(format!(
            "cursor {cursor} is past inventory length {}",
            inventory.paths.len()
        ));
    }
    let end = cursor.saturating_add(page_size).min(inventory.paths.len());
    let next_cursor = (end < inventory.paths.len()).then_some(end);
    let result = json!({
        "contractVersion": inventory.contract_version,
        "scope": inventory.scope,
        "inventoryId": inventory.inventory_id,
        "totalFiles": inventory.paths.len(),
        "cursor": cursor,
        "paths": &inventory.paths[cursor..end],
        "nextCursor": next_cursor,
    });
    ToolOutcome::ok(result.to_string()).with_details(result)
}

async fn diff_inventory(args: Value, ctx: &ToolCtx) -> ToolOutcome {
    let scope = arg_str_opt(&args, "scope").unwrap_or_else(|| ".".into());
    let resolved = match ctx.sandbox.resolve_existing(&scope) {
        Ok(path) => path,
        Err(error) => return ToolOutcome::error(error),
    };
    let kind = match arg_str_opt(&args, "diff_kind").as_deref() {
        None | Some("working_tree") => SecurityDiffKind::WorkingTree,
        Some("range") => SecurityDiffKind::Range,
        Some(_) => return ToolOutcome::error("diff_kind must be `working_tree` or `range`"),
    };
    let base = arg_str_opt(&args, "base").unwrap_or_else(|| "HEAD".into());
    let head = arg_str_opt(&args, "head");
    let repository_inventory = match collect_security_inventory(
        ctx.executor.as_ref(),
        ctx.sandbox.root(),
        &resolved,
    )
    .await
    {
        Ok(inventory) => inventory,
        Err(error) => return ToolOutcome::error(error),
    };
    let diff = match collect_security_diff_inventory(
        ctx.executor.as_ref(),
        ctx.sandbox.root(),
        &resolved,
        kind,
        &base,
        head.as_deref(),
    )
    .await
    {
        Ok(diff) => diff,
        Err(error) => return ToolOutcome::error(error),
    };
    let cursor = args
        .get("cursor")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    let page_size = args
        .get("page_size")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_PAGE_SIZE as u64) as usize;
    if page_size == 0 || page_size > MAX_PAGE_SIZE {
        return ToolOutcome::error(format!("page_size must be between 1 and {MAX_PAGE_SIZE}"));
    }
    if cursor > diff.changed_files.len() {
        return ToolOutcome::error(format!(
            "cursor {cursor} is past changed-file inventory length {}",
            diff.changed_files.len()
        ));
    }
    let end = cursor
        .saturating_add(page_size)
        .min(diff.changed_files.len());
    let next_cursor = (end < diff.changed_files.len()).then_some(end);
    let result = json!({
        "contractVersion": diff.contract_version,
        "scope": diff.scope,
        "inventoryId": repository_inventory.inventory_id,
        "diffTarget": diff.target,
        "resolvedBase": diff.resolved_base,
        "resolvedHead": diff.resolved_head,
        "totalChangedFiles": diff.changed_files.len(),
        "cursor": cursor,
        "changedFiles": &diff.changed_files[cursor..end],
        "nextCursor": next_cursor,
    });
    ToolOutcome::ok(result.to_string()).with_details(result)
}

async fn deep_begin(args: Value, ctx: &ToolCtx) -> ToolOutcome {
    let scan_id = match arg_str(&args, "scan_id") {
        Ok(scan_id) => scan_id,
        Err(_) => return ToolOutcome::error("scan_id is required for deep_begin"),
    };
    let scope = arg_str_opt(&args, "scope").unwrap_or_else(|| ".".into());
    let resolved = match ctx.sandbox.resolve_existing(&scope) {
        Ok(path) => path,
        Err(error) => return ToolOutcome::error(error),
    };
    let inventory = match collect_security_inventory(
        ctx.executor.as_ref(),
        ctx.sandbox.root(),
        &resolved,
    )
    .await
    {
        Ok(inventory) => inventory,
        Err(error) => return ToolOutcome::error(error),
    };
    let status = match ctx
        .session
        .lock()
        .await
        .security_deep
        .begin(&scan_id, &inventory.inventory_id)
    {
        Ok(status) => status,
        Err(error) => return ToolOutcome::error(error),
    };
    let result = json!({
        "contractVersion": SECURITY_SCAN_CONTRACT_VERSION,
        "scope": inventory.scope,
        "inventoryId": inventory.inventory_id,
        "deep": status,
    });
    ToolOutcome::ok(result.to_string()).with_details(result)
}

async fn deep_status(ctx: &ToolCtx) -> ToolOutcome {
    let status = ctx.session.lock().await.security_deep.status();
    match status {
        Some(status) => {
            let details = json!({"deep": status});
            ToolOutcome::ok(details.to_string()).with_details(details)
        }
        None => ToolOutcome::error("no deep security run is active"),
    }
}

async fn deep_checkpoint(args: Value, ctx: &ToolCtx) -> ToolOutcome {
    let run_id = match arg_str(&args, "deep_run_id") {
        Ok(run_id) => run_id,
        Err(_) => return ToolOutcome::error("deep_run_id is required for deep_checkpoint"),
    };
    let orchestration_id = match arg_str(&args, "orchestration_id") {
        Ok(orchestration_id) => orchestration_id,
        Err(_) => return ToolOutcome::error("orchestration_id is required for deep_checkpoint"),
    };
    let candidate_ids = match args.get("candidate_ids").and_then(Value::as_array) {
        Some(values) => {
            let mut ids = Vec::with_capacity(values.len());
            for value in values {
                let Some(id) = value.as_str() else {
                    return ToolOutcome::error("candidate_ids must contain only strings");
                };
                ids.push(id.to_string());
            }
            ids
        }
        None => return ToolOutcome::error("candidate_ids is required for deep_checkpoint"),
    };
    let status = match ctx.session.lock().await.security_deep.checkpoint(
        &run_id,
        &orchestration_id,
        candidate_ids,
    ) {
        Ok(status) => status,
        Err(error) => return ToolOutcome::error(error),
    };
    let details = json!({"deep": status});
    ToolOutcome::ok(details.to_string()).with_details(details)
}

async fn finalize(args: Value, ctx: &ToolCtx) -> ToolOutcome {
    let path = match arg_str(&args, "path") {
        Ok(path) => path,
        Err(_) => return ToolOutcome::error("path is required for finalize"),
    };
    let resolved = match ctx.sandbox.resolve_existing(&path) {
        Ok(path) => path,
        Err(error) => return ToolOutcome::error(error),
    };
    let scans_root = ctx.sandbox.root().join(".agent/security-scans");
    if resolved.file_name().is_none_or(|name| name != "scan.json")
        || !resolved.starts_with(&scans_root)
    {
        return ToolOutcome::error(
            "finalize path must be `.agent/security-scans/<scan-id>/scan.json`",
        );
    }
    let bytes = match ctx.executor.read(&resolved).await {
        Ok(bytes) => bytes,
        Err(error) => return ToolOutcome::error(format!("{path}: {error}")),
    };
    if bytes.len() > MAX_BUNDLE_BYTES {
        return ToolOutcome::error(format!(
            "{path} exceeds the {MAX_BUNDLE_BYTES}-byte security bundle limit"
        ));
    }
    let bundle: SecurityScanBundle = match serde_json::from_slice(&bytes) {
        Ok(bundle) => bundle,
        Err(error) => return ToolOutcome::error(format!("{path}: invalid scan JSON: {error}")),
    };
    let scope = match ctx.sandbox.resolve_existing(&bundle.scope) {
        Ok(scope) => scope,
        Err(error) => return ToolOutcome::error(error),
    };
    let inventory =
        match collect_security_inventory(ctx.executor.as_ref(), ctx.sandbox.root(), &scope).await {
            Ok(inventory) => inventory,
            Err(error) => return ToolOutcome::error(error),
        };
    let poc_ledger = ctx.session.lock().await.security_poc.clone();
    let seal = match bundle.mode {
        SecurityScanMode::Standard => finalize_security_scan(&bundle, &inventory, &poc_ledger),
        SecurityScanMode::Diff => {
            let target = match bundle.diff_target.as_ref() {
                Some(target) => target,
                None => return ToolOutcome::error("diff scan bundle is missing diffTarget"),
            };
            let diff = match collect_security_diff_inventory(
                ctx.executor.as_ref(),
                ctx.sandbox.root(),
                &scope,
                target.kind,
                &target.base,
                target.head.as_deref(),
            )
            .await
            {
                Ok(diff) => diff,
                Err(error) => return ToolOutcome::error(error),
            };
            finalize_security_diff(&bundle, &inventory, &diff, &poc_ledger)
        }
        SecurityScanMode::Deep => {
            let session = ctx.session.lock().await;
            crate::security::finalize_security_deep(
                &bundle,
                &inventory,
                &session.security_deep,
                &session.security_poc,
            )
        }
    };
    let seal = match seal {
        Ok(seal) => seal,
        Err(error) => {
            return ToolOutcome::error(format!("security scan did not finalize: {error}"))
        }
    };
    ctx.note_read(&resolved).await;
    let details = match serde_json::to_value(&seal) {
        Ok(details) => details,
        Err(error) => return ToolOutcome::error(format!("cannot encode security seal: {error}")),
    };
    let mut seal_bytes = match serde_json::to_vec_pretty(&seal) {
        Ok(bytes) => bytes,
        Err(error) => return ToolOutcome::error(format!("cannot encode security seal: {error}")),
    };
    seal_bytes.push(b'\n');
    let seal_path = resolved.with_file_name("seal.json");
    if let Err(error) = ctx.executor.write(&seal_path, &seal_bytes).await {
        return ToolOutcome::error(format!("cannot persist security seal: {error}"));
    }
    ToolOutcome::ok(details.to_string())
        .with_details(details)
        .with_location(ctx.sandbox.display(&resolved), None)
}

fn schema(model: &str) -> Value {
    json!({
        "contractVersion": SECURITY_SCAN_CONTRACT_VERSION,
        "scanId": "scan-unique-id",
        "mode": "standard",
        "model": model,
        "scope": ".",
        "inventoryId": "from security_scan_contract inventory",
        "diffTarget": null,
        "deepRunId": null,
        "phase": "reporting",
        "threatModel": {
            "assets": ["security-relevant asset"],
            "trustBoundaries": ["boundary crossed by attacker input"],
            "attackerInputs": ["attacker-controlled source"],
            "invariants": ["security property that must hold"]
        },
        "coverage": [{
            "path": "path/from/inventory",
            "status": "reviewed",
            "reason": null
        }],
        "supportingCoverage": [],
        "candidates": [{
            "candidateId": "local-ledger-id",
            "source": {"path": "src/input.rs", "line": 1, "description": "attacker-controlled value"},
            "control": {"path": "src/auth.rs", "line": 2, "description": "nearest relevant control or missing-control site"},
            "sink": {"path": "src/sink.rs", "line": 3, "description": "security-relevant operation"},
            "impact": "concrete attacker outcome",
            "validation": {
                "disposition": "reportable",
                "evidence": "source trace plus the host-executed PoC result",
                "counterevidence": ["strongest observed limiting control"]
            },
            "poc": {
                "goal": "demonstrate the vulnerable behavior and a safe negative control",
                "outcome": "reproduced",
                "positiveReceiptId": "from security_poc_execute control=positive",
                "negativeReceiptId": "from security_poc_execute control=negative",
                "limitations": []
            },
            "attackPath": {
                "attacker": "realistic in-scope attacker",
                "entrypoint": "reachable interface",
                "preconditions": ["required access"],
                "path": ["source", "control break", "sink", "impact"],
                "likelihood": "low, medium, or high with rationale",
                "severity": "low"
            }
        }]
    })
}
