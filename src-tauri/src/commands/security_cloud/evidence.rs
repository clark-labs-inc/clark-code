use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

use super::client::ClarkSecurityPlatformClient;
use super::identity::ClarkSecurityScannerIdentity;
use super::model::{ArtifactRecord, ArtifactSpec, PlatformScan, ScannerEnrollment};

#[path = "evidence/poc_material.rs"]
mod poc_material;
use poc_material::{upload_deferred_attestation, upload_executed_attestation};

pub(super) struct UploadedScanEvidence {
    pub manifest_artifact_id: String,
    pub findings_artifact_id: String,
    pub coverage_artifact_id: String,
    pub occurrences: Vec<Value>,
}

pub(super) async fn upload_scan_evidence(
    client: &ClarkSecurityPlatformClient,
    root: &Path,
    record: &provider_local::SecurityScanRecord,
    export: &provider_local::ClarkSecurityCloudExport,
    platform_scan: &PlatformScan,
    poc_identity: &ClarkSecurityScannerIdentity,
    poc_scanner: &ScannerEnrollment,
) -> Result<UploadedScanEvidence, String> {
    let core = upload_scan_ledgers(client, record, export, platform_scan).await?;
    let mut occurrences = Vec::with_capacity(export.occurrences.len());
    for (candidate, occurrence) in record
        .bundle
        .candidates
        .iter()
        .zip(export.occurrences.iter())
    {
        if occurrence.candidate_id != format!("candidate:{}", candidate.candidate_id) {
            return Err(
                "Clark Security occurrence order diverged from the sealed candidate ledger".into(),
            );
        }
        let signed = match candidate.poc.outcome {
            provider_local::security::SecurityPocOutcome::Blocked
            | provider_local::security::SecurityPocOutcome::UnsafeToExecute => {
                vec![
                    upload_deferred_attestation(
                        client,
                        record,
                        platform_scan,
                        candidate,
                        occurrence,
                        poc_identity,
                        poc_scanner,
                    )
                    .await?,
                ]
            }
            provider_local::security::SecurityPocOutcome::Reproduced
            | provider_local::security::SecurityPocOutcome::PartiallyReproduced
            | provider_local::security::SecurityPocOutcome::NotReproduced => {
                if occurrence.local_poc_receipt_ids.len() != 2 {
                    return Err(format!(
                        "candidate {} requires two locally sealed PoC controls",
                        candidate.candidate_id
                    ));
                }
                let mut signed = Vec::with_capacity(2);
                for receipt_id in &occurrence.local_poc_receipt_ids {
                    let receipt = record
                        .poc_receipts
                        .iter()
                        .find(|receipt| &receipt.receipt_id == receipt_id)
                        .ok_or_else(|| {
                            format!(
                                "candidate {} is missing local PoC receipt {receipt_id}",
                                candidate.candidate_id
                            )
                        })?;
                    signed.push(
                        upload_executed_attestation(
                            client,
                            root,
                            record,
                            platform_scan,
                            candidate,
                            occurrence,
                            receipt,
                            poc_identity,
                            poc_scanner,
                        )
                        .await?,
                    );
                }
                signed
            }
        };
        occurrences.push(occurrence.wire_value(signed));
    }

    Ok(UploadedScanEvidence {
        manifest_artifact_id: required_artifact(&core, "manifest")?.id.clone(),
        findings_artifact_id: required_artifact(&core, "findings")?.id.clone(),
        coverage_artifact_id: required_artifact(&core, "coverage")?.id.clone(),
        occurrences,
    })
}

async fn upload_scan_ledgers(
    client: &ClarkSecurityPlatformClient,
    record: &provider_local::SecurityScanRecord,
    export: &provider_local::ClarkSecurityCloudExport,
    platform_scan: &PlatformScan,
) -> Result<BTreeMap<&'static str, ArtifactRecord>, String> {
    let mut specs = vec![
        json_artifact("manifest", &export.manifest)?,
        json_artifact("findings", &export.findings)?,
        json_artifact("coverage", &export.coverage)?,
        json_artifact(
            "inventory",
            &json!({
                "documentType": "clark-security.inventory",
                "schemaVersion": "1.0",
                "scanId": export.identity.client_scan_id,
                "inventoryId": export.identity.inventory_id,
                "snapshotDigest": export.identity.snapshot_digest,
                "scope": record.bundle.scope,
                "coverage": record.bundle.coverage,
                "supportingCoverage": record.bundle.supporting_coverage,
            }),
        )?,
        json_artifact(
            "candidate_ledger",
            &json!({
                "documentType": "clark-security.candidate-ledger",
                "schemaVersion": "1.0",
                "scanId": export.identity.client_scan_id,
                "bundle": record.bundle,
                "localSeal": record.seal,
            }),
        )?,
    ];
    if record.bundle.mode == provider_local::security::SecurityScanMode::Deep {
        specs.push(json_artifact(
            "threat_model",
            &json!({
                "documentType": "clark-security.threat-model",
                "schemaVersion": "1.0",
                "scanId": export.identity.client_scan_id,
                "threatModel": record.bundle.threat_model,
            }),
        )?);
        specs.push(json_artifact(
            "discovery_manifest",
            &json!({
                "documentType": "clark-security.discovery-manifest",
                "schemaVersion": "1.0",
                "scanId": export.identity.client_scan_id,
                "candidateIds": export.occurrences.iter().map(|row| {
                    row.candidate_id.as_str()
                }).collect::<Vec<_>>(),
                "coverageSurfaceIds": export.coverage_surfaces.iter().map(|row| {
                    row.surface_id.as_str()
                }).collect::<Vec<_>>(),
            }),
        )?);
        if !export.occurrences.is_empty() {
            specs.push(json_artifact(
                "validation_ledger",
                &json!({
                    "documentType": "clark-security.validation-ledger",
                    "schemaVersion": "1.0",
                    "scanId": export.identity.client_scan_id,
                    "candidates": record.bundle.candidates,
                }),
            )?);
        }
        if export
            .occurrences
            .iter()
            .any(|occurrence| occurrence.disposition == "reported")
        {
            specs.push(json_artifact(
                "attack_path_ledger",
                &json!({
                    "documentType": "clark-security.attack-path-ledger",
                    "schemaVersion": "1.0",
                    "scanId": export.identity.client_scan_id,
                    "attackPaths": record.bundle.candidates.iter().filter_map(|candidate| {
                        candidate.attack_path.as_ref().map(|path| json!({
                            "candidateId": format!("candidate:{}", candidate.candidate_id),
                            "attackPath": path,
                        }))
                    }).collect::<Vec<_>>(),
                }),
            )?);
        }
    }

    let mut uploaded = BTreeMap::new();
    for spec in specs {
        let role = spec.role;
        let artifact = client
            .upload_artifact(
                &platform_scan.organization_id,
                &platform_scan.repository_id,
                &platform_scan.id,
                &spec,
            )
            .await?;
        uploaded.insert(role, artifact);
    }
    Ok(uploaded)
}

fn json_artifact(role: &'static str, value: &Value) -> Result<ArtifactSpec, String> {
    Ok(ArtifactSpec {
        role,
        storage_tier: "evidence",
        classification: "confidential",
        content_type: "application/json",
        bytes: json_bytes(value)?,
    })
}

fn json_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| format!("cannot encode Clark Security evidence: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn required_artifact<'a>(
    artifacts: &'a BTreeMap<&str, ArtifactRecord>,
    role: &str,
) -> Result<&'a ArtifactRecord, String> {
    artifacts
        .get(role)
        .ok_or_else(|| format!("Clark Security {role} artifact was not uploaded"))
}
