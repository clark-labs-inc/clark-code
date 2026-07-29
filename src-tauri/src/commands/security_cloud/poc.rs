use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::identity::ClarkSecurityScannerIdentity;

const SIGNATURE_DOMAIN: &[u8] = b"clark.security.poc-receipt/v1\0";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PocResourceLimits {
    pub wall_time_ms: u64,
    pub cpu_time_ms: u64,
    pub memory_bytes: u64,
    pub process_count: u32,
    pub file_count: u32,
    pub output_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PocReceiptClaim {
    pub organization_id: String,
    pub repository_id: String,
    pub scan_id: String,
    pub scanner_id: String,
    pub receipt_key: String,
    pub candidate_id: String,
    pub inventory_id: String,
    pub snapshot_digest: String,
    pub control: String,
    pub execution_outcome: String,
    pub containment: String,
    pub network_mode: String,
    pub sandbox_provider: String,
    pub sandbox_image_digest: String,
    pub script_sha256: String,
    pub workspace_sha256: String,
    pub stdout_sha256: Option<String>,
    pub stderr_sha256: Option<String>,
    pub expected_observation: String,
    pub observed_summary: String,
    pub exit_code: Option<i32>,
    pub resource_limits: PocResourceLimits,
    pub script_artifact_id: Option<String>,
    pub stdout_artifact_id: Option<String>,
    pub stderr_artifact_id: Option<String>,
    pub attestation_artifact_id: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub attested_at_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PocClaimContent<'a> {
    organization_id: &'a str,
    repository_id: &'a str,
    scan_id: &'a str,
    scanner_id: &'a str,
    candidate_id: &'a str,
    inventory_id: &'a str,
    snapshot_digest: &'a str,
    control: &'a str,
    execution_outcome: &'a str,
    containment: &'a str,
    network_mode: &'a str,
    sandbox_provider: &'a str,
    sandbox_image_digest: &'a str,
    script_sha256: &'a str,
    workspace_sha256: &'a str,
    stdout_sha256: Option<&'a str>,
    stderr_sha256: Option<&'a str>,
    expected_observation: &'a str,
    observed_summary: &'a str,
    exit_code: Option<i32>,
    resource_limits: &'a PocResourceLimits,
    script_artifact_id: Option<&'a str>,
    stdout_artifact_id: Option<&'a str>,
    stderr_artifact_id: Option<&'a str>,
    attestation_artifact_id: &'a str,
    started_at_ms: i64,
    completed_at_ms: i64,
    attested_at_ms: i64,
}

impl<'a> From<&'a PocReceiptClaim> for PocClaimContent<'a> {
    fn from(claim: &'a PocReceiptClaim) -> Self {
        Self {
            organization_id: &claim.organization_id,
            repository_id: &claim.repository_id,
            scan_id: &claim.scan_id,
            scanner_id: &claim.scanner_id,
            candidate_id: &claim.candidate_id,
            inventory_id: &claim.inventory_id,
            snapshot_digest: &claim.snapshot_digest,
            control: &claim.control,
            execution_outcome: &claim.execution_outcome,
            containment: &claim.containment,
            network_mode: &claim.network_mode,
            sandbox_provider: &claim.sandbox_provider,
            sandbox_image_digest: &claim.sandbox_image_digest,
            script_sha256: &claim.script_sha256,
            workspace_sha256: &claim.workspace_sha256,
            stdout_sha256: claim.stdout_sha256.as_deref(),
            stderr_sha256: claim.stderr_sha256.as_deref(),
            expected_observation: &claim.expected_observation,
            observed_summary: &claim.observed_summary,
            exit_code: claim.exit_code,
            resource_limits: &claim.resource_limits,
            script_artifact_id: claim.script_artifact_id.as_deref(),
            stdout_artifact_id: claim.stdout_artifact_id.as_deref(),
            stderr_artifact_id: claim.stderr_artifact_id.as_deref(),
            attestation_artifact_id: &claim.attestation_artifact_id,
            started_at_ms: claim.started_at_ms,
            completed_at_ms: claim.completed_at_ms,
            attested_at_ms: claim.attested_at_ms,
        }
    }
}

pub(super) fn sign_claim(
    identity: &ClarkSecurityScannerIdentity,
    mut claim: PocReceiptClaim,
) -> Result<serde_json::Value, String> {
    claim.receipt_key.clear();
    let content = serde_json::to_vec(&PocClaimContent::from(&claim))
        .map_err(|error| format!("cannot encode Clark Security PoC claim: {error}"))?;
    claim.receipt_key = format!("poc-receipt:{}", sha256_hex(&content));
    let signature = identity.sign_hex(&transcript(&content));
    Ok(serde_json::json!({
        "claim": claim,
        "signerId": identity.signer_id(),
        "signature": signature,
    }))
}

fn transcript(payload: &[u8]) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(SIGNATURE_DOMAIN.len() + 8 + payload.len());
    transcript.extend_from_slice(SIGNATURE_DOMAIN);
    transcript.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    transcript.extend_from_slice(payload);
    transcript
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signature, VerifyingKey};

    use super::*;

    fn claim() -> PocReceiptClaim {
        PocReceiptClaim {
            organization_id: uuid::Uuid::new_v4().to_string(),
            repository_id: uuid::Uuid::new_v4().to_string(),
            scan_id: uuid::Uuid::new_v4().to_string(),
            scanner_id: uuid::Uuid::new_v4().to_string(),
            receipt_key: String::new(),
            candidate_id: "candidate:sql-injection".into(),
            inventory_id: format!("inventory:{}", "1".repeat(64)),
            snapshot_digest: format!("sha256:{}", "2".repeat(64)),
            control: "positive".into(),
            execution_outcome: "passed".into(),
            containment: "managed_disposable".into(),
            network_mode: "offline".into(),
            sandbox_provider: "clark-desktop-native".into(),
            sandbox_image_digest: format!("sha256:{}", "3".repeat(64)),
            script_sha256: format!("sha256:{}", "4".repeat(64)),
            workspace_sha256: format!("sha256:{}", "5".repeat(64)),
            stdout_sha256: Some(format!("sha256:{}", "6".repeat(64))),
            stderr_sha256: Some(format!("sha256:{}", "7".repeat(64))),
            expected_observation: "protected row marker is observed".into(),
            observed_summary: "positive control reproduced the marker".into(),
            exit_code: Some(0),
            resource_limits: PocResourceLimits {
                wall_time_ms: 5_000,
                cpu_time_ms: 5_000,
                memory_bytes: 128 * 1024 * 1024,
                process_count: 16,
                file_count: 1_024,
                output_bytes: 1_048_576,
            },
            script_artifact_id: Some(uuid::Uuid::new_v4().to_string()),
            stdout_artifact_id: Some(uuid::Uuid::new_v4().to_string()),
            stderr_artifact_id: Some(uuid::Uuid::new_v4().to_string()),
            attestation_artifact_id: uuid::Uuid::new_v4().to_string(),
            started_at_ms: 10,
            completed_at_ms: 20,
            attested_at_ms: 30,
        }
    }

    #[test]
    fn signed_claim_uses_the_clark_security_domain_and_canonical_receipt_key() {
        let temp = tempfile::tempdir().unwrap();
        let identity =
            ClarkSecurityScannerIdentity::load_or_create(temp.path().join("keys"), "binding")
                .unwrap();
        let signed = sign_claim(&identity, claim()).unwrap();
        let claim: PocReceiptClaim = serde_json::from_value(signed["claim"].clone()).unwrap();
        let content = serde_json::to_vec(&PocClaimContent::from(&claim)).unwrap();
        assert_eq!(
            claim.receipt_key,
            format!("poc-receipt:{}", sha256_hex(&content))
        );
        let public_key: [u8; 32] = hex::decode(identity.public_key_hex())
            .unwrap()
            .try_into()
            .unwrap();
        let signature: [u8; 64] = hex::decode(signed["signature"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        VerifyingKey::from_bytes(&public_key)
            .unwrap()
            .verify_strict(&transcript(&content), &Signature::from_bytes(&signature))
            .unwrap();
    }
}
