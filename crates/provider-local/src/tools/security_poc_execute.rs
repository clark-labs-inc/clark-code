#[cfg(test)]
use std::path::Path;
use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use exec_core::{ExecOutput, Executor as _};
use exec_sandbox::{SandboxPolicy, SandboxedExecutor};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{PermissionScope, ToolCtx, ToolExecutor, ToolOutcome, ToolPermissionClass};
use crate::security::{
    collect_security_inventory, SecurityPocControl, SecurityPocExecutionMetadata,
    SecurityPocReceipt, SECURITY_SCAN_CONTRACT_VERSION,
};

#[path = "security_poc_execute_runner.rs"]
mod runner;
use runner::{
    complete_output, control_label, copy_inventory, digest, language_label, persist_artifacts,
    process_spec, run_bounded, script_name, MAX_OUTPUT_BYTES,
};

const MAX_SCRIPT_BYTES: usize = 256 * 1024;
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
pub(super) const MAX_TIMEOUT_SECONDS: u64 = 60;

pub struct SecurityPocExecute;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PocLanguage {
    Shell,
    Python,
    Javascript,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PocArgs {
    scan_id: String,
    candidate_id: String,
    inventory_id: String,
    #[serde(default = "default_scope")]
    scope: String,
    control: SecurityPocControl,
    language: PocLanguage,
    expected_observation: String,
    script: String,
    #[serde(default)]
    expected_exit_code: i32,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}

fn default_scope() -> String {
    ".".to_string()
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

#[async_trait]
impl ToolExecutor for SecurityPocExecute {
    fn name(&self) -> &str {
        "security_poc_execute"
    }

    fn description(&self) -> &str {
        "Execute one positive or negative security proof-of-concept control in a \
         fresh repository copy with network denied and writes confined to the \
         disposable copy. The host records output digests and issues the receipt \
         id required by security_scan_contract; model-authored receipt ids are not \
         accepted. Use a script that exits with expected_exit_code only when the \
         named observation is actually demonstrated."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "scan_id": {
                    "type": "string",
                    "description": "Canonical scan id that will reference this host-issued receipt."
                },
                "candidate_id": {
                    "type": "string",
                    "description": "Stable candidate id under test."
                },
                "inventory_id": {
                    "type": "string",
                    "description": "Exact current repository inventory id returned by security_scan_contract."
                },
                "scope": {
                    "type": "string",
                    "description": "Repository-relative scan scope. Defaults to the project root."
                },
                "control": {
                    "type": "string",
                    "enum": ["positive", "negative"],
                    "description": "Positive demonstrates the suspected vulnerable behavior; negative proves a safe input or control does not trigger it."
                },
                "language": {
                    "type": "string",
                    "enum": ["shell", "python", "javascript"],
                    "description": "Interpreter for the bounded PoC script."
                },
                "expected_observation": {
                    "type": "string",
                    "description": "Concrete behavior the script asserts before returning the expected exit code."
                },
                "script": {
                    "type": "string",
                    "description": "Self-contained offline PoC control. Networking is unavailable and source writes affect only a disposable copy."
                },
                "expected_exit_code": {
                    "type": "integer",
                    "description": "Exit code meaning the control observed its expected behavior. Defaults to 0."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TIMEOUT_SECONDS,
                    "description": "Hard wall-clock limit. Defaults to 30 seconds and cannot exceed 60."
                }
            },
            "required": [
                "scan_id",
                "candidate_id",
                "inventory_id",
                "control",
                "language",
                "expected_observation",
                "script"
            ],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::LocalMutation
    }

    fn permission_scope(&self, args: &Value) -> Option<PermissionScope> {
        let parsed = parse_args(args).ok()?;
        Some(PermissionScope {
            key: format!(
                "security-poc:{}:{}:{:?}",
                parsed.scan_id, parsed.candidate_id, parsed.control
            ),
            title: Some(format!(
                "Run isolated {:?} PoC control for {}",
                parsed.control, parsed.candidate_id
            )),
            always_label: None,
            reason: Some(
                "runs automatically because execution is offline, bounded, and confined to a fresh disposable copy"
                    .to_string(),
            ),
            risk: None,
            remember: false,
            preapproved: true,
        })
    }

    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        parse_args(args).map(|_| ())
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        let parsed = parse_args(args).ok()?;
        Some(format!(
            "Candidate: {}\nControl: {:?}\nLanguage: {:?}\nScope: {}\nTimeout: {}s\nExpected observation: {}",
            parsed.candidate_id,
            parsed.control,
            parsed.language,
            parsed.scope,
            parsed.timeout_seconds,
            parsed.expected_observation
        ))
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args = match parse_args(&args) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::error(error),
        };
        if ctx.executor.is_local() {
            self.invoke_local(args, ctx).await
        } else {
            self.invoke_remote(args, ctx).await
        }
    }
}

impl SecurityPocExecute {
    /// Local path: run the PoC in a disposable, network-denied workspace built
    /// through the (local) executor, sealed by the shared runner.
    async fn invoke_local(&self, args: PocArgs, ctx: &ToolCtx) -> ToolOutcome {
        let scope = match ctx.sandbox.resolve_existing(&args.scope) {
            Ok(scope) => scope,
            Err(error) => return ToolOutcome::error(error),
        };
        let inventory =
            match collect_security_inventory(ctx.executor.as_ref(), ctx.sandbox.root(), &scope)
                .await
            {
                Ok(inventory) => inventory,
                Err(error) => return ToolOutcome::error(error),
            };
        if inventory.inventory_id != args.inventory_id {
            return ToolOutcome::error(
                "inventory_id is stale; re-run security_scan_contract inventory before PoC execution",
            );
        }

        let execution_id = uuid::Uuid::new_v4().simple().to_string();
        let relative_run_root = format!(
            ".clark/security-scans/{}/poc/runs/{}-{}-{execution_id}",
            args.scan_id,
            args.candidate_id,
            control_label(args.control)
        );
        let run_root = match ctx.sandbox.resolve_for_write(&relative_run_root) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };
        let workspace = run_root.join("workspace");
        if let Err(error) = ctx.executor.create_dir_all(&workspace).await {
            return ToolOutcome::error(format!("cannot create disposable PoC workspace: {error}"));
        }
        let workspace_sha256 = match copy_inventory(ctx, &inventory.paths, &workspace).await {
            Ok(digest) => digest,
            Err(error) => return ToolOutcome::error(error),
        };

        let script_dir = workspace.join("__clark_poc__");
        if let Err(error) = ctx.executor.create_dir_all(&script_dir).await {
            return ToolOutcome::error(format!("cannot create PoC script directory: {error}"));
        }
        let script_path = script_dir.join(script_name(args.language));
        if let Err(error) = ctx
            .executor
            .write(&script_path, args.script.as_bytes())
            .await
        {
            return ToolOutcome::error(format!("cannot persist PoC script: {error}"));
        }
        let temp_root = script_dir.join("tmp");
        if let Err(error) = ctx.executor.create_dir_all(&temp_root).await {
            return ToolOutcome::error(format!("cannot create PoC temporary directory: {error}"));
        }

        let policy = SandboxPolicy::read_only()
            .with_write_roots([workspace.clone()])
            .with_process_temp_root(temp_root.clone());
        let poc_executor = match SandboxedExecutor::new(policy) {
            Ok(executor) => executor,
            Err(error) => {
                return ToolOutcome::error(format!(
                    "PoC refused because disposable OS containment is unavailable: {error}"
                ))
            }
        };
        let process = process_spec(args.language, &workspace, &script_path, &temp_root);
        let process = match poc_executor.prepare_process(process) {
            Ok(process) => process,
            Err(error) => {
                return ToolOutcome::error(format!("cannot sandbox PoC process: {error}"))
            }
        };
        let started_at_ms = match epoch_ms() {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        let output = match run_bounded(
            &process,
            Duration::from_secs(args.timeout_seconds),
            &ctx.cancel,
        )
        .await
        {
            Ok(output) => output,
            Err(error) => return ToolOutcome::error(format!("PoC execution failed: {error}")),
        };
        let completed_at_ms = match epoch_ms() {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };

        let artifact_path = format!("{relative_run_root}/receipt.json");
        let script_artifact_path = format!(
            "{relative_run_root}/workspace/__clark_poc__/{}",
            script_name(args.language)
        );
        let mut receipt = SecurityPocReceipt {
            contract_version: SECURITY_SCAN_CONTRACT_VERSION,
            receipt_id: String::new(),
            scan_id: args.scan_id,
            candidate_id: args.candidate_id,
            inventory_id: args.inventory_id,
            control: args.control,
            language: language_label(args.language).to_string(),
            script_sha256: digest(args.script.as_bytes()),
            expected_observation_sha256: digest(args.expected_observation.as_bytes()),
            workspace_sha256,
            stdout_sha256: digest(&output.stdout),
            stderr_sha256: digest(&output.stderr),
            expected_exit_code: args.expected_exit_code,
            exit_code: output.code,
            passed: output.code == Some(args.expected_exit_code),
            containment: "managed_disposable".to_string(),
            artifact_path: artifact_path.clone(),
            execution: Some(SecurityPocExecutionMetadata {
                expected_observation: args.expected_observation.clone(),
                started_at_ms,
                completed_at_ms,
                timeout_ms: args.timeout_seconds.saturating_mul(1_000),
                output_limit_bytes: MAX_OUTPUT_BYTES as u64,
                sandbox_provider: "clark-desktop-native".into(),
                sandbox_profile_sha256: digest(
                    format!(
                        "clark-security-native-sandbox/v1\0{}\0{}\0offline\0disposable-write-root",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    )
                    .as_bytes(),
                ),
                script_path: script_artifact_path,
                stdout_path: format!("{relative_run_root}/stdout.log"),
                stderr_path: format!("{relative_run_root}/stderr.log"),
            }),
        };
        let receipt_preimage = match serde_json::to_vec(&receipt) {
            Ok(bytes) => bytes,
            Err(error) => return ToolOutcome::error(format!("cannot encode PoC receipt: {error}")),
        };
        receipt.receipt_id = format!("poc-{}", &digest(&receipt_preimage)[..32]);

        if let Err(error) = persist_artifacts(ctx, &run_root, &receipt, &output).await {
            return ToolOutcome::error(error);
        }
        if let Err(error) = ctx
            .session
            .lock()
            .await
            .security_poc
            .record(receipt.clone())
        {
            return ToolOutcome::error(error);
        }

        let details = json!({
            "receipt": receipt,
            "expectedObservation": args.expected_observation,
            "stdout": complete_output(&output.stdout),
            "stderr": complete_output(&output.stderr),
        });
        ToolOutcome::ok(details.to_string())
            .with_details(details)
            .with_location(artifact_path, None)
    }

    /// Remote path: stage the inventory snapshot through the remote executor,
    /// then have the target-native `security-poc-v1` service run the PoC in a
    /// disposable, network-denied workspace **on the remote host** and return a
    /// `managed_disposable` receipt. The receipt is constructed by the shared
    /// `security-poc-runner` crate with the same digests and containment as the
    /// local path, so the scan contract accepts it identically.
    async fn invoke_remote(&self, args: PocArgs, ctx: &ToolCtx) -> ToolOutcome {
        use security_poc_runner::{
            PocControl as RunnerControl, PocInventoryFile, PocLanguage as RunnerLanguage,
            SecurityPocRunRequest, SecurityPocRunResponse, SERVICE_NAME,
        };

        let inventory = match resolve_inventory(&args, ctx).await {
            Ok(inventory) => inventory,
            Err(error) => return ToolOutcome::error(error),
        };

        let execution_id = uuid::Uuid::new_v4().simple().to_string();
        let relative_run_root = format!(
            ".clark/security-scans/{}/poc/runs/{}-{}-{execution_id}",
            args.scan_id,
            args.candidate_id,
            control_label(args.control)
        );
        let run_root = match ctx.sandbox.resolve_for_write(&relative_run_root) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::error(error),
        };

        // Read the inventory files through the remote executor and ship their
        // bytes; the target recomputes `workspace_sha256` over exactly these.
        let mut files = Vec::with_capacity(inventory.paths.len());
        for relative in &inventory.paths {
            let source = ctx.sandbox.root().join(relative);
            let bytes = match ctx.executor.read(&source).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    return ToolOutcome::error(format!(
                        "cannot read PoC inventory `{relative}`: {error}"
                    ))
                }
            };
            files.push(PocInventoryFile {
                path: relative.clone(),
                bytes,
            });
        }

        let control = match args.control {
            SecurityPocControl::Positive => RunnerControl::Positive,
            SecurityPocControl::Negative => RunnerControl::Negative,
        };
        let language = match args.language {
            PocLanguage::Shell => RunnerLanguage::Shell,
            PocLanguage::Python => RunnerLanguage::Python,
            PocLanguage::Javascript => RunnerLanguage::Javascript,
        };
        let request = SecurityPocRunRequest {
            scan_id: args.scan_id.clone(),
            candidate_id: args.candidate_id.clone(),
            inventory_id: args.inventory_id.clone(),
            control,
            language,
            expected_observation: args.expected_observation.clone(),
            script: args.script.clone(),
            expected_exit_code: args.expected_exit_code,
            timeout_seconds: args.timeout_seconds,
            run_root: relative_run_root.clone(),
            inventory: files,
        };
        let encoded = match serde_json::to_vec(&request) {
            Ok(bytes) => bytes,
            Err(error) => return ToolOutcome::error(format!("cannot encode PoC request: {error}")),
        };
        let response = match ctx
            .executor
            .target_service_call(SERVICE_NAME, ctx.sandbox.root(), &encoded)
            .await
        {
            Ok(response) => response,
            Err(error) => return ToolOutcome::error(format!("remote PoC runner failed: {error}")),
        };
        let response: SecurityPocRunResponse = match serde_json::from_slice(&response) {
            Ok(response) => response,
            Err(error) => {
                return ToolOutcome::error(format!("invalid remote PoC response: {error}"))
            }
        };

        // Persist stdout/stderr alongside the receipt the runner already wrote on
        // the target, then record the sealed receipt in the session ledger.
        let output = ExecOutput {
            stdout: response.stdout,
            stderr: response.stderr,
            code: response.receipt.exit_code,
        };
        let receipt = convert_receipt(response.receipt);
        let artifact_path = receipt.artifact_path.clone();
        if let Err(error) = persist_artifacts(ctx, &run_root, &receipt, &output).await {
            return ToolOutcome::error(error);
        }
        if let Err(error) = ctx
            .session
            .lock()
            .await
            .security_poc
            .record(receipt.clone())
        {
            return ToolOutcome::error(error);
        }

        let details = json!({
            "receipt": receipt,
            "expectedObservation": args.expected_observation,
            "stdout": complete_output(&output.stdout),
            "stderr": complete_output(&output.stderr),
        });
        ToolOutcome::ok(details.to_string())
            .with_details(details)
            .with_location(artifact_path, None)
    }
}

/// Resolve the project-relative inventory and confirm it matches the requested
/// `inventory_id`, shared by the local and remote PoC paths.
async fn resolve_inventory(
    args: &PocArgs,
    ctx: &ToolCtx,
) -> Result<crate::security::SecurityInventory, String> {
    let scope = ctx.sandbox.resolve_existing(&args.scope)?;
    let inventory =
        collect_security_inventory(ctx.executor.as_ref(), ctx.sandbox.root(), &scope).await?;
    if inventory.inventory_id != args.inventory_id {
        return Err(
            "inventory_id is stale; re-run security_scan_contract inventory before PoC execution"
                .to_string(),
        );
    }
    Ok(inventory)
}

/// Convert the runner's wire receipt into the session's `SecurityPocReceipt`.
/// They serialize identically; the explicit field mapping keeps the two types
/// free to evolve their Rust shapes independently.
fn convert_receipt(receipt: security_poc_runner::SecurityPocReceipt) -> SecurityPocReceipt {
    use security_poc_runner::{PocControl as RC, PocExecutionMetadata as REM};
    SecurityPocReceipt {
        contract_version: receipt.contract_version,
        receipt_id: receipt.receipt_id,
        scan_id: receipt.scan_id,
        candidate_id: receipt.candidate_id,
        inventory_id: receipt.inventory_id,
        control: match receipt.control {
            RC::Positive => SecurityPocControl::Positive,
            RC::Negative => SecurityPocControl::Negative,
        },
        language: receipt.language,
        script_sha256: receipt.script_sha256,
        expected_observation_sha256: receipt.expected_observation_sha256,
        workspace_sha256: receipt.workspace_sha256,
        stdout_sha256: receipt.stdout_sha256,
        stderr_sha256: receipt.stderr_sha256,
        expected_exit_code: receipt.expected_exit_code,
        exit_code: receipt.exit_code,
        passed: receipt.passed,
        containment: receipt.containment,
        artifact_path: receipt.artifact_path,
        execution: receipt.execution.map(|execution: REM| {
            crate::security::SecurityPocExecutionMetadata {
                expected_observation: execution.expected_observation,
                started_at_ms: execution.started_at_ms,
                completed_at_ms: execution.completed_at_ms,
                timeout_ms: execution.timeout_ms,
                output_limit_bytes: execution.output_limit_bytes,
                sandbox_provider: execution.sandbox_provider,
                sandbox_profile_sha256: execution.sandbox_profile_sha256,
                script_path: execution.script_path,
                stdout_path: execution.stdout_path,
                stderr_path: execution.stderr_path,
            }
        }),
    }
}

fn epoch_ms() -> Result<i64, String> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?;
    i64::try_from(duration.as_millis())
        .map_err(|_| "system clock exceeds the Clark Security timestamp range".to_string())
}

fn parse_args(value: &Value) -> Result<PocArgs, String> {
    let args: PocArgs = serde_json::from_value(value.clone())
        .map_err(|_| "invalid security PoC request".to_string())?;
    validate_id("scan_id", &args.scan_id)?;
    validate_id("candidate_id", &args.candidate_id)?;
    validate_id("inventory_id", &args.inventory_id)?;
    if args.expected_observation.trim().is_empty() {
        return Err("expected_observation must not be empty".to_string());
    }
    if args.script.is_empty() || args.script.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "script must contain between 1 and {MAX_SCRIPT_BYTES} bytes"
        ));
    }
    if args.timeout_seconds == 0 || args.timeout_seconds > MAX_TIMEOUT_SECONDS {
        return Err(format!(
            "timeout_seconds must be between 1 and {MAX_TIMEOUT_SECONDS}"
        ));
    }
    Ok(args)
}

fn validate_id(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Err(format!(
            "{name} must contain only letters, numbers, `.`, `_`, or `-`"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;
    use crate::tools::ReadTracker;
    use std::sync::{Arc, Mutex};

    fn context(root: &Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(root).unwrap()),
            executor: Arc::new(crate::exec::LocalExecutor),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: tokio_util::sync::CancellationToken::new(),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(
                crate::loop_state::SessionState::default(),
            )),
            progress: None,
            agent_progress: None,
            call_progress: None,
            model_override: None,
        }
    }

    #[test]
    fn rejects_path_shaped_ids_and_oversized_timeouts() {
        let unsafe_id = json!({
            "scan_id": "../scan",
            "candidate_id": "candidate-1",
            "inventory_id": "inventory-1",
            "control": "positive",
            "language": "shell",
            "expected_observation": "the vulnerable branch is reached",
            "script": "exit 0"
        });
        assert!(parse_args(&unsafe_id).unwrap_err().contains("scan_id"));

        let long = json!({
            "scan_id": "scan-1",
            "candidate_id": "candidate-1",
            "inventory_id": "inventory-1",
            "control": "negative",
            "language": "python",
            "expected_observation": "the safe control rejects the input",
            "script": "raise SystemExit(0)",
            "timeout_seconds": MAX_TIMEOUT_SECONDS + 1
        });
        assert!(parse_args(&long).unwrap_err().contains("timeout_seconds"));
    }

    #[test]
    fn isolated_poc_execution_never_waits_for_permission() {
        let args = json!({
            "scan_id": "scan-1",
            "candidate_id": "candidate-1",
            "inventory_id": "inventory-1",
            "control": "positive",
            "language": "shell",
            "expected_observation": "the vulnerable branch is reached",
            "script": "exit 0"
        });
        let scope = SecurityPocExecute
            .permission_scope(&args)
            .expect("permission scope");
        assert!(scope.preapproved);
        assert_eq!(scope.risk, None);
    }

    #[tokio::test]
    async fn full_tool_issues_distinct_receipts_without_mutating_checkout() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.txt"), "ORIGINAL VULNERABLE\n").unwrap();
        let probe_root = tempfile::tempdir().unwrap();
        let probe_policy = SandboxPolicy::read_only().with_write_roots([probe_root.path().into()]);
        if SandboxedExecutor::new(probe_policy).is_err() {
            // Some minimal CI hosts intentionally omit the platform sandbox
            // helper. Production refuses the execution in this state.
            return;
        }

        let ctx = context(root.path());
        let inventory = collect_security_inventory(
            ctx.executor.as_ref(),
            ctx.sandbox.root(),
            ctx.sandbox.root(),
        )
        .await
        .unwrap();
        let base = json!({
            "scan_id": "scan-integration",
            "candidate_id": "candidate-integration",
            "inventory_id": inventory.inventory_id,
            "scope": ".",
            "language": "shell",
            "expected_exit_code": 0,
            "timeout_seconds": 10
        });
        let mut positive = base.clone();
        positive["control"] = json!("positive");
        positive["expected_observation"] = json!("the vulnerable marker is present");
        positive["script"] =
            json!("grep -q VULNERABLE source.txt && printf 'MUTATED\\n' > source.txt");
        let positive = SecurityPocExecute.invoke(positive, &ctx).await;
        assert!(!positive.is_error, "{}", positive.content);

        let mut negative = base;
        negative["control"] = json!("negative");
        negative["expected_observation"] = json!("a safe marker remains absent");
        negative["script"] =
            json!("! grep -q SAFE source.txt && printf 'NEGATIVE-COPY\\n' > source.txt");
        let negative = SecurityPocExecute.invoke(negative, &ctx).await;
        assert!(!negative.is_error, "{}", negative.content);

        assert_eq!(
            std::fs::read_to_string(root.path().join("source.txt")).unwrap(),
            "ORIGINAL VULNERABLE\n"
        );
        let positive_id = positive.details["receipt"]["receiptId"].as_str().unwrap();
        let negative_id = negative.details["receipt"]["receiptId"].as_str().unwrap();
        assert_ne!(positive_id, negative_id);
        let session = ctx.session.lock().await;
        assert!(session.security_poc.get(positive_id).is_some());
        assert!(session.security_poc.get(negative_id).is_some());
    }

    /// An executor that reports `is_local() == false` and answers the
    /// `security-poc-v1` target-service call by running the real shared runner
    /// against a directory it owns — standing in for any external executor.
    /// Everything else delegates to the local executor over the same root.
    struct FakeExternalExecutor {
        root: std::path::PathBuf,
        local: crate::exec::LocalExecutor,
    }

    #[async_trait]
    impl crate::exec::Executor for FakeExternalExecutor {
        fn is_local(&self) -> bool {
            false
        }

        async fn target_service_call(
            &self,
            service: &str,
            _root: &Path,
            request: &[u8],
        ) -> exec_core::ExecResult<Vec<u8>> {
            security_poc_runner::dispatch(service, &self.root, request).await
        }

        async fn read(&self, path: &Path) -> exec_core::ExecResult<Vec<u8>> {
            self.local.read(path).await
        }
        async fn write(&self, path: &Path, data: &[u8]) -> exec_core::ExecResult<()> {
            self.local.write(path, data).await
        }
        async fn create_dir_all(&self, path: &Path) -> exec_core::ExecResult<()> {
            self.local.create_dir_all(path).await
        }
        async fn remove_file(&self, path: &Path) -> exec_core::ExecResult<()> {
            self.local.remove_file(path).await
        }
        async fn remove_dir_all(&self, path: &Path) -> exec_core::ExecResult<()> {
            self.local.remove_dir_all(path).await
        }
        async fn rename(&self, from: &Path, to: &Path) -> exec_core::ExecResult<()> {
            self.local.rename(from, to).await
        }
        async fn read_dir(&self, path: &Path) -> exec_core::ExecResult<Vec<exec_core::DirEntry>> {
            self.local.read_dir(path).await
        }
        async fn metadata(&self, path: &Path) -> exec_core::ExecResult<exec_core::FileMeta> {
            self.local.metadata(path).await
        }
        async fn canonicalize(&self, path: &Path) -> exec_core::ExecResult<std::path::PathBuf> {
            self.local.canonicalize(path).await
        }
        async fn home_dir(&self, cwd: &Path) -> exec_core::ExecResult<std::path::PathBuf> {
            self.local.home_dir(cwd).await
        }
        async fn walk(&self, root: &Path) -> exec_core::ExecResult<Vec<exec_core::WalkEntry>> {
            self.local.walk(root).await
        }
        async fn exec(
            &self,
            command: &str,
            cwd: &Path,
            timeout: std::time::Duration,
            cancel: &tokio_util::sync::CancellationToken,
        ) -> exec_core::ExecResult<exec_core::ExecOutput> {
            self.local.exec(command, cwd, timeout, cancel).await
        }
    }

    #[tokio::test]
    async fn managed_executor_routes_poc_through_the_target_service() {
        // The shared runner seals only inside a locally enforced OS sandbox;
        // minimal CI hosts (e.g. ubuntu without bubblewrap) omit it. Skip there
        // exactly like the local-path test does — production refuses in this
        // state, and the live e2e exercises the real sandboxed path on a host
        // that has containment.
        let probe_root = tempfile::tempdir().unwrap();
        let probe_policy = SandboxPolicy::read_only().with_write_roots([probe_root.path().into()]);
        if SandboxedExecutor::new(probe_policy).is_err() {
            return;
        }

        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.txt"), "REMOTE VULNERABLE\n").unwrap();
        let mut ctx = context(root.path());
        ctx.executor = Arc::new(FakeExternalExecutor {
            root: root.path().to_path_buf(),
            local: crate::exec::LocalExecutor,
        });

        let inventory = collect_security_inventory(
            ctx.executor.as_ref(),
            ctx.sandbox.root(),
            ctx.sandbox.root(),
        )
        .await
        .unwrap();
        let args = json!({
            "scan_id": "scan-remote",
            "candidate_id": "candidate-remote",
            "inventory_id": inventory.inventory_id,
            "scope": ".",
            "control": "positive",
            "language": "shell",
            "expected_observation": "the remote vulnerable marker is present",
            "script": "grep -q VULNERABLE source.txt",
            "expected_exit_code": 0,
            "timeout_seconds": 10
        });
        let outcome = SecurityPocExecute.invoke(args, &ctx).await;
        assert!(!outcome.is_error, "{}", outcome.content);

        // The receipt came back through the target service with managed
        // disposable containment and was recorded in the session ledger.
        let receipt = &outcome.details["receipt"];
        assert_eq!(receipt["containment"], json!("managed_disposable"));
        assert_eq!(receipt["passed"], json!(true));
        let receipt_id = receipt["receiptId"].as_str().unwrap();
        let session = ctx.session.lock().await;
        let recorded = session
            .security_poc
            .get(receipt_id)
            .expect("remote PoC receipt recorded");
        assert_eq!(recorded.containment, "managed_disposable");
        assert!(recorded.passed);
    }
}
