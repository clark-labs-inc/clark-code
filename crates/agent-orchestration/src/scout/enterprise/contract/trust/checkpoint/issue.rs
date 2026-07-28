use std::collections::BTreeMap;

use super::{
    validate_ledger_summary_compatibility, EnterpriseLedgerCheckpoint, EnterpriseLedgerCommitment,
    EnterpriseLedgerSummary, EnterpriseSnapshotCommitment, EnterpriseSnapshotCommitmentV2,
};
use crate::scout::enterprise::contract::trust::crypto::{AuthTranscript, EnterpriseSigningKey};
use crate::scout::enterprise::contract::trust::model::{
    EnterpriseTrustManifest, ENTERPRISE_TRUST_SCHEMA_VERSION,
};

impl EnterpriseLedgerCheckpoint {
    pub fn issue(
        manifest: &EnterpriseTrustManifest,
        sequence: u64,
        previous_checkpoint_id: Option<String>,
        issued_at_ms: u64,
        summary: &EnterpriseLedgerSummary,
        snapshot_commitment: Option<EnterpriseSnapshotCommitment>,
        approvers: &[&EnterpriseSigningKey],
    ) -> Result<Self, String> {
        Self::issue_inner(
            manifest,
            sequence,
            previous_checkpoint_id,
            issued_at_ms,
            summary,
            None,
            snapshot_commitment,
            None,
            approvers,
        )
    }

    pub fn issue_v2(
        manifest: &EnterpriseTrustManifest,
        sequence: u64,
        previous_checkpoint_id: Option<String>,
        issued_at_ms: u64,
        ledger_commitment: EnterpriseLedgerCommitment,
        snapshot_commitment_v2: Option<EnterpriseSnapshotCommitmentV2>,
        approvers: &[&EnterpriseSigningKey],
    ) -> Result<Self, String> {
        ledger_commitment.validate(&manifest.enterprise_id)?;
        let summary = EnterpriseLedgerSummary {
            enterprise_id: manifest.enterprise_id.clone(),
            batch_root: ledger_commitment.compatibility_batch_root(&manifest.enterprise_id)?,
            event_root: ledger_commitment.compatibility_event_root(&manifest.enterprise_id)?,
            batch_count: ledger_commitment.batch_count,
            event_count: ledger_commitment.event_count,
        };
        Self::issue_inner(
            manifest,
            sequence,
            previous_checkpoint_id,
            issued_at_ms,
            &summary,
            Some(ledger_commitment),
            None,
            snapshot_commitment_v2,
            approvers,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn issue_with_commitments(
        manifest: &EnterpriseTrustManifest,
        sequence: u64,
        previous_checkpoint_id: Option<String>,
        issued_at_ms: u64,
        summary: &EnterpriseLedgerSummary,
        ledger_commitment: Option<EnterpriseLedgerCommitment>,
        snapshot_commitment: Option<EnterpriseSnapshotCommitment>,
        approvers: &[&EnterpriseSigningKey],
    ) -> Result<Self, String> {
        Self::issue_inner(
            manifest,
            sequence,
            previous_checkpoint_id,
            issued_at_ms,
            summary,
            ledger_commitment,
            snapshot_commitment,
            None,
            approvers,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn issue_inner(
        manifest: &EnterpriseTrustManifest,
        sequence: u64,
        previous_checkpoint_id: Option<String>,
        issued_at_ms: u64,
        summary: &EnterpriseLedgerSummary,
        ledger_commitment: Option<EnterpriseLedgerCommitment>,
        snapshot_commitment: Option<EnterpriseSnapshotCommitment>,
        snapshot_commitment_v2: Option<EnterpriseSnapshotCommitmentV2>,
        approvers: &[&EnterpriseSigningKey],
    ) -> Result<Self, String> {
        manifest.validate_shape()?;
        summary.validate()?;
        if let Some(commitment) = &ledger_commitment {
            commitment.validate(&summary.enterprise_id)?;
            validate_ledger_summary_compatibility(summary, commitment)?;
        }
        if let Some(commitment) = &snapshot_commitment {
            commitment.validate(&summary.enterprise_id)?;
        }
        if let Some(commitment) = &snapshot_commitment_v2 {
            commitment.validate(&summary.enterprise_id)?;
        }
        if summary.enterprise_id != manifest.enterprise_id {
            return Err("ledger summary belongs to another enterprise".into());
        }
        if !(manifest.issued_at_ms..=manifest.expires_at_ms).contains(&issued_at_ms) {
            return Err("ledger checkpoint falls outside its manifest validity interval".into());
        }
        let mut value = Self {
            schema_version: ENTERPRISE_TRUST_SCHEMA_VERSION,
            checkpoint_id: String::new(),
            enterprise_id: summary.enterprise_id.clone(),
            manifest_id: manifest.manifest_id.clone(),
            sequence,
            previous_checkpoint_id,
            issued_at_ms,
            batch_root: summary.batch_root.clone(),
            event_root: summary.event_root.clone(),
            batch_count: summary.batch_count,
            event_count: summary.event_count,
            ledger_commitment,
            snapshot_commitment,
            snapshot_commitment_v2,
            approvals: BTreeMap::new(),
        };
        value.checkpoint_id = value.content_id()?;
        for key in approvers {
            let signer_id = key.signer_id();
            value.approvals.insert(
                signer_id.clone(),
                key.sign(&AuthTranscript {
                    kind: "ledger_checkpoint",
                    enterprise_id: value.enterprise_id.as_str(),
                    payload_id: &value.checkpoint_id,
                    manifest_id: &value.manifest_id,
                    grant_id: "",
                    signer_id: &signer_id,
                }),
            );
        }
        value.validate_shape()?;
        Ok(value)
    }
}
