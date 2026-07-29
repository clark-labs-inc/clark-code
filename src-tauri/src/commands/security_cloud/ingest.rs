use std::path::PathBuf;

use serde_json::{json, Value};

use super::client::ClarkSecurityPlatformClient;
use super::evidence::{upload_scan_evidence, UploadedScanEvidence};
use super::identity::ClarkSecurityScannerIdentity;
use super::model::{
    LocalCloudReceipt, PlatformScan, ScannerEnrollment, SecurityCloudScanSync,
    SecurityCloudScanSyncStatus, SecurityCloudSyncResult,
};

const MAX_SCANS_PER_SYNC: usize = 20;
const PRODUCTION_MODEL: &str = "z-ai/glm-5.2";

#[path = "ingest/marker.rs"]
mod marker;
use marker::{marker_path, now_ms, read_matching_marker, write_marker};

pub(super) struct SecuritySyncRequest {
    pub rest_base: String,
    pub api_key: String,
    pub owner_scope: String,
    pub organization_id: String,
    pub repository_id: String,
    pub policy_id: Option<String>,
    pub root: PathBuf,
    pub identity_root: PathBuf,
    pub repository: provider_local::RepositoryIdentity,
    pub http: reqwest::Client,
}

pub(super) async fn sync_security_scans(
    request: SecuritySyncRequest,
) -> Result<SecurityCloudSyncResult, String> {
    let records =
        provider_local::list_security_scans(&provider_local::LocalExecutor, &request.root).await?;
    let sealed_scan_count = records
        .iter()
        .filter(|record| record.seal.is_some())
        .count();
    if sealed_scan_count == 0 {
        return Ok(SecurityCloudSyncResult::from_scans(0, Vec::new()));
    }

    let desktop_identity = load_identity(&request, "desktop")?;
    let poc_identity = load_identity(&request, "poc_lab")?;
    let client = ClarkSecurityPlatformClient::new(
        request.rest_base.clone(),
        request.api_key.clone(),
        request.http.clone(),
    )?;
    let desktop_scanner = enroll_identity(
        &client,
        &request.organization_id,
        "desktop",
        "Clark Desktop Security",
        &desktop_identity,
    )
    .await?;
    let poc_scanner = enroll_identity(
        &client,
        &request.organization_id,
        "poc_lab",
        "Clark Desktop PoC Lab",
        &poc_identity,
    )
    .await?;

    let mut attempted = 0usize;
    let mut results = Vec::with_capacity(sealed_scan_count);
    for record in records.into_iter().filter(|record| record.seal.is_some()) {
        let seal = record.seal.as_ref().expect("filtered sealed scan");
        let marker_path = marker_path(
            &request.root,
            &record,
            &request.organization_id,
            &request.repository_id,
        )
        .await?;
        if let Some(marker) = read_matching_marker(
            &marker_path,
            &request.organization_id,
            &request.repository_id,
            &record.bundle.scan_id,
            &seal.bundle_digest,
        )
        .await?
        {
            results.push(SecurityCloudScanSync {
                local_scan_id: record.bundle.scan_id.clone(),
                platform_scan_id: Some(marker.platform_scan_id),
                status: SecurityCloudScanSyncStatus::AlreadySynced,
                seal_receipt_key: marker.platform_seal_receipt_key,
                message: None,
            });
            continue;
        }
        if attempted >= MAX_SCANS_PER_SYNC {
            results.push(SecurityCloudScanSync {
                local_scan_id: record.bundle.scan_id.clone(),
                platform_scan_id: None,
                status: SecurityCloudScanSyncStatus::Pending,
                seal_receipt_key: None,
                message: Some("Queued for the next automatic Clark Security sync.".into()),
            });
            continue;
        }
        attempted += 1;
        match sync_one(
            &client,
            &request,
            &record,
            &poc_identity,
            &desktop_scanner,
            &poc_scanner,
        )
        .await
        {
            Ok((scan, seal_receipt_key)) => {
                let marker = LocalCloudReceipt {
                    product: "Clark Security".into(),
                    schema_version: 1,
                    organization_id: request.organization_id.clone(),
                    repository_id: request.repository_id.clone(),
                    local_scan_id: record.bundle.scan_id.clone(),
                    local_bundle_digest: seal.bundle_digest.clone(),
                    platform_scan_id: scan.id.clone(),
                    platform_seal_receipt_key: seal_receipt_key.clone(),
                    synced_at_ms: now_ms()?,
                };
                write_marker(&marker_path, &marker).await?;
                results.push(SecurityCloudScanSync {
                    local_scan_id: record.bundle.scan_id.clone(),
                    platform_scan_id: Some(scan.id),
                    status: SecurityCloudScanSyncStatus::Synced,
                    seal_receipt_key,
                    message: None,
                });
            }
            Err(error) if error.starts_with("pending:") => {
                results.push(SecurityCloudScanSync {
                    local_scan_id: record.bundle.scan_id.clone(),
                    platform_scan_id: None,
                    status: SecurityCloudScanSyncStatus::Pending,
                    seal_receipt_key: None,
                    message: Some(error.trim_start_matches("pending:").trim().into()),
                });
            }
            Err(error) => {
                results.push(SecurityCloudScanSync {
                    local_scan_id: record.bundle.scan_id.clone(),
                    platform_scan_id: None,
                    status: SecurityCloudScanSyncStatus::Failed,
                    seal_receipt_key: None,
                    message: Some(error),
                });
            }
        }
    }
    Ok(SecurityCloudSyncResult::from_scans(
        sealed_scan_count,
        results,
    ))
}

async fn sync_one(
    client: &ClarkSecurityPlatformClient,
    request: &SecuritySyncRequest,
    record: &provider_local::SecurityScanRecord,
    poc_identity: &ClarkSecurityScannerIdentity,
    desktop_scanner: &ScannerEnrollment,
    poc_scanner: &ScannerEnrollment,
) -> Result<(PlatformScan, Option<String>), String> {
    let identity = provider_local::clark_security_cloud_identity(record)?;
    let (target_kind, revision, base_revision) = scan_target(record, &request.repository)?;
    let scan = client
        .create_scan(&json!({
            "organizationId": request.organization_id,
            "repositoryId": request.repository_id,
            "policyId": request.policy_id,
            "parentScanId": null,
            "clientScanId": identity.client_scan_id,
            "idempotencyKey": identity.idempotency_key,
            "mode": scan_mode(identity.mode),
            "targetKind": target_kind,
            "revision": revision,
            "baseRevision": base_revision,
            "snapshotDigest": identity.snapshot_digest,
            "inventoryId": identity.inventory_id,
            "scannerVersion": format!("clark-desktop/{}", env!("CARGO_PKG_VERSION")),
            "executionLane": "production",
            "model": PRODUCTION_MODEL,
            "trigger": "desktop",
        }))
        .await?;
    validate_scan_binding(&scan, request, &identity)?;
    if scan.status == "completed" {
        return Ok((scan, None));
    }
    require_mutable_scan(&scan)?;

    let scan_uuid = uuid::Uuid::parse_str(&scan.id)
        .map_err(|_| "Clark Security returned an invalid scan id".to_string())?;
    let repository_uuid = uuid::Uuid::parse_str(&request.repository_id)
        .map_err(|_| "Clark Security repository id is invalid".to_string())?;
    let export =
        provider_local::build_clark_security_cloud_export(record, repository_uuid, scan_uuid)?;
    let evidence = upload_scan_evidence(
        client,
        &request.root,
        record,
        &export,
        &scan,
        poc_identity,
        poc_scanner,
    )
    .await?;
    advance_pipeline(
        client,
        record,
        &scan,
        desktop_scanner,
        poc_scanner,
        &evidence,
    )
    .await?;

    let Some(seal_task) = client
        .claim_task(
            &scan.organization_id,
            &scan.repository_id,
            &scan.id,
            &desktop_scanner.id,
            "seal",
        )
        .await?
    else {
        let current = client.get_scan(&scan.organization_id, &scan.id).await?;
        return match current.status.as_str() {
            "completed" => Ok((current, None)),
            "failed" | "canceled" | "superseded" => Err(format!(
                "Clark Security scan became terminal with status {}",
                current.status
            )),
            _ => Err(format!(
                "pending: Clark Security scan is currently {} and will resume automatically.",
                current.status
            )),
        };
    };
    let seal = client
        .seal_scan(
            &scan.id,
            &json!({
                "organizationId": scan.organization_id,
                "repositoryId": scan.repository_id,
                "scanId": scan.id,
                "scannerId": desktop_scanner.id,
                "sealTaskId": seal_task.task.id,
                "leaseFence": seal_task.task.lease_fence,
                "expectedScanVersion": seal_task.scan.version,
                "inventoryId": scan.inventory_id,
                "coverageCompleteness": export.coverage_completeness,
                "coverageSurfaces": export.coverage_surfaces,
                "manifestArtifactId": evidence.manifest_artifact_id,
                "findingsArtifactId": evidence.findings_artifact_id,
                "coverageArtifactId": evidence.coverage_artifact_id,
                "occurrences": evidence.occurrences,
            }),
        )
        .await?;
    if seal.scan.id != scan.id
        || seal.scan.organization_id != scan.organization_id
        || seal.scan.repository_id != scan.repository_id
        || seal.scan.status != "completed"
        || !seal.receipt.receipt_key.starts_with("security-seal:")
    {
        return Err("Clark Security seal response did not match the requested scan".into());
    }
    Ok((seal.scan, Some(seal.receipt.receipt_key)))
}

async fn advance_pipeline(
    client: &ClarkSecurityPlatformClient,
    record: &provider_local::SecurityScanRecord,
    scan: &PlatformScan,
    desktop_scanner: &ScannerEnrollment,
    poc_scanner: &ScannerEnrollment,
    evidence: &UploadedScanEvidence,
) -> Result<(), String> {
    for task_kind in [
        "inventory",
        "threat_model",
        "discovery",
        "attack_path",
        "validation",
    ] {
        let Some(claimed) = client
            .claim_task(
                &scan.organization_id,
                &scan.repository_id,
                &scan.id,
                &desktop_scanner.id,
                task_kind,
            )
            .await?
        else {
            continue;
        };
        let mut result = json!({
            "source": "clark-desktop",
            "localScanId": record.bundle.scan_id,
            "localSealDigest": record.seal.as_ref().map(|seal| &seal.bundle_digest),
        });
        if task_kind == "validation" {
            result["pocRequired"] = Value::Bool(!evidence.occurrences.is_empty());
        }
        client
            .complete_task(
                &scan.organization_id,
                &scan.repository_id,
                &scan.id,
                &desktop_scanner.id,
                &claimed.task,
                result,
            )
            .await?;
    }
    if !evidence.occurrences.is_empty() {
        if let Some(claimed) = client
            .claim_task(
                &scan.organization_id,
                &scan.repository_id,
                &scan.id,
                &poc_scanner.id,
                "poc",
            )
            .await?
        {
            client
                .complete_task(
                    &scan.organization_id,
                    &scan.repository_id,
                    &scan.id,
                    &poc_scanner.id,
                    &claimed.task,
                    json!({
                        "source": "clark-desktop",
                        "attempted": evidence.occurrences.len(),
                    }),
                )
                .await?;
        }
    }
    if let Some(claimed) = client
        .claim_task(
            &scan.organization_id,
            &scan.repository_id,
            &scan.id,
            &desktop_scanner.id,
            "adjudication",
        )
        .await?
    {
        client
            .complete_task(
                &scan.organization_id,
                &scan.repository_id,
                &scan.id,
                &desktop_scanner.id,
                &claimed.task,
                json!({
                    "source": "clark-desktop",
                    "occurrenceCount": evidence.occurrences.len(),
                }),
            )
            .await?;
    }
    Ok(())
}

fn load_identity(
    request: &SecuritySyncRequest,
    kind: &str,
) -> Result<ClarkSecurityScannerIdentity, String> {
    ClarkSecurityScannerIdentity::load_or_create(
        &request.identity_root,
        &format!(
            "{}|{}|{}|{kind}",
            request.rest_base, request.owner_scope, request.organization_id
        ),
    )
}

async fn enroll_identity(
    client: &ClarkSecurityPlatformClient,
    organization_id: &str,
    kind: &str,
    display_name: &str,
    identity: &ClarkSecurityScannerIdentity,
) -> Result<ScannerEnrollment, String> {
    let enrollment = client
        .enroll_scanner(
            organization_id,
            &identity.public_key_hex(),
            kind,
            display_name,
        )
        .await?;
    if enrollment.signer_id != identity.signer_id() {
        return Err("Clark Security enrollment returned a different signer identity".into());
    }
    Ok(enrollment)
}

fn validate_scan_binding(
    scan: &PlatformScan,
    request: &SecuritySyncRequest,
    identity: &provider_local::ClarkSecurityCloudIdentity,
) -> Result<(), String> {
    if scan.organization_id != request.organization_id
        || scan.repository_id != request.repository_id
        || scan.client_scan_id != identity.client_scan_id
        || scan.snapshot_digest != identity.snapshot_digest
        || scan.inventory_id != identity.inventory_id
        || scan.model != PRODUCTION_MODEL
    {
        return Err("Clark Security scan creation changed the immutable local target".into());
    }
    Ok(())
}

fn require_mutable_scan(scan: &PlatformScan) -> Result<(), String> {
    if matches!(scan.status.as_str(), "failed" | "canceled" | "superseded") {
        Err(format!(
            "Clark Security scan is terminal with status {}",
            scan.status
        ))
    } else {
        Ok(())
    }
}

fn scan_target(
    record: &provider_local::SecurityScanRecord,
    repository: &provider_local::RepositoryIdentity,
) -> Result<(&'static str, Option<String>, Option<String>), String> {
    if record.bundle.mode == provider_local::security::SecurityScanMode::Diff {
        let target = record
            .bundle
            .diff_target
            .as_ref()
            .ok_or_else(|| "Clark Security diff scan has no sealed diff target".to_string())?;
        let revision = target
            .head
            .clone()
            .or_else(|| repository.head_oid.clone())
            .ok_or_else(|| "Clark Security diff scan has no target revision".to_string())?;
        return Ok(("git_diff", Some(revision), Some(target.base.clone())));
    }
    Ok((
        if repository.dirty {
            "git_worktree"
        } else {
            "git_revision"
        },
        repository.head_oid.clone(),
        None,
    ))
}

fn scan_mode(mode: provider_local::security::SecurityScanMode) -> &'static str {
    match mode {
        provider_local::security::SecurityScanMode::Standard => "standard",
        provider_local::security::SecurityScanMode::Diff => "diff",
        provider_local::security::SecurityScanMode::Deep => "deep",
    }
}
