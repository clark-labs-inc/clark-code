use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_orchestration::{EnterpriseId, EnterpriseTrustChain};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use scout_accumulator::{
    plan_insert, prove_persistent, AccumulatorContext, AccumulatorError, AccumulatorHead,
    AccumulatorRoot, Proof, StoredNode,
};
use scout_ingest_protocol::{CoordinatorSigningKey, IngestReceipt, IngestRequest, ScoutTenantId};

use crate::database;

const COORDINATOR_SCHEMA_VERSION: u16 = 5;
const BATCH_ACCUMULATOR_NAMESPACE: &str = "accepted-batches";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnterprisePinStatus {
    pub tenant_id: ScoutTenantId,
    pub enterprise_id: EnterpriseId,
    pub anchor_manifest_id: String,
    pub trust_generation: usize,
    pub accepted_batches: u64,
    pub batch_accumulator_root: String,
    pub next_sequence: u64,
    pub last_receipt_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatchAccumulatorProof {
    pub tenant_id: ScoutTenantId,
    pub enterprise_id: EnterpriseId,
    pub root: AccumulatorRoot,
    pub proof: Proof,
}

#[derive(Clone)]
pub struct CoordinatorStore {
    pub(crate) root: PathBuf,
    signer: Arc<CoordinatorSigningKey>,
}

impl CoordinatorStore {
    pub fn open(root: impl AsRef<Path>, signer: CoordinatorSigningKey) -> Result<Self, String> {
        let store = Self {
            root: root.as_ref().to_path_buf(),
            signer: Arc::new(signer),
        };
        let connection = database::open(&store.root)?;
        let observed = connection
            .query_row(
                "SELECT schema_version, coordinator_id, coordinator_public_key
                 FROM coordinator_meta WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, u16>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let expected_id = store.signer.coordinator_id();
        let expected_key = store.signer.public_key_hex();
        match observed {
            Some((_, coordinator_id, coordinator_key))
                if coordinator_id != expected_id || coordinator_key != expected_key =>
            {
                return Err("Scout coordinator state is pinned to another signing identity".into())
            }
            Some((COORDINATOR_SCHEMA_VERSION, _, _)) => {}
            Some(_) => return Err("unsupported Scout coordinator schema version".into()),
            None => {
                connection
                    .execute(
                        "INSERT INTO coordinator_meta (
                             singleton, schema_version, coordinator_id, coordinator_public_key
                         ) VALUES (1, ?1, ?2, ?3)",
                        params![COORDINATOR_SCHEMA_VERSION, expected_id, expected_key],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(store)
    }

    pub fn coordinator_public_key(&self) -> String {
        self.signer.public_key_hex()
    }

    pub fn pin_enterprise(
        &self,
        tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        expected_anchor_manifest_id: &str,
        chain: &EnterpriseTrustChain,
    ) -> Result<EnterprisePinStatus, String> {
        chain.verify(enterprise_id)?;
        if chain.anchor_manifest_id != expected_anchor_manifest_id {
            return Err("enterprise trust chain does not match the administrator pin".into());
        }
        let mut connection = database::open(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let existing = read_pin_chain(&transaction, tenant_id, enterprise_id)?;
        match existing {
            Some(local) => {
                let accepted = accept_chain(&local, chain, enterprise_id)?;
                if accepted.manifests.len() > local.manifests.len() {
                    update_chain(&transaction, tenant_id, enterprise_id, &accepted)?;
                }
            }
            None => {
                let accumulator =
                    AccumulatorHead::empty(&batch_accumulator_context(tenant_id, enterprise_id)?);
                transaction
                    .execute(
                        "INSERT INTO enterprise_pins (
                             tenant_id, enterprise_id, anchor_manifest_id, trust_chain_json,
                             batch_accumulator_head_json, next_sequence,
                             last_issued_at_ms, last_receipt_id
                         ) VALUES (?1, ?2, ?3, ?4, ?5, 1, 0, NULL)",
                        params![
                            tenant_id.as_str(),
                            enterprise_id.as_str(),
                            expected_anchor_manifest_id,
                            serde_json::to_vec(chain).map_err(|error| error.to_string())?,
                            serde_json::to_vec(&accumulator).map_err(|error| error.to_string())?
                        ],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        transaction.commit().map_err(|error| error.to_string())?;
        self.status(tenant_id, enterprise_id)?
            .ok_or_else(|| "enterprise pin disappeared after commit".to_string())
    }

    pub fn ingest(
        &self,
        authorized_tenant_id: &ScoutTenantId,
        request: &IngestRequest,
        observed_at_ms: u64,
    ) -> Result<IngestReceipt, String> {
        let mut connection = database::open(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let receipt =
            self.ingest_transaction(&transaction, authorized_tenant_id, request, observed_at_ms)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(receipt)
    }

    pub(crate) fn ingest_transaction(
        &self,
        transaction: &Transaction<'_>,
        authorized_tenant_id: &ScoutTenantId,
        request: &IngestRequest,
        observed_at_ms: u64,
    ) -> Result<IngestReceipt, String> {
        request.validate()?;
        if &request.tenant_id != authorized_tenant_id {
            return Err("authenticated tenant does not match the Scout ingestion request".into());
        }
        if observed_at_ms == 0 {
            return Err("coordinator observation time must be positive".into());
        }
        let enterprise_id = &request.bundle.signed_batch.batch.enterprise_id;
        let tenant_id = authorized_tenant_id;
        let batch_id = &request.bundle.signed_batch.batch.batch_id;
        let envelope_sha256 = request.envelope_sha256()?;
        if let Some((stored_hash, receipt_bytes)) =
            read_existing_receipt(transaction, tenant_id, enterprise_id, batch_id.as_str())?
        {
            if stored_hash != envelope_sha256 {
                return Err(
                    "central ingestion already witnessed conflicting content for this batch id"
                        .into(),
                );
            }
            let receipt: IngestReceipt =
                serde_json::from_slice(&receipt_bytes).map_err(|error| error.to_string())?;
            receipt.verify(&self.signer.public_key_hex())?;
            return Ok(receipt);
        }
        let local = read_pin_chain(transaction, tenant_id, enterprise_id)?.ok_or_else(|| {
            "enterprise trust anchor has not been administratively pinned".to_string()
        })?;
        let accepted = accept_chain(&local, &request.bundle.trust_chain, enterprise_id)?;
        accepted.verify_signed_batch(request.bundle.signed_batch.clone())?;
        let (next_sequence, last_issued_at_ms, previous_receipt_id, accumulator_head) =
            read_sequence(transaction, tenant_id, enterprise_id)?;
        let accumulator_context = batch_accumulator_context(tenant_id, enterprise_id)?;
        let mutation = plan_insert(
            &accumulator_context,
            accumulator_head,
            batch_id.as_str(),
            |digest| {
                read_accumulator_node(transaction, tenant_id, enterprise_id, digest.to_string())
            },
        )
        .map_err(|error| error.to_string())?;
        if mutation.next.root.count != next_sequence {
            return Err("batch accumulator count disagrees with coordinator sequence".into());
        }
        persist_accumulator_nodes(
            transaction,
            tenant_id,
            enterprise_id,
            &accumulator_context,
            &mutation.nodes,
        )?;
        remove_obsolete_accumulator_nodes(
            transaction,
            tenant_id,
            enterprise_id,
            &mutation.obsolete_nodes,
        )?;
        let issued_at_ms = observed_at_ms.max(
            last_issued_at_ms
                .checked_add(1)
                .ok_or_else(|| "coordinator receipt time overflow".to_string())?,
        );
        let receipt = IngestReceipt::issue(
            tenant_id.clone(),
            enterprise_id.clone(),
            accepted.anchor_manifest_id.clone(),
            batch_id.clone(),
            envelope_sha256.clone(),
            mutation.next.root.digest.to_string(),
            mutation.next.root.count,
            next_sequence,
            issued_at_ms,
            previous_receipt_id,
            &self.signer,
        )?;
        let next = next_sequence
            .checked_add(1)
            .ok_or_else(|| "coordinator sequence overflow".to_string())?;
        transaction
            .execute(
                "INSERT INTO ingest_receipts (
                     tenant_id, enterprise_id, batch_id, envelope_sha256, bundle_json,
                     receipt_json, sequence, receipt_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    tenant_id.as_str(),
                    enterprise_id.as_str(),
                    batch_id.as_str(),
                    envelope_sha256,
                    serde_json::to_vec(&request.bundle).map_err(|error| error.to_string())?,
                    serde_json::to_vec(&receipt).map_err(|error| error.to_string())?,
                    next_sequence,
                    receipt.receipt_id
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE enterprise_pins SET
                     trust_chain_json = ?3, next_sequence = ?4,
                     last_issued_at_ms = ?5, last_receipt_id = ?6,
                     batch_accumulator_head_json = ?7
                 WHERE tenant_id = ?1 AND enterprise_id = ?2",
                params![
                    tenant_id.as_str(),
                    enterprise_id.as_str(),
                    serde_json::to_vec(&accepted).map_err(|error| error.to_string())?,
                    next,
                    issued_at_ms,
                    receipt.receipt_id,
                    serde_json::to_vec(&mutation.next).map_err(|error| error.to_string())?
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(receipt)
    }

    pub fn receipt(
        &self,
        tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        batch_id: &str,
    ) -> Result<Option<IngestReceipt>, String> {
        let connection = database::open(&self.root)?;
        let bytes = connection
            .query_row(
                "SELECT receipt_json FROM ingest_receipts
                 WHERE tenant_id = ?1 AND enterprise_id = ?2 AND batch_id = ?3",
                params![tenant_id.as_str(), enterprise_id.as_str(), batch_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        bytes
            .map(|value| {
                let receipt: IngestReceipt =
                    serde_json::from_slice(&value).map_err(|error| error.to_string())?;
                receipt.verify(&self.signer.public_key_hex())?;
                Ok(receipt)
            })
            .transpose()
    }

    pub fn batch_proof(
        &self,
        tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        batch_id: &str,
    ) -> Result<BatchAccumulatorProof, String> {
        let mut connection = database::open(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| error.to_string())?;
        let (_, _, _, head) = read_sequence(&transaction, tenant_id, enterprise_id)?;
        let context = batch_accumulator_context(tenant_id, enterprise_id)?;
        let proof = prove_persistent(&context, head, batch_id, |digest| {
            read_accumulator_node(&transaction, tenant_id, enterprise_id, digest.to_string())
        })
        .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(BatchAccumulatorProof {
            tenant_id: tenant_id.clone(),
            enterprise_id: enterprise_id.clone(),
            root: head.root,
            proof,
        })
    }

    pub fn status(
        &self,
        tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
    ) -> Result<Option<EnterprisePinStatus>, String> {
        let connection = database::open(&self.root)?;
        let row = connection
            .query_row(
                "SELECT p.anchor_manifest_id, p.trust_chain_json,
                        p.next_sequence, p.last_receipt_id, COUNT(r.batch_id),
                        p.batch_accumulator_head_json
                 FROM enterprise_pins p
                 LEFT JOIN ingest_receipts r
                   ON r.tenant_id = p.tenant_id AND r.enterprise_id = p.enterprise_id
                 WHERE p.tenant_id = ?1 AND p.enterprise_id = ?2
                 GROUP BY p.tenant_id, p.enterprise_id",
                params![tenant_id.as_str(), enterprise_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, u64>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        row.map(
            |(
                anchor_manifest_id,
                chain_bytes,
                next_sequence,
                last_receipt_id,
                accepted_batches,
                accumulator_bytes,
            )| {
                let chain: EnterpriseTrustChain =
                    serde_json::from_slice(&chain_bytes).map_err(|error| error.to_string())?;
                chain.verify(enterprise_id)?;
                let accumulator: AccumulatorHead = serde_json::from_slice(&accumulator_bytes)
                    .map_err(|error| error.to_string())?;
                accumulator
                    .validate(&batch_accumulator_context(tenant_id, enterprise_id)?)
                    .map_err(|error| error.to_string())?;
                if accumulator.root.count != accepted_batches
                    || next_sequence != accepted_batches.saturating_add(1)
                {
                    return Err("coordinator status disagrees with its batch accumulator".into());
                }
                Ok(EnterprisePinStatus {
                    tenant_id: tenant_id.clone(),
                    enterprise_id: enterprise_id.clone(),
                    anchor_manifest_id,
                    trust_generation: chain.manifests.len(),
                    accepted_batches,
                    batch_accumulator_root: accumulator.root.digest.to_string(),
                    next_sequence,
                    last_receipt_id,
                })
            },
        )
        .transpose()
    }
}

fn read_pin_chain(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
) -> Result<Option<EnterpriseTrustChain>, String> {
    let bytes = transaction
        .query_row(
            "SELECT trust_chain_json FROM enterprise_pins
             WHERE tenant_id = ?1 AND enterprise_id = ?2",
            params![tenant_id.as_str(), enterprise_id.as_str()],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    bytes
        .map(|value| serde_json::from_slice(&value).map_err(|error| error.to_string()))
        .transpose()
}

fn accept_chain(
    local: &EnterpriseTrustChain,
    incoming: &EnterpriseTrustChain,
    enterprise_id: &EnterpriseId,
) -> Result<EnterpriseTrustChain, String> {
    local.verify(enterprise_id)?;
    incoming.verify(enterprise_id)?;
    if incoming.anchor_manifest_id != local.anchor_manifest_id {
        return Err("incoming trust chain does not match the pinned enterprise anchor".into());
    }
    let common = incoming.manifests.len().min(local.manifests.len());
    if incoming.manifests[..common] != local.manifests[..common] {
        return Err("enterprise trust fork detected; refusing automatic resolution".into());
    }
    Ok(if incoming.manifests.len() > local.manifests.len() {
        incoming.clone()
    } else {
        local.clone()
    })
}

fn update_chain(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    chain: &EnterpriseTrustChain,
) -> Result<(), String> {
    transaction
        .execute(
            "UPDATE enterprise_pins SET trust_chain_json = ?3
             WHERE tenant_id = ?1 AND enterprise_id = ?2",
            params![
                tenant_id.as_str(),
                enterprise_id.as_str(),
                serde_json::to_vec(chain).map_err(|error| error.to_string())?
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn read_existing_receipt(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    batch_id: &str,
) -> Result<Option<(String, Vec<u8>)>, String> {
    transaction
        .query_row(
            "SELECT envelope_sha256, receipt_json FROM ingest_receipts
             WHERE tenant_id = ?1 AND enterprise_id = ?2 AND batch_id = ?3",
            params![tenant_id.as_str(), enterprise_id.as_str(), batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn read_sequence(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
) -> Result<(u64, u64, Option<String>, AccumulatorHead), String> {
    transaction
        .query_row(
            "SELECT next_sequence, last_issued_at_ms, last_receipt_id,
                    batch_accumulator_head_json
             FROM enterprise_pins
             WHERE tenant_id = ?1 AND enterprise_id = ?2",
            params![tenant_id.as_str(), enterprise_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            },
        )
        .map_err(|error| error.to_string())
        .and_then(|(next, time, previous, bytes)| {
            serde_json::from_slice(&bytes)
                .map(|head| (next, time, previous, head))
                .map_err(|error| error.to_string())
        })
}

fn batch_accumulator_context(
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
) -> Result<AccumulatorContext, String> {
    AccumulatorContext::new(
        format!(
            "clark.scout.central-ingestion/tenant/{}",
            tenant_id.as_str()
        ),
        enterprise_id.as_str(),
        BATCH_ACCUMULATOR_NAMESPACE,
    )
    .map_err(|error| error.to_string())
}

fn read_accumulator_node(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    digest: String,
) -> Result<Option<StoredNode>, AccumulatorError> {
    let bytes = transaction
        .query_row(
            "SELECT node_json FROM accumulator_nodes
             WHERE tenant_id = ?1 AND enterprise_id = ?2
               AND namespace = ?3 AND node_digest = ?4",
            params![
                tenant_id.as_str(),
                enterprise_id.as_str(),
                BATCH_ACCUMULATOR_NAMESPACE,
                digest
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(|error| AccumulatorError::Storage(error.to_string()))?;
    bytes
        .map(|value| {
            serde_json::from_slice(&value)
                .map_err(|error| AccumulatorError::Storage(error.to_string()))
        })
        .transpose()
}

fn persist_accumulator_nodes(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    context: &AccumulatorContext,
    nodes: &[StoredNode],
) -> Result<(), String> {
    for node in nodes {
        let digest = node.digest(context).map_err(|error| error.to_string())?;
        let encoded = serde_json::to_vec(node).map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO accumulator_nodes (
                     tenant_id, enterprise_id, namespace, node_digest, node_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    tenant_id.as_str(),
                    enterprise_id.as_str(),
                    BATCH_ACCUMULATOR_NAMESPACE,
                    digest.to_string(),
                    encoded
                ],
            )
            .map_err(|error| error.to_string())?;
        let observed = transaction
            .query_row(
                "SELECT node_json FROM accumulator_nodes
                 WHERE tenant_id = ?1 AND enterprise_id = ?2
                   AND namespace = ?3 AND node_digest = ?4",
                params![
                    tenant_id.as_str(),
                    enterprise_id.as_str(),
                    BATCH_ACCUMULATOR_NAMESPACE,
                    digest.to_string()
                ],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map_err(|error| error.to_string())?;
        if observed != encoded {
            return Err("content-addressed accumulator node conflicts with stored content".into());
        }
    }
    Ok(())
}

fn remove_obsolete_accumulator_nodes(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    digests: &[scout_accumulator::Digest],
) -> Result<(), String> {
    for digest in digests {
        let removed = transaction
            .execute(
                "DELETE FROM accumulator_nodes
                 WHERE tenant_id = ?1 AND enterprise_id = ?2
                   AND namespace = ?3 AND node_digest = ?4",
                params![
                    tenant_id.as_str(),
                    enterprise_id.as_str(),
                    BATCH_ACCUMULATOR_NAMESPACE,
                    digest.to_string()
                ],
            )
            .map_err(|error| error.to_string())?;
        if removed != 1 {
            return Err("obsolete accumulator path node was not stored exactly once".into());
        }
    }
    Ok(())
}
