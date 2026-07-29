use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

use super::super::client::ClarkSecurityPlatformClient;
use super::super::identity::ClarkSecurityScannerIdentity;
use super::super::model::{ArtifactRecord, ArtifactSpec, PlatformScan, ScannerEnrollment};
use super::super::poc::{sign_claim, PocReceiptClaim, PocResourceLimits};

const MAX_SCRIPT_BYTES: u64 = 256 * 1024;
const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_POC_TEXT_BYTES: usize = 16 * 1024;

pub(super) async fn upload_executed_attestation(
    client: &ClarkSecurityPlatformClient,
    root: &Path,
    record: &provider_local::SecurityScanRecord,
    platform_scan: &PlatformScan,
    candidate: &provider_local::security::SecurityCandidate,
    occurrence: &provider_local::ClarkSecurityOccurrenceDraft,
    receipt: &provider_local::security::SecurityPocReceipt,
    poc_identity: &ClarkSecurityScannerIdentity,
    poc_scanner: &ScannerEnrollment,
) -> Result<Value, String> {
    verify_local_receipt(record, candidate, receipt)?;
    let execution = receipt.execution.as_ref().ok_or_else(|| {
        format!(
            "PoC receipt {} predates cloud-verifiable Clark execution metadata; rerun the scan",
            receipt.receipt_id
        )
    })?;
    require_poc_text("PoC expected observation", &execution.expected_observation)?;
    let script = read_verified_artifact(
        root,
        &record.bundle.scan_id,
        &execution.script_path,
        MAX_SCRIPT_BYTES,
        &receipt.script_sha256,
    )
    .await?;
    let stdout = read_verified_artifact(
        root,
        &record.bundle.scan_id,
        &execution.stdout_path,
        MAX_OUTPUT_BYTES,
        &receipt.stdout_sha256,
    )
    .await?;
    let stderr = read_verified_artifact(
        root,
        &record.bundle.scan_id,
        &execution.stderr_path,
        MAX_OUTPUT_BYTES,
        &receipt.stderr_sha256,
    )
    .await?;

    let script_artifact = upload_vault_bytes(client, platform_scan, "poc_recipe", script).await?;
    let stdout_artifact = if stdout.is_empty() {
        None
    } else {
        Some(upload_vault_bytes(client, platform_scan, "poc_stdout", stdout).await?)
    };
    let stderr_artifact = if stderr.is_empty() {
        None
    } else {
        Some(upload_vault_bytes(client, platform_scan, "poc_stderr", stderr).await?)
    };
    let attested_at_ms = attestation_time(execution.completed_at_ms, poc_scanner.enrolled_at_ms)?;
    let attestation = json_bytes(&json!({
        "documentType": "clark-security.poc-attestation",
        "schemaVersion": "1.0",
        "candidateId": occurrence.candidate_id,
        "localReceipt": receipt,
        "attestedAtMs": attested_at_ms,
        "containment": {
            "mode": "managed_disposable",
            "network": "offline",
            "enforced": [
                "network",
                "repository_copy",
                "write_root",
                "wall_time",
                "output_bytes",
                "process_tree_cleanup"
            ],
            "declaredCeilings": [
                "memory_bytes",
                "process_count",
                "file_count"
            ]
        }
    }))?;
    let attestation_artifact =
        upload_vault_bytes(client, platform_scan, "sandbox_attestation", attestation).await?;
    let observed_summary = format!(
        "{} control exited {:?}; Clark's expected-observation assertion {}",
        control_name(receipt.control),
        receipt.exit_code,
        if receipt.passed { "passed" } else { "failed" }
    );
    sign_claim(
        poc_identity,
        PocReceiptClaim {
            organization_id: platform_scan.organization_id.clone(),
            repository_id: platform_scan.repository_id.clone(),
            scan_id: platform_scan.id.clone(),
            scanner_id: poc_scanner.id.clone(),
            receipt_key: String::new(),
            candidate_id: occurrence.candidate_id.clone(),
            inventory_id: platform_scan.inventory_id.clone(),
            snapshot_digest: platform_scan.snapshot_digest.clone(),
            control: control_name(receipt.control).into(),
            execution_outcome: executed_outcome(candidate.poc.outcome, receipt.control).into(),
            containment: "managed_disposable".into(),
            network_mode: "offline".into(),
            sandbox_provider: execution.sandbox_provider.clone(),
            sandbox_image_digest: format!("sha256:{}", execution.sandbox_profile_sha256),
            script_sha256: format!("sha256:{}", receipt.script_sha256),
            workspace_sha256: format!("sha256:{}", receipt.workspace_sha256),
            stdout_sha256: Some(format!("sha256:{}", receipt.stdout_sha256)),
            stderr_sha256: Some(format!("sha256:{}", receipt.stderr_sha256)),
            expected_observation: execution.expected_observation.clone(),
            observed_summary,
            exit_code: receipt.exit_code,
            resource_limits: resource_limits(execution.timeout_ms, execution.output_limit_bytes),
            script_artifact_id: Some(script_artifact.id),
            stdout_artifact_id: stdout_artifact.map(|artifact| artifact.id),
            stderr_artifact_id: stderr_artifact.map(|artifact| artifact.id),
            attestation_artifact_id: attestation_artifact.id,
            started_at_ms: execution.started_at_ms,
            completed_at_ms: execution.completed_at_ms,
            attested_at_ms,
        },
    )
}

pub(super) async fn upload_deferred_attestation(
    client: &ClarkSecurityPlatformClient,
    record: &provider_local::SecurityScanRecord,
    platform_scan: &PlatformScan,
    candidate: &provider_local::security::SecurityCandidate,
    occurrence: &provider_local::ClarkSecurityOccurrenceDraft,
    poc_identity: &ClarkSecurityScannerIdentity,
    poc_scanner: &ScannerEnrollment,
) -> Result<Value, String> {
    require_poc_text("PoC goal", &candidate.poc.goal)?;
    let observed_summary = if candidate.poc.limitations.is_empty() {
        "Clark recorded the PoC as unavailable without bypassing containment.".to_string()
    } else {
        candidate.poc.limitations.join("; ")
    };
    require_poc_text("PoC limitation", &observed_summary)?;
    let attested_at_ms = attestation_time(1, poc_scanner.enrolled_at_ms)?;
    let attestation = json_bytes(&json!({
        "documentType": "clark-security.poc-attestation",
        "schemaVersion": "1.0",
        "candidateId": occurrence.candidate_id,
        "localScanId": record.bundle.scan_id,
        "outcome": candidate.poc.outcome,
        "goal": candidate.poc.goal,
        "limitations": candidate.poc.limitations,
        "attestedAtMs": attested_at_ms,
        "containmentBypassAttempted": false,
    }))?;
    let attestation_artifact =
        upload_vault_bytes(client, platform_scan, "sandbox_attestation", attestation).await?;
    let digest = sha256_hex(
        format!(
            "clark-security-deferred-poc/v1\0{}\0{}",
            occurrence.candidate_id, candidate.poc.goal
        )
        .as_bytes(),
    );
    let execution_outcome = match candidate.poc.outcome {
        provider_local::security::SecurityPocOutcome::Blocked => "blocked",
        provider_local::security::SecurityPocOutcome::UnsafeToExecute => "unsafe_to_execute",
        _ => return Err("only blocked or unsafe PoCs can use deferred attestation".into()),
    };
    sign_claim(
        poc_identity,
        PocReceiptClaim {
            organization_id: platform_scan.organization_id.clone(),
            repository_id: platform_scan.repository_id.clone(),
            scan_id: platform_scan.id.clone(),
            scanner_id: poc_scanner.id.clone(),
            receipt_key: String::new(),
            candidate_id: occurrence.candidate_id.clone(),
            inventory_id: platform_scan.inventory_id.clone(),
            snapshot_digest: platform_scan.snapshot_digest.clone(),
            control: "positive".into(),
            execution_outcome: execution_outcome.into(),
            containment: "managed_disposable".into(),
            network_mode: "offline".into(),
            sandbox_provider: "clark-desktop-native".into(),
            sandbox_image_digest: format!("sha256:{digest}"),
            script_sha256: format!("sha256:{digest}"),
            workspace_sha256: platform_scan.snapshot_digest.clone(),
            stdout_sha256: None,
            stderr_sha256: None,
            expected_observation: candidate.poc.goal.clone(),
            observed_summary,
            exit_code: None,
            resource_limits: resource_limits(60_000, 1_048_576),
            script_artifact_id: None,
            stdout_artifact_id: None,
            stderr_artifact_id: None,
            attestation_artifact_id: attestation_artifact.id,
            started_at_ms: attested_at_ms,
            completed_at_ms: attested_at_ms,
            attested_at_ms,
        },
    )
}

async fn upload_vault_bytes(
    client: &ClarkSecurityPlatformClient,
    scan: &PlatformScan,
    role: &'static str,
    bytes: Vec<u8>,
) -> Result<ArtifactRecord, String> {
    client
        .upload_artifact(
            &scan.organization_id,
            &scan.repository_id,
            &scan.id,
            &ArtifactSpec {
                role,
                storage_tier: "zero_day_vault",
                classification: "restricted",
                content_type: if role == "sandbox_attestation" {
                    "application/json"
                } else {
                    "application/octet-stream"
                },
                bytes,
            },
        )
        .await
}

fn verify_local_receipt(
    record: &provider_local::SecurityScanRecord,
    candidate: &provider_local::security::SecurityCandidate,
    receipt: &provider_local::security::SecurityPocReceipt,
) -> Result<(), String> {
    let mut unsigned = receipt.clone();
    unsigned.receipt_id.clear();
    let preimage = serde_json::to_vec(&unsigned)
        .map_err(|error| format!("cannot verify local Clark PoC receipt: {error}"))?;
    let expected_id = format!("poc-{}", &sha256_hex(&preimage)[..32]);
    if receipt.receipt_id != expected_id
        || receipt.scan_id != record.bundle.scan_id
        || receipt.inventory_id != record.bundle.inventory_id
        || receipt.candidate_id != candidate.candidate_id
        || receipt.containment != "managed_disposable"
        || !receipt.passed
    {
        return Err(format!(
            "local Clark PoC receipt {} failed its sealed binding",
            receipt.receipt_id
        ));
    }
    Ok(())
}

async fn read_verified_artifact(
    root: &Path,
    scan_id: &str,
    relative: &str,
    maximum_bytes: u64,
    expected_sha256: &str,
) -> Result<Vec<u8>, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Clark PoC artifact path escapes the repository".into());
    }
    let expected_prefix = PathBuf::from(".clark")
        .join("security-scans")
        .join(scan_id)
        .join("poc")
        .join("runs");
    if !relative.starts_with(&expected_prefix) {
        return Err("Clark PoC artifact path is outside its sealed scan run".into());
    }
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|error| format!("cannot resolve Clark repository root: {error}"))?;
    let artifact = tokio::fs::canonicalize(root.join(relative))
        .await
        .map_err(|error| format!("cannot resolve Clark PoC artifact: {error}"))?;
    if !artifact.starts_with(&canonical_root) {
        return Err("Clark PoC artifact resolves outside the repository".into());
    }
    let metadata = tokio::fs::metadata(&artifact)
        .await
        .map_err(|error| format!("cannot inspect Clark PoC artifact: {error}"))?;
    if !metadata.is_file() || metadata.len() > maximum_bytes {
        return Err("Clark PoC artifact is not a bounded regular file".into());
    }
    let bytes = tokio::fs::read(&artifact)
        .await
        .map_err(|error| format!("cannot read Clark PoC artifact: {error}"))?;
    if sha256_hex(&bytes) != expected_sha256 {
        return Err("Clark PoC artifact digest no longer matches its local receipt".into());
    }
    Ok(bytes)
}

fn resource_limits(timeout_ms: u64, output_bytes: u64) -> PocResourceLimits {
    let wall_time_ms = timeout_ms.clamp(1, 60_000);
    PocResourceLimits {
        wall_time_ms,
        cpu_time_ms: wall_time_ms,
        memory_bytes: 2 * 1024 * 1024 * 1024,
        process_count: 1_024,
        file_count: 100_000,
        output_bytes: output_bytes.clamp(1, 8 * 1024 * 1024),
    }
}

fn executed_outcome(
    outcome: provider_local::security::SecurityPocOutcome,
    control: provider_local::security::SecurityPocControl,
) -> &'static str {
    match outcome {
        provider_local::security::SecurityPocOutcome::Reproduced
        | provider_local::security::SecurityPocOutcome::PartiallyReproduced => "passed",
        provider_local::security::SecurityPocOutcome::NotReproduced => match control {
            provider_local::security::SecurityPocControl::Positive => "failed",
            provider_local::security::SecurityPocControl::Negative => "passed",
        },
        provider_local::security::SecurityPocOutcome::Blocked => "blocked",
        provider_local::security::SecurityPocOutcome::UnsafeToExecute => "unsafe_to_execute",
    }
}

fn control_name(control: provider_local::security::SecurityPocControl) -> &'static str {
    match control {
        provider_local::security::SecurityPocControl::Positive => "positive",
        provider_local::security::SecurityPocControl::Negative => "negative",
    }
}

fn require_poc_text(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > MAX_POC_TEXT_BYTES {
        return Err(format!(
            "{label} must contain 1..={MAX_POC_TEXT_BYTES} bytes"
        ));
    }
    Ok(())
}

fn attestation_time(completed_at_ms: i64, enrolled_at_ms: i64) -> Result<i64, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?;
    let now = i64::try_from(now.as_millis())
        .map_err(|_| "system clock exceeds the Clark Security timestamp range".to_string())?;
    Ok(now.max(completed_at_ms).max(enrolled_at_ms))
}

fn json_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| format!("cannot encode Clark Security PoC attestation: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
