use std::collections::BTreeSet;

use agent_orchestration::{
    EnterpriseEvent, EnterpriseEventId, EnterpriseFact, EnterpriseId, EnterpriseSignedBatch,
};
use rusqlite::Connection;

use super::super::super::database::{read_meta, INDEX_AUTH_KEY_BYTES};
use super::super::events::{self, ProjectionLocator};
use super::super::{ProjectionLedgerCursor, PROJECTION_VERSION};
use crate::model::{IndexReceipt, IndexedStatus};

pub(super) fn projection_identity(
    connection: &Connection,
    enterprise_id: &EnterpriseId,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<(), String> {
    if read_meta(connection, "enterprise_id", auth_key)? != enterprise_id.as_str() {
        return Err("Scout cached projection belongs to another enterprise".into());
    }
    if read_meta(connection, "projection_version", auth_key)? != PROJECTION_VERSION.to_string() {
        return Err("Scout cached projection version is stale".into());
    }
    Ok(())
}

pub(super) fn cached_metadata(
    enterprise_id: &EnterpriseId,
    cursor: &ProjectionLedgerCursor,
    status: &IndexedStatus,
    receipt: &IndexReceipt,
) -> Result<(), String> {
    if status.enterprise_id != *enterprise_id
        || status.batches != usize::try_from(cursor.batch_count).unwrap_or(usize::MAX)
        || status.events != usize::try_from(cursor.event_count).unwrap_or(usize::MAX)
        || receipt.event_root != status.event_root
        || receipt.graph_digest != status.graph_digest
        || receipt.event_set_root_v1 != status.event_set_root_v1
        || receipt.projection_map_root_v2 != status.projection_map_root_v2
        || receipt.enterprise_snapshot_root_v2 != status.enterprise_snapshot_root_v2
    {
        return Err("Scout authenticated cached projection is internally inconsistent".into());
    }
    Ok(())
}

pub(super) fn single_append(
    previous: &ProjectionLedgerCursor,
    current: &ProjectionLedgerCursor,
    envelope: &EnterpriseSignedBatch,
) -> Result<(), String> {
    if !current.is_direct_successor_of(previous) {
        return Err("Scout ledger change is not one append over the cached cursor".into());
    }
    let added_events = current
        .event_count
        .checked_sub(previous.event_count)
        .ok_or_else(|| "Scout ledger event count moved backwards".to_string())?;
    if added_events > envelope.batch.events.len() as u64 {
        return Err("Scout ledger event delta exceeds its successor batch".into());
    }
    Ok(())
}

pub(super) fn control_barrier(
    envelope: &EnterpriseSignedBatch,
    inserted: &BTreeSet<EnterpriseEventId>,
    sealed_epoch: Option<u64>,
) -> Result<(), String> {
    for event in &envelope.batch.events {
        if !inserted.contains(&event.event_id) {
            continue;
        }
        if matches!(
            event.fact,
            EnterpriseFact::DiscoveryCharterObserved(_)
                | EnterpriseFact::DiscoveryPassSealed(_)
                | EnterpriseFact::ObservationRetracted { .. }
        ) {
            return Err("Scout control-plane append requires an immutable cold rebuild".into());
        }
        if sealed_epoch.is_some_and(|epoch| {
            event.provenance.discovery_epoch_sequence <= epoch
                && matches!(
                    event.fact,
                    EnterpriseFact::EntityObserved(_)
                        | EnterpriseFact::EdgeObserved(_)
                        | EnterpriseFact::SimulationContractObserved(_)
                )
        }) {
            return Err(
                "Scout append crosses the sealed projection barrier and requires a cold rebuild"
                    .into(),
            );
        }
    }
    Ok(())
}

pub(super) fn inserted_locators(
    envelope: &EnterpriseSignedBatch,
    inserted: &BTreeSet<EnterpriseEventId>,
    include_topology: bool,
) -> Result<BTreeSet<ProjectionLocator>, String> {
    envelope
        .batch
        .events
        .iter()
        .filter(|event| inserted.contains(&event.event_id))
        .filter(|event| {
            include_topology
                || !matches!(
                    event.fact,
                    EnterpriseFact::EntityObserved(_)
                        | EnterpriseFact::EdgeObserved(_)
                        | EnterpriseFact::SimulationContractObserved(_)
                )
        })
        .map(events::locator)
        .collect()
}

pub(super) fn locator_is_selected(
    event: &EnterpriseEvent,
    locators: &BTreeSet<ProjectionLocator>,
) -> bool {
    events::locator(event).is_ok_and(|locator| locators.contains(&locator))
}
