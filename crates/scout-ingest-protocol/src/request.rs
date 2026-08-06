use agent_orchestration::EnterpriseBatchBundle;
use serde::{Deserialize, Serialize};

use crate::crypto::{digest_hex, validate_digest_reference};
use crate::{ScoutTenantId, INGEST_PROTOCOL_SCHEMA_VERSION};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestRequest {
    pub schema_version: u16,
    pub tenant_id: ScoutTenantId,
    pub attempt_id: String,
    pub bundle: EnterpriseBatchBundle,
}

impl IngestRequest {
    pub fn new(
        tenant_id: ScoutTenantId,
        attempt_id: impl Into<String>,
        bundle: EnterpriseBatchBundle,
    ) -> Result<Self, String> {
        let request = Self {
            schema_version: INGEST_PROTOCOL_SCHEMA_VERSION,
            tenant_id,
            attempt_id: attempt_id.into(),
            bundle,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != INGEST_PROTOCOL_SCHEMA_VERSION {
            return Err("unsupported Scout central-ingestion request schema".into());
        }
        self.tenant_id.validate()?;
        validate_digest_reference("outbox attempt", &self.attempt_id, "outbox-attempt:")?;
        let enterprise_id = &self.bundle.signed_batch.batch.enterprise_id;
        self.bundle.trust_chain.verify(enterprise_id)?;
        self.bundle
            .trust_chain
            .verify_signed_batch(self.bundle.signed_batch.clone())?;
        Ok(())
    }

    pub fn envelope_sha256(&self) -> Result<String, String> {
        self.validate()?;
        serde_json::to_vec(&self.bundle.signed_batch)
            .map(|bytes| digest_hex(&bytes))
            .map_err(|error| error.to_string())
    }
}
