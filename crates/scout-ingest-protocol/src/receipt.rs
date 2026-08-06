use agent_orchestration::{EnterpriseBatchId, EnterpriseId};
use serde::{Deserialize, Serialize};

use crate::crypto::RECEIPT_DOMAIN;
use crate::crypto::{digest_hex, validate_digest_reference, verify, CoordinatorSigningKey};
use crate::ScoutTenantId;

pub const INGEST_PROTOCOL_SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestReceipt {
    pub schema_version: u16,
    pub tenant_id: ScoutTenantId,
    pub enterprise_id: EnterpriseId,
    pub anchor_manifest_id: String,
    pub batch_id: EnterpriseBatchId,
    pub envelope_sha256: String,
    pub batch_accumulator_root: String,
    pub batch_accumulator_count: u64,
    pub coordinator_id: String,
    pub coordinator_public_key: String,
    pub sequence: u64,
    pub issued_at_ms: u64,
    pub previous_receipt_id: Option<String>,
    pub receipt_id: String,
    pub signature: String,
}

impl IngestReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        tenant_id: ScoutTenantId,
        enterprise_id: EnterpriseId,
        anchor_manifest_id: String,
        batch_id: EnterpriseBatchId,
        envelope_sha256: String,
        batch_accumulator_root: String,
        batch_accumulator_count: u64,
        sequence: u64,
        issued_at_ms: u64,
        previous_receipt_id: Option<String>,
        signer: &CoordinatorSigningKey,
    ) -> Result<Self, String> {
        let mut receipt = Self {
            schema_version: INGEST_PROTOCOL_SCHEMA_VERSION,
            tenant_id,
            enterprise_id,
            anchor_manifest_id,
            batch_id,
            envelope_sha256,
            batch_accumulator_root,
            batch_accumulator_count,
            coordinator_id: signer.coordinator_id(),
            coordinator_public_key: signer.public_key_hex(),
            sequence,
            issued_at_ms,
            previous_receipt_id,
            receipt_id: String::new(),
            signature: String::new(),
        };
        receipt.validate_shape()?;
        let transcript = receipt.transcript();
        receipt.receipt_id = format!("central-ingestion:{}", digest_hex(&transcript));
        receipt.signature = signer.sign(&transcript);
        receipt.verify(&receipt.coordinator_public_key)?;
        Ok(receipt)
    }

    pub fn verify(&self, expected_coordinator_public_key: &str) -> Result<(), String> {
        self.validate_shape()?;
        if self.coordinator_public_key != expected_coordinator_public_key {
            return Err("ingest receipt is signed by an unpinned coordinator key".into());
        }
        let transcript = self.transcript();
        let expected_id = format!("central-ingestion:{}", digest_hex(&transcript));
        if self.receipt_id != expected_id {
            return Err("ingest receipt id does not match its authenticated content".into());
        }
        verify(
            &self.coordinator_public_key,
            &self.coordinator_id,
            &self.signature,
            &transcript,
        )
    }

    fn validate_shape(&self) -> Result<(), String> {
        if self.schema_version != INGEST_PROTOCOL_SCHEMA_VERSION {
            return Err("unsupported Scout central-ingestion receipt schema".into());
        }
        self.tenant_id.validate()?;
        validate_digest_reference(
            "enterprise trust anchor",
            &self.anchor_manifest_id,
            "trust-manifest:",
        )?;
        validate_digest_reference("enterprise batch", self.batch_id.as_str(), "batch:")?;
        validate_digest_reference("envelope SHA-256", &self.envelope_sha256, "")?;
        validate_digest_reference("batch accumulator root", &self.batch_accumulator_root, "")?;
        validate_digest_reference("coordinator", &self.coordinator_id, "coordinator:")?;
        if let Some(previous) = &self.previous_receipt_id {
            validate_digest_reference("previous ingest receipt", previous, "central-ingestion:")?;
        }
        if self.sequence == 0 || self.issued_at_ms == 0 {
            return Err("ingest receipt sequence and issued time must be positive".into());
        }
        if self.batch_accumulator_count != self.sequence {
            return Err("batch accumulator count must equal the accepted receipt sequence".into());
        }
        Ok(())
    }

    fn transcript(&self) -> Vec<u8> {
        let mut output = RECEIPT_DOMAIN.to_vec();
        for field in [
            self.schema_version.to_string(),
            self.tenant_id.as_str().to_owned(),
            self.enterprise_id.as_str().to_owned(),
            self.anchor_manifest_id.clone(),
            self.batch_id.as_str().to_owned(),
            self.envelope_sha256.clone(),
            self.batch_accumulator_root.clone(),
            self.batch_accumulator_count.to_string(),
            self.coordinator_id.clone(),
            self.coordinator_public_key.clone(),
            self.sequence.to_string(),
            self.issued_at_ms.to_string(),
            self.previous_receipt_id.clone().unwrap_or_default(),
        ] {
            output.extend_from_slice(&(field.len() as u64).to_le_bytes());
            output.extend_from_slice(field.as_bytes());
        }
        output
    }
}
