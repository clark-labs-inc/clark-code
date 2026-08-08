use std::path::Path;

use agent_orchestration::EnterpriseId;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::read_pinned_chain;
use crate::ledger_authority::{LedgerAuthority, LedgerAuthorityWork, LedgerHead};

pub(crate) struct OpenLedger {
    pub(crate) authority: LedgerAuthority,
    pub(crate) head: LedgerHead,
    pub(crate) work: LedgerAuthorityWork,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectionLedgerCursor {
    pub(super) generation: u64,
    pub(super) head_id: String,
    pub(super) previous_head_id: Option<String>,
    pub(super) trust_chain_digest: String,
    pub(super) batch_count: u64,
    pub(super) event_count: u64,
    pub(super) batch_set_root_v1: String,
    pub(super) event_set_root_v1: String,
}

impl ProjectionLedgerCursor {
    pub(super) fn from_head(head: &LedgerHead) -> Self {
        Self {
            generation: head.generation,
            head_id: head.head_id.clone(),
            previous_head_id: head.previous_head_id.clone(),
            trust_chain_digest: head.trust_chain_digest.clone(),
            batch_count: head.batch_count,
            event_count: head.event_count,
            batch_set_root_v1: root_id("scout-batch-set-v1", &head.batch_accumulator),
            event_set_root_v1: root_id("scout-event-set-v1", &head.event_accumulator),
        }
    }

    pub(super) fn is_direct_successor_of(&self, previous: &Self) -> bool {
        self.generation == previous.generation.saturating_add(1)
            && self.batch_count == previous.batch_count.saturating_add(1)
            && self.previous_head_id.as_deref() == Some(previous.head_id.as_str())
            && self.trust_chain_digest == previous.trust_chain_digest
            && self.head_id != previous.head_id
    }
}

pub(crate) fn commitment(
    root: &Path,
    enterprise_id: &EnterpriseId,
) -> Result<agent_orchestration::EnterpriseLedgerCommitment, String> {
    open(root, enterprise_id)?.head.ledger_commitment()
}

pub(crate) fn open(root: &Path, enterprise_id: &EnterpriseId) -> Result<OpenLedger, String> {
    let (chain, _) = read_pinned_chain(root, enterprise_id)?;
    // The authority is pinned to the immutable target-private anchor, not to
    // the serialized evolving chain. Legitimate grant/revocation manifests
    // must not make an existing enterprise ledger impossible to open.
    let trust_chain_digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                "scout-ledger-authority-anchor-v1",
                enterprise_id.as_str(),
                &chain.anchor_manifest_id,
            ))
            .map_err(|error| error.to_string())?
        )
    );
    let opened = LedgerAuthority::open_with_head(root, enterprise_id.clone(), trust_chain_digest)?;
    Ok(OpenLedger {
        authority: opened.authority,
        head: opened.head,
        work: opened.work,
    })
}

fn root_id(namespace: &str, head: &scout_accumulator::PartitionedAccumulatorHead) -> String {
    format!(
        "{namespace}:{}:{}:{}",
        head.root.partition_bits,
        head.root.count,
        head.root.digest.to_hex()
    )
}
