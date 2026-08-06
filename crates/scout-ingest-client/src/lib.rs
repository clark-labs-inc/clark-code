mod http;

use std::path::{Path, PathBuf};

use agent_orchestration::{EnterpriseBatchId, EnterpriseId};
use async_trait::async_trait;
use scout_ingest_protocol::{IngestReceipt, IngestRequest, ScoutTenantId};
use scout_store::{
    OutboxEntry, OutboxResolution, OutboxState, ScoutStoreRequest, ScoutStoreResponse,
};

pub use http::{ReqwestCentralIngestTransport, ReqwestTransportConfig};

#[async_trait]
pub trait CentralIngestTransport: Send + Sync {
    async fn submit(&self, request: &IngestRequest) -> Result<IngestReceipt, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryResult {
    pub outbox: OutboxEntry,
    pub receipt: IngestReceipt,
    pub idempotent_local_resolution: bool,
}

pub struct ScoutIngestClient<T> {
    store_root: PathBuf,
    tenant_id: ScoutTenantId,
    coordinator_public_key: String,
    transport: T,
}

impl<T: CentralIngestTransport> ScoutIngestClient<T> {
    pub fn new(
        store_root: impl Into<PathBuf>,
        tenant_id: ScoutTenantId,
        coordinator_public_key: impl Into<String>,
        transport: T,
    ) -> Self {
        Self {
            store_root: store_root.into(),
            tenant_id,
            coordinator_public_key: coordinator_public_key.into(),
            transport,
        }
    }

    pub async fn deliver(
        &self,
        enterprise_id: &EnterpriseId,
        batch_id: &EnterpriseBatchId,
        attempt_id: &str,
        previous_attempt_id: Option<&str>,
    ) -> Result<DeliveryResult, String> {
        let (started, _) = updated(scout_store::request(
            &self.store_root,
            ScoutStoreRequest::BeginOutboxDelivery {
                enterprise_id: enterprise_id.clone(),
                batch_id: batch_id.clone(),
                attempt_id: attempt_id.to_owned(),
                previous_attempt_id: previous_attempt_id.map(str::to_owned),
            },
        )?)?;
        if !matches!(started.state, OutboxState::InFlight { .. }) {
            return Err("central-ingestion delivery is already terminal".into());
        }
        let bundle =
            scout_store::outbox_delivery_bundle(&self.store_root, enterprise_id, batch_id)?;
        let request = IngestRequest::new(self.tenant_id.clone(), attempt_id, bundle)?;
        let envelope_sha256 = request.envelope_sha256()?;
        let receipt = self.transport.submit(&request).await?;
        verify_receipt(
            &receipt,
            &self.coordinator_public_key,
            &self.tenant_id,
            enterprise_id,
            batch_id,
            &request.bundle.trust_chain.anchor_manifest_id,
            &envelope_sha256,
        )?;
        let (outbox, idempotent_local_resolution) = updated(scout_store::request(
            &self.store_root,
            ScoutStoreRequest::ResolveOutboxDelivery {
                enterprise_id: enterprise_id.clone(),
                batch_id: batch_id.clone(),
                attempt_id: attempt_id.to_owned(),
                resolution: OutboxResolution::Acked,
                resolution_id: receipt.receipt_id.clone(),
            },
        )?)?;
        Ok(DeliveryResult {
            outbox,
            receipt,
            idempotent_local_resolution,
        })
    }
}

pub fn enqueue(
    store_root: &Path,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
) -> Result<(OutboxEntry, bool), String> {
    updated(scout_store::request(
        store_root,
        ScoutStoreRequest::EnqueueOutbox {
            enterprise_id: enterprise_id.clone(),
            batch_id: batch_id.clone(),
        },
    )?)
}

fn verify_receipt(
    receipt: &IngestReceipt,
    coordinator_public_key: &str,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    batch_id: &EnterpriseBatchId,
    anchor_manifest_id: &str,
    envelope_sha256: &str,
) -> Result<(), String> {
    receipt.verify(coordinator_public_key)?;
    if &receipt.tenant_id != tenant_id
        || &receipt.enterprise_id != enterprise_id
        || &receipt.batch_id != batch_id
        || receipt.anchor_manifest_id != anchor_manifest_id
        || receipt.envelope_sha256 != envelope_sha256
    {
        return Err("central-ingestion receipt does not acknowledge the requested envelope".into());
    }
    Ok(())
}

fn updated(response: ScoutStoreResponse) -> Result<(OutboxEntry, bool), String> {
    match response {
        ScoutStoreResponse::OutboxUpdated { entry, idempotent } => Ok((entry, idempotent)),
        _ => Err("Scout store returned the wrong outbox response kind".into()),
    }
}
