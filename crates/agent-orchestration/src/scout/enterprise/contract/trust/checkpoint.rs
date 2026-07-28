use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::crypto::{validate_signature, AuthTranscript, EnterpriseSigningKey};
use super::model::{
    validate_prefixed_digest, EnterpriseTrustManifest, ENTERPRISE_TRUST_SCHEMA_VERSION,
};
use crate::scout::enterprise::contract::{
    canonical_digest, EnterpriseBatch, EnterpriseBatchId, EnterpriseEvent, EnterpriseEventId,
    EnterpriseId,
};

const MAX_CHECKPOINT_APPROVALS: usize = 64;

mod commitment;
mod issue;
mod state;

pub use commitment::{
    EnterpriseLedgerCommitment, EnterpriseSnapshotCommitment, EnterpriseSnapshotCommitmentV2,
    ENTERPRISE_LEDGER_COMMITMENT_SCHEMA_VERSION, ENTERPRISE_SNAPSHOT_COMMITMENT_SCHEMA_VERSION,
    ENTERPRISE_SNAPSHOT_COMMITMENT_V2_SCHEMA_VERSION,
};
pub use state::{
    EnterpriseCheckpointCursor, EnterpriseCheckpointObservation, VerifiedEnterpriseCheckpoint,
    VerifiedEnterpriseInclusion,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseLedgerSummary {
    pub enterprise_id: EnterpriseId,
    pub batch_root: String,
    pub event_root: String,
    pub batch_count: u64,
    pub event_count: u64,
}

impl EnterpriseLedgerSummary {
    pub fn from_batches<'a>(
        enterprise_id: EnterpriseId,
        batches: impl IntoIterator<Item = &'a EnterpriseBatch>,
    ) -> Result<Self, String> {
        let mut batches_by_id = BTreeMap::<EnterpriseBatchId, &EnterpriseBatch>::new();
        let mut events_by_id = BTreeMap::<EnterpriseEventId, &EnterpriseEvent>::new();
        for batch in batches {
            batch.validate()?;
            if batch.enterprise_id != enterprise_id {
                return Err("ledger summary contains a batch for another enterprise".into());
            }
            if let Some(existing) = batches_by_id.insert(batch.batch_id.clone(), batch) {
                if existing != batch {
                    return Err("ledger summary contains a batch-id collision".into());
                }
            }
            for event in &batch.events {
                match events_by_id.insert(event.event_id.clone(), event) {
                    Some(existing) if existing != event => {
                        return Err("ledger summary contains an event-id collision".into())
                    }
                    _ => {}
                }
            }
        }
        let batch_root = canonical_digest(&BatchRoot {
            schema: "scout-enterprise-batch-root-v1",
            enterprise_id: &enterprise_id,
            batch_ids: batches_by_id.keys().collect(),
        })?;
        let event_root = canonical_digest(&EventRoot {
            schema: "scout-enterprise-event-root-v2",
            enterprise_id: &enterprise_id,
            event_ids: events_by_id.keys().collect(),
        })?;
        Ok(Self {
            enterprise_id,
            batch_root,
            event_root,
            batch_count: u64::try_from(batches_by_id.len())
                .map_err(|_| "ledger batch count does not fit in u64".to_string())?,
            event_count: u64::try_from(events_by_id.len())
                .map_err(|_| "ledger event count does not fit in u64".to_string())?,
        })
    }

    fn validate(&self) -> Result<(), String> {
        validate_digest("ledger batch root", &self.batch_root)?;
        validate_digest("ledger event root", &self.event_root)
    }
}

#[derive(Serialize)]
struct BatchRoot<'a> {
    schema: &'static str,
    enterprise_id: &'a EnterpriseId,
    batch_ids: Vec<&'a EnterpriseBatchId>,
}

#[derive(Serialize)]
struct EventRoot<'a> {
    schema: &'static str,
    enterprise_id: &'a EnterpriseId,
    event_ids: Vec<&'a EnterpriseEventId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseLedgerCheckpoint {
    pub schema_version: u16,
    pub checkpoint_id: String,
    pub enterprise_id: EnterpriseId,
    pub manifest_id: String,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_checkpoint_id: Option<String>,
    pub issued_at_ms: u64,
    pub batch_root: String,
    pub event_root: String,
    pub batch_count: u64,
    pub event_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger_commitment: Option<EnterpriseLedgerCommitment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_commitment: Option<EnterpriseSnapshotCommitment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_commitment_v2: Option<EnterpriseSnapshotCommitmentV2>,
    pub approvals: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct CheckpointContent<'a> {
    schema_version: u16,
    enterprise_id: &'a EnterpriseId,
    manifest_id: &'a str,
    sequence: u64,
    previous_checkpoint_id: &'a Option<String>,
    issued_at_ms: u64,
    batch_root: &'a str,
    event_root: &'a str,
    batch_count: u64,
    event_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    ledger_commitment: Option<&'a EnterpriseLedgerCommitment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_commitment: Option<&'a EnterpriseSnapshotCommitment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_commitment_v2: Option<&'a EnterpriseSnapshotCommitmentV2>,
}

impl EnterpriseLedgerCheckpoint {
    pub(super) fn content_id(&self) -> Result<String, String> {
        Ok(format!(
            "ledger-checkpoint:{}",
            canonical_digest(&CheckpointContent {
                schema_version: self.schema_version,
                enterprise_id: &self.enterprise_id,
                manifest_id: &self.manifest_id,
                sequence: self.sequence,
                previous_checkpoint_id: &self.previous_checkpoint_id,
                issued_at_ms: self.issued_at_ms,
                batch_root: &self.batch_root,
                event_root: &self.event_root,
                batch_count: self.batch_count,
                event_count: self.event_count,
                ledger_commitment: self.ledger_commitment.as_ref(),
                snapshot_commitment: self.snapshot_commitment.as_ref(),
                snapshot_commitment_v2: self.snapshot_commitment_v2.as_ref(),
            })?
        ))
    }

    fn validate_shape(&self) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_TRUST_SCHEMA_VERSION {
            return Err("unsupported enterprise ledger checkpoint schema".into());
        }
        if let Some(commitment) = &self.ledger_commitment {
            commitment.validate(&self.enterprise_id)?;
            validate_ledger_summary_compatibility(
                &EnterpriseLedgerSummary {
                    enterprise_id: self.enterprise_id.clone(),
                    batch_root: self.batch_root.clone(),
                    event_root: self.event_root.clone(),
                    batch_count: self.batch_count,
                    event_count: self.event_count,
                },
                commitment,
            )?;
        }
        if let Some(commitment) = &self.snapshot_commitment {
            commitment.validate(&self.enterprise_id)?;
        }
        if let Some(commitment) = &self.snapshot_commitment_v2 {
            commitment.validate(&self.enterprise_id)?;
        }
        if self.snapshot_commitment.is_some() && self.snapshot_commitment_v2.is_some() {
            return Err("ledger checkpoint contains both snapshot commitment versions".into());
        }
        if self.content_id()? != self.checkpoint_id {
            return Err("ledger checkpoint content digest mismatch".into());
        }
        validate_prefixed_digest(
            "ledger checkpoint",
            &self.checkpoint_id,
            "ledger-checkpoint:",
        )?;
        validate_prefixed_digest("trust manifest", &self.manifest_id, "trust-manifest:")?;
        if self.sequence == 0
            || (self.sequence == 1) != self.previous_checkpoint_id.is_none()
            || self.issued_at_ms == 0
        {
            return Err(
                "ledger checkpoint sequence, predecessor, or issued time is invalid".into(),
            );
        }
        if let Some(previous) = &self.previous_checkpoint_id {
            validate_prefixed_digest("previous ledger checkpoint", previous, "ledger-checkpoint:")?;
            if previous == &self.checkpoint_id {
                return Err("ledger checkpoint cannot name itself as predecessor".into());
            }
        }
        validate_digest("ledger batch root", &self.batch_root)?;
        validate_digest("ledger event root", &self.event_root)?;
        validate_approvals(&self.approvals)
    }
}

fn validate_ledger_summary_compatibility(
    summary: &EnterpriseLedgerSummary,
    commitment: &EnterpriseLedgerCommitment,
) -> Result<(), String> {
    if commitment.batch_count != summary.batch_count
        || commitment.event_count != summary.event_count
        || commitment.compatibility_batch_root(&summary.enterprise_id)? != summary.batch_root
        || commitment.compatibility_event_root(&summary.enterprise_id)? != summary.event_root
    {
        return Err("ledger commitment does not match the checkpoint compatibility summary".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseBatchInclusionReceipt {
    pub schema_version: u16,
    pub receipt_id: String,
    pub enterprise_id: EnterpriseId,
    pub manifest_id: String,
    pub checkpoint_id: String,
    pub checkpoint_sequence: u64,
    pub batch_id: EnterpriseBatchId,
    pub issued_at_ms: u64,
    pub approvals: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct InclusionContent<'a> {
    schema_version: u16,
    enterprise_id: &'a EnterpriseId,
    manifest_id: &'a str,
    checkpoint_id: &'a str,
    checkpoint_sequence: u64,
    batch_id: &'a EnterpriseBatchId,
    issued_at_ms: u64,
}

impl EnterpriseBatchInclusionReceipt {
    pub fn issue(
        manifest: &EnterpriseTrustManifest,
        checkpoint: &EnterpriseLedgerCheckpoint,
        batch: &EnterpriseBatch,
        issued_at_ms: u64,
        approvers: &[&EnterpriseSigningKey],
    ) -> Result<Self, String> {
        manifest.validate_shape()?;
        checkpoint.validate_shape()?;
        batch.validate()?;
        if checkpoint.enterprise_id != manifest.enterprise_id
            || batch.enterprise_id != manifest.enterprise_id
            || checkpoint.manifest_id != manifest.manifest_id
        {
            return Err("batch inclusion crosses an enterprise or trust manifest".into());
        }
        if issued_at_ms < checkpoint.issued_at_ms
            || issued_at_ms > manifest.expires_at_ms
            || issued_at_ms < manifest.issued_at_ms
        {
            return Err("batch inclusion issued time is outside its authenticated interval".into());
        }
        let mut value = Self {
            schema_version: ENTERPRISE_TRUST_SCHEMA_VERSION,
            receipt_id: String::new(),
            enterprise_id: manifest.enterprise_id.clone(),
            manifest_id: manifest.manifest_id.clone(),
            checkpoint_id: checkpoint.checkpoint_id.clone(),
            checkpoint_sequence: checkpoint.sequence,
            batch_id: batch.batch_id.clone(),
            issued_at_ms,
            approvals: BTreeMap::new(),
        };
        value.receipt_id = value.content_id()?;
        for key in approvers {
            let signer_id = key.signer_id();
            value.approvals.insert(
                signer_id.clone(),
                key.sign(&AuthTranscript {
                    kind: "batch_inclusion",
                    enterprise_id: value.enterprise_id.as_str(),
                    payload_id: &value.receipt_id,
                    manifest_id: &value.manifest_id,
                    grant_id: &value.checkpoint_id,
                    signer_id: &signer_id,
                }),
            );
        }
        value.validate_shape()?;
        Ok(value)
    }

    pub(super) fn content_id(&self) -> Result<String, String> {
        Ok(format!(
            "inclusion:{}",
            canonical_digest(&InclusionContent {
                schema_version: self.schema_version,
                enterprise_id: &self.enterprise_id,
                manifest_id: &self.manifest_id,
                checkpoint_id: &self.checkpoint_id,
                checkpoint_sequence: self.checkpoint_sequence,
                batch_id: &self.batch_id,
                issued_at_ms: self.issued_at_ms,
            })?
        ))
    }

    fn validate_shape(&self) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_TRUST_SCHEMA_VERSION {
            return Err("unsupported enterprise batch inclusion schema".into());
        }
        if self.content_id()? != self.receipt_id {
            return Err("batch inclusion content digest mismatch".into());
        }
        validate_prefixed_digest("batch inclusion", &self.receipt_id, "inclusion:")?;
        validate_prefixed_digest("trust manifest", &self.manifest_id, "trust-manifest:")?;
        validate_prefixed_digest(
            "ledger checkpoint",
            &self.checkpoint_id,
            "ledger-checkpoint:",
        )?;
        if self.checkpoint_sequence == 0 || self.issued_at_ms == 0 {
            return Err("batch inclusion sequence or issued time is invalid".into());
        }
        validate_approvals(&self.approvals)
    }
}

fn validate_approvals(approvals: &BTreeMap<String, String>) -> Result<(), String> {
    if approvals.len() > MAX_CHECKPOINT_APPROVALS {
        return Err("coordinator statement has too many approvals".into());
    }
    for (signer_id, signature) in approvals {
        validate_prefixed_digest("coordinator signer", signer_id, "signer:")?;
        validate_signature(signature)?;
    }
    Ok(())
}

fn validate_digest(label: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} must be a 64-character hexadecimal digest"));
    }
    Ok(())
}
