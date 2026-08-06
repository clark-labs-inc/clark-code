use agent_orchestration::EnterpriseId;
use rusqlite::{params, OptionalExtension, Transaction};
use scout_accumulator::{
    prove_persistent, verify_proof, AccumulatorContext, AccumulatorError, AccumulatorHead, Digest,
    InsertOutcome, PartitionedAccumulatorEditor, PartitionedAccumulatorHead,
    PartitionedAccumulatorUpdate, ProofStatus, StoredNode,
};

use super::database::{auth_mac, verify_mac, AUTH_KEY_BYTES};
use super::{LedgerAuthorityWork, LedgerHead};

const ACCUMULATOR_DOMAIN: &str = "clark.scout.enterprise-ledger";
const BATCH_NAMESPACE: &str = "batch";
const EVENT_NAMESPACE: &str = "event";
const PARTITION_BITS: u8 = 12;

#[derive(Clone, Copy)]
pub(super) enum Lane {
    Batch,
    Event,
}

impl Lane {
    fn as_str(self) -> &'static str {
        match self {
            Self::Batch => "batch",
            Self::Event => "event",
        }
    }

    fn namespace(self) -> &'static str {
        match self {
            Self::Batch => BATCH_NAMESPACE,
            Self::Event => EVENT_NAMESPACE,
        }
    }
}

pub(super) fn empty_heads(
    enterprise_id: &EnterpriseId,
) -> Result<(PartitionedAccumulatorHead, PartitionedAccumulatorHead), String> {
    Ok((
        PartitionedAccumulatorHead::empty(context(enterprise_id, Lane::Batch)?, PARTITION_BITS)
            .map_err(|error| error.to_string())?,
        PartitionedAccumulatorHead::empty(context(enterprise_id, Lane::Event)?, PARTITION_BITS)
            .map_err(|error| error.to_string())?,
    ))
}

pub(super) fn validate_heads(head: &LedgerHead) -> Result<(), String> {
    for (lane, accumulator, count) in [
        (Lane::Batch, &head.batch_accumulator, head.batch_count),
        (Lane::Event, &head.event_accumulator, head.event_count),
    ] {
        accumulator.validate().map_err(|error| error.to_string())?;
        if accumulator.context != context(&head.enterprise_id, lane)?
            || accumulator.partition_bits != PARTITION_BITS
            || accumulator.root.count != count
        {
            return Err("Scout ledger accumulator head metadata is inconsistent".into());
        }
    }
    Ok(())
}

pub(super) fn editor(
    head: PartitionedAccumulatorHead,
) -> Result<PartitionedAccumulatorEditor, String> {
    PartitionedAccumulatorEditor::new(head).map_err(|error| error.to_string())
}

pub(super) fn insert(
    transaction: &Transaction<'_>,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    lane: Lane,
    editor: &mut PartitionedAccumulatorEditor,
    object_id: &str,
    work: &mut LedgerAuthorityWork,
) -> Result<InsertOutcome, String> {
    let update = editor
        .insert(object_id, |partition, digest| {
            read_node(
                transaction,
                auth_key,
                enterprise_id,
                lane,
                partition,
                digest,
                work,
            )
        })
        .map_err(|error| error.to_string())?;
    persist_nodes(
        transaction,
        auth_key,
        enterprise_id,
        lane,
        editor,
        &update,
        work,
    )?;
    Ok(update.outcome)
}

fn read_node(
    transaction: &Transaction<'_>,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    lane: Lane,
    partition: u16,
    digest: Digest,
    work: &mut LedgerAuthorityWork,
) -> Result<Option<StoredNode>, AccumulatorError> {
    work.accumulator_node_lookups += 1;
    let row = transaction
        .query_row(
            "SELECT node_json, mac FROM ledger_accumulator_nodes
             WHERE lane = ?1 AND partition_id = ?2 AND node_digest = ?3",
            params![lane.as_str(), partition, digest.to_string()],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()
        .map_err(|error| AccumulatorError::Storage(error.to_string()))?;
    row.map(|(node_json, observed_mac)| {
        verify_mac(
            auth_key,
            "ledger-accumulator-node-v1",
            &(
                enterprise_id,
                lane.as_str(),
                partition,
                digest.to_string(),
                &node_json,
            ),
            &observed_mac,
        )
        .map_err(AccumulatorError::Storage)?;
        let node: StoredNode = serde_json::from_slice(&node_json)
            .map_err(|error| AccumulatorError::Storage(error.to_string()))?;
        let node_context =
            partition_context(enterprise_id, lane, partition).map_err(AccumulatorError::Storage)?;
        if node
            .digest(&node_context)
            .map_err(|error| AccumulatorError::Storage(error.to_string()))?
            != digest
        {
            return Err(AccumulatorError::Storage(
                "authenticated Scout accumulator node digest changed".into(),
            ));
        }
        Ok(node)
    })
    .transpose()
}

fn persist_nodes(
    transaction: &Transaction<'_>,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    lane: Lane,
    editor: &PartitionedAccumulatorEditor,
    update: &PartitionedAccumulatorUpdate,
    work: &mut LedgerAuthorityWork,
) -> Result<(), String> {
    let node_context = editor
        .head()
        .partition_context(update.partition)
        .map_err(|error| error.to_string())?;
    for node in &update.nodes {
        let digest = node
            .digest(&node_context)
            .map_err(|error| error.to_string())?;
        let node_json = serde_json::to_vec(node).map_err(|error| error.to_string())?;
        let mac = auth_mac(
            auth_key,
            "ledger-accumulator-node-v1",
            &(
                enterprise_id,
                lane.as_str(),
                update.partition,
                digest.to_string(),
                &node_json,
            ),
        )?;
        let written = transaction
            .execute(
                "INSERT OR IGNORE INTO ledger_accumulator_nodes
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    lane.as_str(),
                    update.partition,
                    digest.to_string(),
                    node_json,
                    mac
                ],
            )
            .map_err(|error| error.to_string())?;
        if written == 0 {
            let existing = read_node(
                transaction,
                auth_key,
                enterprise_id,
                lane,
                update.partition,
                digest,
                work,
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Scout accumulator node disappeared during insert".to_string())?;
            if existing != *node {
                return Err("Scout accumulator node digest collision".into());
            }
        } else {
            work.accumulator_nodes_written += 1;
        }
    }
    for digest in &update.obsolete_nodes {
        read_node(
            transaction,
            auth_key,
            enterprise_id,
            lane,
            update.partition,
            *digest,
            work,
        )
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Scout obsolete accumulator node is missing".to_string())?;
        let deleted = transaction
            .execute(
                "DELETE FROM ledger_accumulator_nodes
                 WHERE lane = ?1 AND partition_id = ?2 AND node_digest = ?3",
                params![lane.as_str(), update.partition, digest.to_string()],
            )
            .map_err(|error| error.to_string())?;
        if deleted != 1 {
            return Err("Scout obsolete accumulator node changed during deletion".into());
        }
        work.accumulator_nodes_deleted += 1;
    }
    Ok(())
}

fn context(enterprise_id: &EnterpriseId, lane: Lane) -> Result<AccumulatorContext, String> {
    AccumulatorContext::new(ACCUMULATOR_DOMAIN, enterprise_id.as_str(), lane.namespace())
        .map_err(|error| error.to_string())
}

fn partition_context(
    enterprise_id: &EnterpriseId,
    lane: Lane,
    partition: u16,
) -> Result<AccumulatorContext, String> {
    let head = PartitionedAccumulatorHead::empty(context(enterprise_id, lane)?, PARTITION_BITS)
        .map_err(|error| error.to_string())?;
    head.partition_context(partition)
        .map_err(|error| error.to_string())
}

pub(super) fn root_id(namespace: &str, head: &PartitionedAccumulatorHead) -> String {
    format!(
        "{namespace}:{}:{}:{}",
        head.root.partition_bits,
        head.root.count,
        head.root.digest.to_hex()
    )
}

pub(super) fn require_member(
    transaction: &Transaction<'_>,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    lane: Lane,
    head: &PartitionedAccumulatorHead,
    object_id: &str,
    work: &mut LedgerAuthorityWork,
) -> Result<(), String> {
    let partition = head
        .partition_for(object_id)
        .map_err(|error| error.to_string())?;
    let context = head
        .partition_context(partition)
        .map_err(|error| error.to_string())?;
    let partition_head = head
        .partitions()
        .get(&partition)
        .copied()
        .unwrap_or_else(|| AccumulatorHead::empty(&context));
    let proof = prove_persistent(&context, partition_head, object_id, |digest| {
        read_node(
            transaction,
            auth_key,
            enterprise_id,
            lane,
            partition,
            digest,
            work,
        )
    })
    .map_err(|error| error.to_string())?;
    if verify_proof(&partition_head.root, &proof).map_err(|error| error.to_string())?
        != ProofStatus::Member
    {
        return Err("Scout ledger successor accumulator is missing a committed identity".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use agent_orchestration::EnterpriseId;
    use rusqlite::TransactionBehavior;

    use super::*;
    use crate::ledger_authority::{database, LedgerAuthority, LedgerAuthorityWork};

    #[test]
    fn obsolete_path_nodes_are_deleted_from_live_storage() {
        let root = tempfile::tempdir().expect("temporary ledger");
        let enterprise_id = EnterpriseId::new("enterprise:node-gc").expect("enterprise");
        let authority = LedgerAuthority::open(root.path(), enterprise_id.clone(), "a".repeat(64))
            .expect("authority");
        let (batch_head, _) = empty_heads(&enterprise_id).expect("empty heads");
        let target_partition = batch_head.partition_for("member-0").expect("partition");
        let mut members = Vec::new();
        for candidate in 0..100_000 {
            let id = format!("member-{candidate}");
            if batch_head.partition_for(&id).expect("partition") == target_partition {
                members.push(id);
                if members.len() == 6 {
                    break;
                }
            }
        }
        assert_eq!(members.len(), 6);

        let mut connection = database::open_connection(root.path()).expect("database");
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("transaction");
        let mut editor = editor(batch_head).expect("editor");
        let mut work = LedgerAuthorityWork::default();
        for member in &members {
            assert_eq!(
                insert(
                    &transaction,
                    &authority.auth_key,
                    &enterprise_id,
                    Lane::Batch,
                    &mut editor,
                    member,
                    &mut work,
                )
                .expect("insert"),
                InsertOutcome::Inserted
            );
        }
        let live_nodes: usize = transaction
            .query_row(
                "SELECT COUNT(*) FROM ledger_accumulator_nodes WHERE lane = 'batch'",
                [],
                |row| row.get(0),
            )
            .expect("live nodes");
        assert_eq!(live_nodes, members.len() * 2 - 1);
        assert!(work.accumulator_nodes_deleted > 0);
        transaction.rollback().expect("rollback test mutation");
    }
}
