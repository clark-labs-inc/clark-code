use agent_orchestration::VerifiedEnterpriseBatch;
use fs2::FileExt;
use rusqlite::TransactionBehavior;
use scout_accumulator::InsertOutcome;

use super::accumulator::{self, Lane};
use super::database;
use super::{
    LedgerAppendOutcome, LedgerAppendReceipt, LedgerAuthority, LedgerAuthorityWork, LedgerHead,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AppendFailpoint {
    None,
    #[cfg(test)]
    AfterPayloadRows,
    #[cfg(test)]
    AfterHeadWrite,
    #[cfg(test)]
    AfterCommitBeforeSeal,
}

pub(super) fn append(
    authority: &LedgerAuthority,
    verified: &VerifiedEnterpriseBatch,
    _failpoint: AppendFailpoint,
) -> Result<LedgerAppendReceipt, String> {
    let envelope = verified.envelope();
    if envelope.batch.enterprise_id != authority.enterprise_id {
        return Err("verified ledger batch belongs to another enterprise".into());
    }
    envelope.batch.validate()?;
    let lock = database::open_lock(&authority.root)?;
    FileExt::lock_exclusive(&lock).map_err(|error| error.to_string())?;
    let sealed = super::seal::validate(
        &authority.root,
        &authority.auth_key,
        &authority.enterprise_id,
    )?;
    let envelope_json = serde_json::to_vec(envelope).map_err(|error| error.to_string())?;
    let mut connection = database::open_connection(&authority.root)?;
    super::seal::validate_unchanged(
        &authority.root,
        &authority.auth_key,
        &authority.enterprise_id,
        &sealed,
    )?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let mut work = LedgerAuthorityWork::default();
    let head = database::read_head(
        &transaction,
        &authority.auth_key,
        &authority.enterprise_id,
        &authority.trust_chain_digest,
        &mut work,
    )?;
    super::seal::require_head(&sealed, &head)?;
    let existing = database::read_batch(
        &transaction,
        &authority.auth_key,
        &authority.enterprise_id,
        envelope.batch.batch_id.as_str(),
        &mut work,
    )?;
    if let Some(existing) = existing {
        if existing.envelope_json != envelope_json
            || existing.envelope_sha256 != database::sha256_hex(&envelope_json)
            || existing.event_count != envelope.batch.events.len() as u64
        {
            return Err("ledger batch id already exists with different signed bytes".into());
        }
        let decoded = existing.decode()?;
        if decoded != *envelope {
            return Err("ledger batch id already exists with different signed content".into());
        }
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(LedgerAppendReceipt {
            outcome: LedgerAppendOutcome::AlreadyPresent,
            head,
            work,
        });
    }

    let next_generation = head
        .generation
        .checked_add(1)
        .ok_or_else(|| "Scout ledger generation overflow".to_string())?;
    let mut batch_editor = accumulator::editor(head.batch_accumulator.clone())?;
    if accumulator::insert(
        &transaction,
        &authority.auth_key,
        &authority.enterprise_id,
        Lane::Batch,
        &mut batch_editor,
        envelope.batch.batch_id.as_str(),
        &mut work,
    )? != InsertOutcome::Inserted
    {
        return Err("Scout ledger batch row is missing from an existing accumulator member".into());
    }

    let mut event_editor = accumulator::editor(head.event_accumulator.clone())?;
    let mut inserted_events = 0_u64;
    for event in &envelope.batch.events {
        let event_json = serde_json::to_vec(event).map_err(|error| error.to_string())?;
        let existing_event = database::read_event(
            &transaction,
            &authority.auth_key,
            &authority.enterprise_id,
            event.event_id.as_str(),
            &mut work,
        )?;
        let outcome = accumulator::insert(
            &transaction,
            &authority.auth_key,
            &authority.enterprise_id,
            Lane::Event,
            &mut event_editor,
            event.event_id.as_str(),
            &mut work,
        )?;
        match (existing_event, outcome) {
            (None, InsertOutcome::Inserted) => {
                database::insert_event(
                    &transaction,
                    &authority.auth_key,
                    &authority.enterprise_id,
                    event.event_id.as_str(),
                    envelope.batch.batch_id.as_str(),
                    &event_json,
                )?;
                inserted_events = inserted_events
                    .checked_add(1)
                    .ok_or_else(|| "Scout ledger event count overflow".to_string())?;
                work.event_rows_written += 1;
            }
            (Some(existing), InsertOutcome::AlreadyPresent)
                if existing.event_json == event_json
                    && existing.event_sha256 == database::sha256_hex(&event_json)
                    && !existing.first_batch_id.is_empty() => {}
            (Some(_), InsertOutcome::AlreadyPresent) => {
                return Err("Scout ledger event-id collision detected".into())
            }
            (None, InsertOutcome::AlreadyPresent) => {
                return Err(
                    "Scout ledger event accumulator has no corresponding identity row".into(),
                )
            }
            (Some(_), InsertOutcome::Inserted) => {
                return Err("Scout ledger event row is absent from its accumulator".into())
            }
        }
    }

    database::insert_batch(
        &transaction,
        &authority.auth_key,
        &authority.enterprise_id,
        envelope.batch.batch_id.as_str(),
        next_generation,
        envelope.batch.events.len() as u64,
        &envelope_json,
    )?;
    work.batch_rows_written += 1;

    #[cfg(test)]
    if _failpoint == AppendFailpoint::AfterPayloadRows {
        return Err("test crash after ledger payload rows".into());
    }

    let next_head = successor(
        &head,
        next_generation,
        inserted_events,
        batch_editor.into_head(),
        event_editor.into_head(),
    )?;
    database::write_head(&transaction, &authority.auth_key, &next_head)?;
    work.head_rows_written += 1;

    #[cfg(test)]
    if _failpoint == AppendFailpoint::AfterHeadWrite {
        return Err("test crash after ledger head write".into());
    }

    transaction.commit().map_err(|error| error.to_string())?;
    #[cfg(test)]
    if _failpoint == AppendFailpoint::AfterCommitBeforeSeal {
        return Err("test crash after ledger commit before seal".into());
    }
    super::seal::write(&authority.root, &authority.auth_key, &next_head)?;
    Ok(LedgerAppendReceipt {
        outcome: LedgerAppendOutcome::Inserted,
        head: next_head,
        work,
    })
}

fn successor(
    previous: &LedgerHead,
    generation: u64,
    inserted_events: u64,
    batch_accumulator: scout_accumulator::PartitionedAccumulatorHead,
    event_accumulator: scout_accumulator::PartitionedAccumulatorHead,
) -> Result<LedgerHead, String> {
    let batch_count = previous
        .batch_count
        .checked_add(1)
        .ok_or_else(|| "Scout ledger batch count overflow".to_string())?;
    let event_count = previous
        .event_count
        .checked_add(inserted_events)
        .ok_or_else(|| "Scout ledger event count overflow".to_string())?;
    let head = database::make_head(database::LedgerHeadFields {
        enterprise_id: previous.enterprise_id.clone(),
        generation,
        previous_head_id: Some(previous.head_id.clone()),
        trust_chain_digest: previous.trust_chain_digest.clone(),
        batch_count,
        event_count,
        batch_accumulator,
        event_accumulator,
    })?;
    database::validate_head(&head, &previous.enterprise_id, &previous.trust_chain_digest)?;
    Ok(head)
}
