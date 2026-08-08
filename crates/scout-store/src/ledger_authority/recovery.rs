use std::collections::BTreeSet;
use std::path::Path;

use agent_orchestration::EnterpriseId;
use rusqlite::TransactionBehavior;

use super::accumulator::{self, Lane};
use super::database::{self, AUTH_KEY_BYTES};
use super::seal::{self, StorageSeal};
use super::LedgerAuthorityWork;

pub(super) fn recover_exact_successor(
    root: &Path,
    auth_key: &[u8; AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    trust_chain_digest: &str,
    sealed: &StorageSeal,
) -> Result<(), String> {
    let mut connection = database::open_connection(root)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| error.to_string())?;
    let mut work = LedgerAuthorityWork::default();
    let current = database::read_head(
        &transaction,
        auth_key,
        enterprise_id,
        trust_chain_digest,
        &mut work,
    )?;
    let expected_generation = sealed
        .generation()
        .checked_add(1)
        .ok_or_else(|| "sealed Scout ledger generation overflow".to_string())?;
    if current.generation != expected_generation {
        return Err(
            "Scout ledger recovery requires exactly one committed successor generation".into(),
        );
    }
    if current.previous_head_id.as_deref() != Some(sealed.head_id()) {
        return Err("Scout ledger successor does not extend the sealed head".into());
    }
    let previous = database::read_head_history_by_id(
        &transaction,
        auth_key,
        enterprise_id,
        trust_chain_digest,
        sealed.head_id(),
    )?
    .ok_or_else(|| "sealed Scout ledger head is missing from authenticated history".to_string())?;
    if previous.generation != sealed.generation() {
        return Err("sealed Scout ledger history generation mismatch".into());
    }
    let successors = database::read_head_history_generation(
        &transaction,
        auth_key,
        enterprise_id,
        trust_chain_digest,
        expected_generation,
    )?;
    if successors.len() != 1 || successors[0] != current {
        return Err("Scout ledger recovery found a missing or forked successor".into());
    }
    if current.batch_count != previous.batch_count.saturating_add(1) {
        return Err("Scout ledger successor batch count is not a single append".into());
    }

    let generations = database::read_batch_range(
        &transaction,
        auth_key,
        enterprise_id,
        expected_generation,
        expected_generation,
        &mut work,
    )?;
    let successor = generations
        .into_iter()
        .next()
        .ok_or_else(|| "Scout ledger successor payload row is missing".to_string())?;
    let batch = &successor.envelope.batch;
    batch.validate()?;
    accumulator::require_member(
        &transaction,
        auth_key,
        enterprise_id,
        Lane::Batch,
        &current.batch_accumulator,
        batch.batch_id.as_str(),
        &mut work,
    )?;

    let mut newly_inserted = BTreeSet::new();
    for event in &batch.events {
        let encoded = serde_json::to_vec(event).map_err(|error| error.to_string())?;
        let stored = database::read_event(
            &transaction,
            auth_key,
            enterprise_id,
            event.event_id.as_str(),
            &mut work,
        )?
        .ok_or_else(|| "Scout ledger successor event identity row is missing".to_string())?;
        if stored.event_json != encoded {
            return Err("Scout ledger successor event identity collision".into());
        }
        if stored.first_batch_id == batch.batch_id.as_str() {
            newly_inserted.insert(event.event_id.as_str().to_owned());
        }
        accumulator::require_member(
            &transaction,
            auth_key,
            enterprise_id,
            Lane::Event,
            &current.event_accumulator,
            event.event_id.as_str(),
            &mut work,
        )?;
    }
    let indexed_new = database::read_event_ids_for_first_batch(
        &transaction,
        auth_key,
        enterprise_id,
        batch.batch_id.as_str(),
        &mut work,
    )?;
    if indexed_new != newly_inserted {
        return Err("Scout ledger successor has unreferenced event identity rows".into());
    }
    if current.event_count
        != previous
            .event_count
            .checked_add(newly_inserted.len() as u64)
            .ok_or_else(|| "Scout ledger successor event count overflow".to_string())?
    {
        return Err("Scout ledger successor event count does not match its payload".into());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    seal::write(root, auth_key, &current)
}
