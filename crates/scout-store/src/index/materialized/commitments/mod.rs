use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    EnterpriseEventId, EnterpriseId, EnterpriseProjectionSlice, EnterpriseSnapshot,
    EnterpriseSnapshotCommitmentV2,
};
use rusqlite::Connection;
use scout_accumulator::{
    plan_insert, AccumulatorContext, AccumulatorHead, Digest, InsertOutcome,
    PartitionedAccumulatorHead, StoredNode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::index::database::{read_meta_json, write_meta_json, INDEX_AUTH_KEY_BYTES};
use crate::index::materialized::state::ProjectionState;

mod projection;
mod storage;

const COMMITMENT_SCHEMA_VERSION: u16 = 3;
const META_KEY: &str = "commitment_state_v1";
const EVENT_LANE: &str = "event-set-v1";
const PROJECTION_LANE: &str = "projection-map-v2";
const PARTITION_BITS: u8 = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CommitmentWork {
    pub(super) rows_written: usize,
    pub(super) rows_deleted: usize,
}

pub(super) struct ProjectionDelta<'a> {
    pub(super) inserted_event_ids: &'a BTreeSet<EnterpriseEventId>,
    pub(super) update: &'a EnterpriseProjectionSlice,
    pub(super) projection_state: &'a ProjectionState,
    pub(super) conflict_mutation: &'a super::conflicts::ConflictMutation,
}

impl CommitmentWork {
    pub(super) fn add(&mut self, other: Self) {
        self.rows_written = self.rows_written.saturating_add(other.rows_written);
        self.rows_deleted = self.rows_deleted.saturating_add(other.rows_deleted);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommitmentState {
    schema_version: u16,
    event_set: PartitionedAccumulatorHead,
    projection_map: scout_accumulator::PartitionedMapHead,
}

impl CommitmentState {
    pub(super) fn event_root_id(&self) -> String {
        let root = self.event_set.root;
        format!(
            "scout-event-set-v1:{}:{}:{}",
            root.partition_bits,
            root.count,
            root.digest.to_hex()
        )
    }

    pub(super) fn projection_root_id(&self) -> String {
        let root = self.projection_map.root;
        format!(
            "scout-projection-map-v2:{}:{}:{}",
            root.partition_bits,
            root.count,
            root.digest.to_hex()
        )
    }

    pub(super) fn snapshot_root_id(
        &self,
        enterprise_id: &EnterpriseId,
        graph_digest: &str,
    ) -> Result<String, String> {
        Ok(EnterpriseSnapshotCommitmentV2::new(
            enterprise_id,
            graph_digest,
            self.event_root_id(),
            self.projection_root_id(),
        )?
        .enterprise_snapshot_root_v2)
    }

    pub(super) fn materialized_event_digest(
        &self,
        enterprise_id: &EnterpriseId,
    ) -> Result<String, String> {
        digest_identity(&(
            "scout-materialized-event-root-v1",
            enterprise_id.as_str(),
            self.event_root_id(),
        ))
    }

    pub(super) fn materialized_graph_digest(
        &self,
        enterprise_id: &EnterpriseId,
    ) -> Result<String, String> {
        digest_identity(&(
            "scout-materialized-graph-root-v2",
            enterprise_id.as_str(),
            self.event_root_id(),
            self.projection_root_id(),
        ))
    }

    pub(super) fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        if self.schema_version != COMMITMENT_SCHEMA_VERSION {
            return Err("Scout commitment state has an unsupported version".into());
        }
        if self.event_set.context != event_context(enterprise_id)? {
            return Err("Scout event commitment belongs to another enterprise".into());
        }
        self.event_set
            .validate()
            .map_err(|error| error.to_string())?;
        if self.projection_map.context != projection::context(enterprise_id)? {
            return Err("Scout projection commitment belongs to another enterprise".into());
        }
        self.projection_map
            .validate()
            .map_err(|error| error.to_string())?;
        if self.event_set.partition_bits != PARTITION_BITS
            || self.projection_map.partition_bits != PARTITION_BITS
        {
            return Err("Scout commitment state uses unsupported partition routing".into());
        }
        Ok(())
    }
}

fn digest_identity(value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(super) fn rebuild(
    connection: &Connection,
    enterprise_id: &EnterpriseId,
    event_ids: impl IntoIterator<Item = EnterpriseEventId>,
    snapshot: &EnterpriseSnapshot,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<(CommitmentState, CommitmentWork), String> {
    let event_context = event_context(enterprise_id)?;
    let router = PartitionedAccumulatorHead::empty(event_context.clone(), PARTITION_BITS)
        .map_err(|error| error.to_string())?;
    let event_entries = partition_event_ids(&router, event_ids)?;
    let event_partitions = build_event_partitions(&router, &event_entries)?;
    let event_set = PartitionedAccumulatorHead::from_partitions(
        event_context,
        PARTITION_BITS,
        event_partitions,
    )
    .map_err(|error| error.to_string())?;
    let (projection_map, projection_entries) = projection::build(enterprise_id, snapshot)?;
    let work = storage::replace_all(connection, auth_key, &event_entries, &projection_entries)?;
    let state = CommitmentState {
        schema_version: COMMITMENT_SCHEMA_VERSION,
        event_set,
        projection_map,
    };
    state.validate(enterprise_id)?;
    Ok((state, work))
}

pub(super) fn append(
    connection: &Connection,
    enterprise_id: &EnterpriseId,
    mut state: CommitmentState,
    delta: ProjectionDelta<'_>,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<(CommitmentState, CommitmentWork), String> {
    state.validate(enterprise_id)?;
    let (event_set, mut work) = append_event_ids(
        connection,
        auth_key,
        state.event_set,
        delta.inserted_event_ids,
    )?;
    state.event_set = event_set;
    let mut extra_projection_mutation = projection::ProjectionMutation::default();
    for (identity, conflict) in delta.conflict_mutation.commitment_puts() {
        extra_projection_mutation.put("conflict", &identity, conflict)?;
    }
    for identity in delta.conflict_mutation.commitment_removals() {
        extra_projection_mutation.remove("conflict", &identity)?;
    }
    let (projection_map, projection_work) = projection::append_with_mutation(
        connection,
        auth_key,
        enterprise_id,
        state.projection_map,
        delta.update,
        delta.projection_state,
        extra_projection_mutation,
    )?;
    state.projection_map = projection_map;
    work.add(projection_work);
    state.validate(enterprise_id)?;
    Ok((state, work))
}

pub(super) fn read(
    connection: &Connection,
    enterprise_id: &EnterpriseId,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<CommitmentState, String> {
    let state: CommitmentState = read_meta_json(connection, META_KEY, auth_key)?;
    state.validate(enterprise_id)?;
    Ok(state)
}

pub(super) fn write(
    connection: &Connection,
    state: &CommitmentState,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<(), String> {
    write_meta_json(connection, META_KEY, state, auth_key)
}

pub(super) fn validate_storage(
    connection: &Connection,
    enterprise_id: &EnterpriseId,
    state: &CommitmentState,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
) -> Result<(), String> {
    state.validate(enterprise_id)?;
    storage::validate_counts(
        connection,
        state.event_set.root.count,
        state.projection_map.root.count,
    )?;
    let mut stored_partitions = storage::partition_ids(connection)?;
    let event_partitions = stored_partitions.remove(EVENT_LANE).unwrap_or_default();
    let projection_partitions = stored_partitions
        .remove(PROJECTION_LANE)
        .unwrap_or_default();
    if !stored_partitions.is_empty() {
        return Err("Scout commitment storage contains an unknown lane".into());
    }
    validate_event_storage(connection, auth_key, &state.event_set, &event_partitions)?;
    projection::validate_storage(
        connection,
        auth_key,
        &state.projection_map,
        &projection_partitions,
    )
}

fn validate_event_storage(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    head: &PartitionedAccumulatorHead,
    stored_partitions: &BTreeSet<u16>,
) -> Result<(), String> {
    let mut partitions = stored_partitions.clone();
    partitions.extend(head.partitions().keys().copied());
    for partition in partitions {
        let partition_context = head
            .partition_context(partition)
            .map_err(|error| error.to_string())?;
        let event_ids = storage::read_event_partition(connection, auth_key, partition)?;
        validate_event_routing(head, partition, &event_ids)?;
        let observed = build_event_partition(&partition_context, &event_ids)?;
        let expected = head
            .partitions()
            .get(&partition)
            .copied()
            .unwrap_or_else(|| AccumulatorHead::empty(&partition_context));
        if observed != expected {
            return Err("Scout event commitment storage does not realize its root".into());
        }
    }
    Ok(())
}

fn append_event_ids(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    head: PartitionedAccumulatorHead,
    inserted_event_ids: &BTreeSet<EnterpriseEventId>,
) -> Result<(PartitionedAccumulatorHead, CommitmentWork), String> {
    let changes = partition_event_ids(&head, inserted_event_ids.iter().cloned())?;
    let mut partitions = head.partitions().clone();
    for (partition, changed_ids) in &changes {
        let partition_context = head
            .partition_context(*partition)
            .map_err(|error| error.to_string())?;
        let mut current = storage::read_event_partition(connection, auth_key, *partition)?;
        validate_event_routing(&head, *partition, &current)?;
        let observed = build_event_partition(&partition_context, &current)?;
        let expected = head
            .partitions()
            .get(partition)
            .copied()
            .unwrap_or_else(|| AccumulatorHead::empty(&partition_context));
        if observed != expected {
            return Err("Scout event commitment partition is incomplete".into());
        }
        current.extend(changed_ids.iter().cloned());
        let rebuilt = build_event_partition(&partition_context, &current)?;
        partitions.insert(*partition, rebuilt);
    }
    let work = storage::insert_event_entries(connection, auth_key, &changes)?;
    if work.rows_written != inserted_event_ids.len() {
        return Err("Scout incremental event commitment did not insert every new id".into());
    }
    let next = PartitionedAccumulatorHead::from_partitions(
        head.context.clone(),
        head.partition_bits,
        partitions,
    )
    .map_err(|error| error.to_string())?;
    Ok((next, work))
}

fn partition_event_ids(
    head: &PartitionedAccumulatorHead,
    event_ids: impl IntoIterator<Item = EnterpriseEventId>,
) -> Result<BTreeMap<u16, BTreeSet<String>>, String> {
    let mut entries = BTreeMap::<u16, BTreeSet<String>>::new();
    for event_id in event_ids {
        let object_id = event_id.as_str().to_owned();
        let partition = head
            .partition_for(&object_id)
            .map_err(|error| error.to_string())?;
        if !entries.entry(partition).or_default().insert(object_id) {
            return Err("Scout event commitment contains a duplicate id".into());
        }
    }
    Ok(entries)
}

fn build_event_partitions(
    head: &PartitionedAccumulatorHead,
    entries: &BTreeMap<u16, BTreeSet<String>>,
) -> Result<BTreeMap<u16, AccumulatorHead>, String> {
    entries
        .iter()
        .map(|(partition, event_ids)| {
            let context = head
                .partition_context(*partition)
                .map_err(|error| error.to_string())?;
            build_event_partition(&context, event_ids).map(|built| (*partition, built))
        })
        .collect()
}

fn build_event_partition(
    context: &AccumulatorContext,
    event_ids: &BTreeSet<String>,
) -> Result<AccumulatorHead, String> {
    let mut head = AccumulatorHead::empty(context);
    let mut nodes = BTreeMap::<Digest, StoredNode>::new();
    for event_id in event_ids {
        let mutation = plan_insert(context, head, event_id.clone(), |digest| {
            Ok(nodes.get(&digest).cloned())
        })
        .map_err(|error| error.to_string())?;
        if mutation.outcome != InsertOutcome::Inserted {
            return Err("Scout event commitment contains a duplicate id".into());
        }
        let written = mutation
            .nodes
            .iter()
            .map(|node| {
                node.digest(context)
                    .map_err(|error| error.to_string())
                    .map(|digest| (digest, node))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        for (digest, node) in &written {
            nodes.insert(*digest, (*node).clone());
        }
        for digest in &mutation.obsolete_nodes {
            if !written.contains_key(digest) && nodes.remove(digest).is_none() {
                return Err("Scout event commitment path is incomplete".into());
            }
        }
        head = mutation.next;
    }
    Ok(head)
}

fn validate_event_routing(
    head: &PartitionedAccumulatorHead,
    partition: u16,
    event_ids: &BTreeSet<String>,
) -> Result<(), String> {
    for event_id in event_ids {
        if head
            .partition_for(event_id)
            .map_err(|error| error.to_string())?
            != partition
        {
            return Err("Scout event commitment entry is misrouted".into());
        }
    }
    Ok(())
}

fn event_context(enterprise_id: &EnterpriseId) -> Result<AccumulatorContext, String> {
    AccumulatorContext::new(
        "clark.scout.enterprise-ledger",
        enterprise_id.as_str(),
        "event",
    )
    .map_err(|error| error.to_string())
}
