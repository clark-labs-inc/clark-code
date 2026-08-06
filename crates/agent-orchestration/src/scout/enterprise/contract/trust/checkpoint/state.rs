use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    EnterpriseBatchInclusionReceipt, EnterpriseLedgerCheckpoint, EnterpriseLedgerCommitment,
    EnterpriseLedgerSummary, EnterpriseSnapshotCommitmentV2,
};
use crate::scout::enterprise::contract::trust::crypto::{verify_signature, AuthTranscript};
use crate::scout::enterprise::contract::trust::model::{
    EnterpriseTrustChain, EnterpriseTrustManifest,
};
use crate::scout::enterprise::contract::{EnterpriseBatch, EnterpriseId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedEnterpriseCheckpoint {
    checkpoint: EnterpriseLedgerCheckpoint,
    manifest_generation: u64,
}

impl VerifiedEnterpriseCheckpoint {
    pub fn checkpoint(&self) -> &EnterpriseLedgerCheckpoint {
        &self.checkpoint
    }

    pub fn manifest_generation(&self) -> u64 {
        self.manifest_generation
    }

    pub fn ledger_commitment(&self) -> Option<&EnterpriseLedgerCommitment> {
        self.checkpoint.ledger_commitment.as_ref()
    }

    pub fn ledger_generation(&self) -> Option<u64> {
        self.ledger_commitment()
            .map(|commitment| commitment.generation)
    }

    pub fn enterprise_ledger_root_v2(&self) -> Option<&str> {
        self.ledger_commitment()
            .map(|commitment| commitment.enterprise_ledger_root_v2.as_str())
    }

    pub fn snapshot_commitment_v2(&self) -> Option<&EnterpriseSnapshotCommitmentV2> {
        self.checkpoint.snapshot_commitment_v2.as_ref()
    }

    pub fn verify_ledger_commitment(
        &self,
        observed: &EnterpriseLedgerCommitment,
    ) -> Result<(), String> {
        observed.validate(&self.checkpoint.enterprise_id)?;
        if self.ledger_commitment() != Some(observed) {
            return Err(
                "local ledger commitment does not match the authenticated checkpoint".into(),
            );
        }
        Ok(())
    }

    pub fn verify_batches<'a>(
        &self,
        batches: impl IntoIterator<Item = &'a EnterpriseBatch>,
    ) -> Result<(), String> {
        if let Some(expected) = self.ledger_commitment() {
            let observed = EnterpriseLedgerCommitment::from_batches(
                &self.checkpoint.enterprise_id,
                expected.generation,
                batches,
            )?;
            return self.verify_ledger_commitment(&observed);
        }
        let observed =
            EnterpriseLedgerSummary::from_batches(self.checkpoint.enterprise_id.clone(), batches)?;
        let expected = EnterpriseLedgerSummary {
            enterprise_id: self.checkpoint.enterprise_id.clone(),
            batch_root: self.checkpoint.batch_root.clone(),
            event_root: self.checkpoint.event_root.clone(),
            batch_count: self.checkpoint.batch_count,
            event_count: self.checkpoint.event_count,
        };
        if observed != expected {
            return Err("local ledger does not match the authenticated checkpoint".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedEnterpriseInclusion {
    receipt: EnterpriseBatchInclusionReceipt,
}

impl VerifiedEnterpriseInclusion {
    pub fn receipt(&self) -> &EnterpriseBatchInclusionReceipt {
        &self.receipt
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnterpriseCheckpointObservation {
    Advanced,
    Duplicate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseCheckpointCursor {
    enterprise_id: EnterpriseId,
    highest_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    highest_checkpoint_id: Option<String>,
    highest_manifest_generation: u64,
    highest_issued_at_ms: u64,
}

impl EnterpriseCheckpointCursor {
    pub fn new(enterprise_id: EnterpriseId) -> Self {
        Self {
            enterprise_id,
            highest_sequence: 0,
            highest_checkpoint_id: None,
            highest_manifest_generation: 0,
            highest_issued_at_ms: 0,
        }
    }

    pub fn highest_sequence(&self) -> u64 {
        self.highest_sequence
    }

    pub fn highest_checkpoint_id(&self) -> Option<&str> {
        self.highest_checkpoint_id.as_deref()
    }

    pub fn observe(
        &mut self,
        verified: &VerifiedEnterpriseCheckpoint,
    ) -> Result<EnterpriseCheckpointObservation, String> {
        let checkpoint = verified.checkpoint();
        if checkpoint.enterprise_id != self.enterprise_id {
            return Err("checkpoint cursor belongs to another enterprise".into());
        }
        if checkpoint.sequence < self.highest_sequence {
            return Err("ledger checkpoint rollback detected".into());
        }
        if checkpoint.sequence == self.highest_sequence && self.highest_sequence != 0 {
            if self.highest_checkpoint_id.as_deref() == Some(&checkpoint.checkpoint_id) {
                if self.highest_manifest_generation != verified.manifest_generation
                    || self.highest_issued_at_ms != checkpoint.issued_at_ms
                {
                    return Err(
                        "persisted checkpoint cursor does not match its authenticated checkpoint"
                            .into(),
                    );
                }
                return Ok(EnterpriseCheckpointObservation::Duplicate);
            }
            return Err("conflicting ledger checkpoints share one sequence".into());
        }
        if self.highest_sequence == 0 {
            if checkpoint.sequence != 1 || checkpoint.previous_checkpoint_id.is_some() {
                return Err("checkpoint cursor must begin at sequence one".into());
            }
        } else {
            if checkpoint.sequence != self.highest_sequence + 1
                || checkpoint.previous_checkpoint_id.as_deref()
                    != self.highest_checkpoint_id.as_deref()
            {
                return Err("ledger checkpoint chain skips or names the wrong predecessor".into());
            }
            if verified.manifest_generation < self.highest_manifest_generation {
                return Err("ledger checkpoint rolls back the highest-seen trust manifest".into());
            }
            if checkpoint.issued_at_ms <= self.highest_issued_at_ms {
                return Err("ledger checkpoint authoritative time is not monotonic".into());
            }
        }
        self.highest_sequence = checkpoint.sequence;
        self.highest_checkpoint_id = Some(checkpoint.checkpoint_id.clone());
        self.highest_manifest_generation = verified.manifest_generation;
        self.highest_issued_at_ms = checkpoint.issued_at_ms;
        Ok(EnterpriseCheckpointObservation::Advanced)
    }
}

impl EnterpriseTrustChain {
    pub fn verify_ledger_checkpoint(
        &self,
        checkpoint: EnterpriseLedgerCheckpoint,
    ) -> Result<VerifiedEnterpriseCheckpoint, String> {
        checkpoint.validate_shape()?;
        let current = self.verify(&checkpoint.enterprise_id)?;
        let manifest = self
            .manifests
            .iter()
            .find(|manifest| manifest.manifest_id == checkpoint.manifest_id)
            .ok_or_else(|| {
                "ledger checkpoint references a manifest outside the pinned chain".to_string()
            })?;
        verify_coordinator_approvals(
            manifest,
            current,
            checkpoint.issued_at_ms,
            "ledger_checkpoint",
            &checkpoint.checkpoint_id,
            "",
            &checkpoint.approvals,
        )?;
        Ok(VerifiedEnterpriseCheckpoint {
            checkpoint,
            manifest_generation: manifest.generation,
        })
    }

    pub fn verify_batch_inclusion(
        &self,
        checkpoint: &VerifiedEnterpriseCheckpoint,
        receipt: EnterpriseBatchInclusionReceipt,
        batch: &EnterpriseBatch,
    ) -> Result<VerifiedEnterpriseInclusion, String> {
        receipt.validate_shape()?;
        batch.validate()?;
        let authoritative = checkpoint.checkpoint();
        if receipt.enterprise_id != authoritative.enterprise_id
            || receipt.manifest_id != authoritative.manifest_id
            || receipt.checkpoint_id != authoritative.checkpoint_id
            || receipt.checkpoint_sequence != authoritative.sequence
            || receipt.batch_id != batch.batch_id
            || batch.enterprise_id != authoritative.enterprise_id
        {
            return Err("batch inclusion does not match its checkpoint or batch".into());
        }
        if receipt.issued_at_ms < authoritative.issued_at_ms {
            return Err("batch inclusion predates its authenticated checkpoint".into());
        }
        let current = self.verify(&receipt.enterprise_id)?;
        let manifest = self
            .manifests
            .iter()
            .find(|manifest| manifest.manifest_id == receipt.manifest_id)
            .ok_or_else(|| {
                "batch inclusion references a manifest outside the pinned chain".to_string()
            })?;
        verify_coordinator_approvals(
            manifest,
            current,
            receipt.issued_at_ms,
            "batch_inclusion",
            &receipt.receipt_id,
            &receipt.checkpoint_id,
            &receipt.approvals,
        )?;
        Ok(VerifiedEnterpriseInclusion { receipt })
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_coordinator_approvals(
    manifest: &EnterpriseTrustManifest,
    current: &EnterpriseTrustManifest,
    issued_at_ms: u64,
    kind: &str,
    payload_id: &str,
    context_id: &str,
    approvals: &BTreeMap<String, String>,
) -> Result<(), String> {
    if !(manifest.issued_at_ms..=manifest.expires_at_ms).contains(&issued_at_ms) {
        return Err("coordinator statement falls outside its manifest validity interval".into());
    }
    let mut valid = 0_usize;
    for (signer_id, signature) in approvals {
        let Some(public_key) = manifest.coordinators.get(signer_id) else {
            continue;
        };
        let revoked_at_issue = current
            .revoked_signer_ids
            .get(signer_id)
            .is_some_and(|effective| issued_at_ms >= *effective);
        if revoked_at_issue {
            continue;
        }
        verify_signature(
            public_key,
            signature,
            &AuthTranscript {
                kind,
                enterprise_id: manifest.enterprise_id.as_str(),
                payload_id,
                manifest_id: &manifest.manifest_id,
                grant_id: context_id,
                signer_id,
            },
        )?;
        valid += 1;
    }
    if valid < usize::from(manifest.coordinator_threshold) {
        return Err(format!(
            "authenticated coordinator threshold not met: {valid}/{}",
            manifest.coordinator_threshold
        ));
    }
    Ok(())
}
