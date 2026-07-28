use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{EnterpriseId, EnterpriseProjectionSlice, EnterpriseSnapshot};
use rusqlite::Connection;
use scout_accumulator::{
    plan_map_put, AccumulatorContext, Digest, MapHead, MapMutationOutcome, MapStoredNode,
    PartitionedMapHead,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::{storage, CommitmentWork, PARTITION_BITS};
use crate::index::database::INDEX_AUTH_KEY_BYTES;
use crate::index::materialized::state::ProjectionState;

mod control;

use control::ProjectionControl;

type PartitionEntries = BTreeMap<u16, BTreeMap<String, Digest>>;
type PartitionRemovals = BTreeMap<u16, BTreeSet<String>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ProjectionMutation {
    puts: BTreeMap<String, Digest>,
    removals: BTreeSet<String>,
}

impl ProjectionMutation {
    pub(super) fn put(
        &mut self,
        kind: &str,
        identity: &(impl Serialize + ?Sized),
        value: &impl Serialize,
    ) -> Result<(), String> {
        let (object_id, value_digest) = object_entry(kind, identity, value)?;
        if self.puts.insert(object_id, value_digest).is_some() {
            return Err("Scout projection commitment identity collision".into());
        }
        Ok(())
    }

    #[allow(dead_code)] // Used by callers that supply non-monotonic projection mutations.
    pub(super) fn remove(
        &mut self,
        kind: &str,
        identity: &(impl Serialize + ?Sized),
    ) -> Result<(), String> {
        self.removals.insert(object_id(kind, identity)?);
        Ok(())
    }

    pub(super) fn merge(&mut self, other: Self) -> Result<(), String> {
        for (object_id, value_digest) in other.puts {
            if self
                .puts
                .get(&object_id)
                .is_some_and(|previous| *previous != value_digest)
            {
                return Err("Scout projection mutation has conflicting puts".into());
            }
            self.puts.insert(object_id, value_digest);
        }
        self.removals.extend(other.removals);
        Ok(())
    }
}

pub(super) fn object_entry(
    kind: &str,
    identity: &(impl Serialize + ?Sized),
    value: &impl Serialize,
) -> Result<(String, Digest), String> {
    Ok((
        object_id(kind, identity)?,
        value_digest(kind, identity, value)?,
    ))
}

pub(super) fn context(enterprise_id: &EnterpriseId) -> Result<AccumulatorContext, String> {
    AccumulatorContext::new(
        "clark.scout.enterprise-projection",
        enterprise_id.as_str(),
        "materialized-v2",
    )
    .map_err(|error| error.to_string())
}

pub(super) fn build(
    enterprise_id: &EnterpriseId,
    snapshot: &EnterpriseSnapshot,
) -> Result<(PartitionedMapHead, PartitionEntries), String> {
    let context = context(enterprise_id)?;
    let router = PartitionedMapHead::empty(context.clone(), PARTITION_BITS)
        .map_err(|error| error.to_string())?;
    let entries = partition_entries(&router, snapshot_entries(snapshot)?)?;
    let partitions = build_partitions(&router, &entries)?;
    let head = PartitionedMapHead::from_partitions(context, PARTITION_BITS, partitions)
        .map_err(|error| error.to_string())?;
    Ok((head, entries))
}

pub(super) fn append_with_mutation(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    head: PartitionedMapHead,
    update: &EnterpriseProjectionSlice,
    projection_state: &ProjectionState,
    mut mutation: ProjectionMutation,
) -> Result<(PartitionedMapHead, CommitmentWork), String> {
    mutation.merge(update_mutation(update, projection_state)?)?;
    mutate(connection, auth_key, enterprise_id, head, &mutation)
}

pub(super) fn mutate(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    enterprise_id: &EnterpriseId,
    head: PartitionedMapHead,
    mutation: &ProjectionMutation,
) -> Result<(PartitionedMapHead, CommitmentWork), String> {
    head.validate().map_err(|error| error.to_string())?;
    if head.context != context(enterprise_id)? || head.partition_bits != PARTITION_BITS {
        return Err("Scout projection commitment has the wrong routing context".into());
    }
    let puts = partition_entries(&head, mutation.puts.clone())?;
    let removals = partition_removals(&head, &mutation.removals)?;
    let touched = puts
        .keys()
        .chain(removals.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut partitions = head.partitions().clone();
    for partition in touched {
        let partition_context = head
            .partition_context(partition)
            .map_err(|error| error.to_string())?;
        let mut current = storage::read_projection_partition(connection, auth_key, partition)?;
        validate_routing(&head, partition, current.keys())?;
        let observed = build_partition(&partition_context, &current)?;
        let expected = head
            .partitions()
            .get(&partition)
            .copied()
            .unwrap_or_else(|| MapHead::empty(&partition_context));
        if observed != expected {
            return Err("Scout projection commitment partition is incomplete".into());
        }
        if let Some(object_ids) = removals.get(&partition) {
            current.retain(|object_id, _| !object_ids.contains(object_id));
        }
        if let Some(changed_entries) = puts.get(&partition) {
            current.extend(changed_entries.clone());
        }
        let rebuilt = build_partition(&partition_context, &current)?;
        if rebuilt.root.count == 0 {
            partitions.remove(&partition);
        } else {
            partitions.insert(partition, rebuilt);
        }
    }
    let work = storage::mutate_projection_entries(connection, auth_key, &puts, &removals)?;
    let next =
        PartitionedMapHead::from_partitions(head.context.clone(), head.partition_bits, partitions)
            .map_err(|error| error.to_string())?;
    Ok((next, work))
}

pub(super) fn validate_storage(
    connection: &Connection,
    auth_key: &[u8; INDEX_AUTH_KEY_BYTES],
    head: &PartitionedMapHead,
    stored_partitions: &std::collections::BTreeSet<u16>,
) -> Result<(), String> {
    let mut partitions = stored_partitions.clone();
    partitions.extend(head.partitions().keys().copied());
    for partition in partitions {
        let partition_context = head
            .partition_context(partition)
            .map_err(|error| error.to_string())?;
        let entries = storage::read_projection_partition(connection, auth_key, partition)?;
        validate_routing(head, partition, entries.keys())?;
        let observed = build_partition(&partition_context, &entries)?;
        let expected = head
            .partitions()
            .get(&partition)
            .copied()
            .unwrap_or_else(|| MapHead::empty(&partition_context));
        if observed != expected {
            return Err("Scout projection commitment storage does not realize its root".into());
        }
    }
    Ok(())
}

fn snapshot_entries(snapshot: &EnterpriseSnapshot) -> Result<BTreeMap<String, Digest>, String> {
    let mut entries = BTreeMap::new();
    for entity in snapshot.entities.values() {
        insert(&mut entries, "entity", entity.entity_id.as_str(), entity)?;
    }
    for edge in snapshot.edges.values() {
        insert(&mut entries, "edge", edge.edge_id.as_str(), edge)?;
    }
    for coverage in snapshot.coverage.values() {
        insert(
            &mut entries,
            "coverage",
            coverage.cell_id.as_str(),
            coverage,
        )?;
    }
    for frontier in snapshot.frontier.values() {
        insert(
            &mut entries,
            "frontier",
            frontier.task_id.as_str(),
            frontier,
        )?;
    }
    for simulation in snapshot.simulation_contracts.values() {
        insert(
            &mut entries,
            "simulation",
            simulation.runtime_id.as_str(),
            simulation,
        )?;
    }
    for conflict in &snapshot.conflicts {
        let identity = crate::index::materialized::conflicts::stable_key(conflict)?;
        insert(&mut entries, "conflict", &identity, conflict)?;
    }
    for (entity_id, versions) in &snapshot.entity_history {
        for (index, version) in versions.iter().enumerate() {
            insert(
                &mut entries,
                "entity-history",
                &(entity_id.as_str(), index),
                version,
            )?;
        }
    }
    for (edge_id, versions) in &snapshot.edge_history {
        for (index, version) in versions.iter().enumerate() {
            insert(
                &mut entries,
                "edge-history",
                &(edge_id.as_str(), index),
                version,
            )?;
        }
    }
    insert(
        &mut entries,
        "control",
        &"singleton",
        &ProjectionControl::from(snapshot),
    )?;
    Ok(entries)
}

fn update_mutation(
    update: &EnterpriseProjectionSlice,
    projection_state: &ProjectionState,
) -> Result<ProjectionMutation, String> {
    let mut mutation = ProjectionMutation::default();
    for entity in update.entities.values() {
        mutation.put("entity", entity.entity_id.as_str(), entity)?;
    }
    for edge in update.edges.values() {
        mutation.put("edge", edge.edge_id.as_str(), edge)?;
    }
    for coverage in update.coverage.values() {
        mutation.put("coverage", coverage.cell_id.as_str(), coverage)?;
    }
    for frontier in update.frontier.values() {
        mutation.put("frontier", frontier.task_id.as_str(), frontier)?;
    }
    for simulation in update.simulation_contracts.values() {
        mutation.put("simulation", simulation.runtime_id.as_str(), simulation)?;
    }
    mutation.put(
        "control",
        &"singleton",
        &ProjectionControl::from(projection_state),
    )?;
    Ok(mutation)
}

fn insert(
    entries: &mut BTreeMap<String, Digest>,
    kind: &str,
    identity: &(impl Serialize + ?Sized),
    value: &impl Serialize,
) -> Result<(), String> {
    let object_id = object_id(kind, identity)?;
    let value_digest = value_digest(kind, identity, value)?;
    if entries.insert(object_id, value_digest).is_some() {
        return Err("Scout projection commitment identity collision".into());
    }
    Ok(())
}

fn partition_entries(
    head: &PartitionedMapHead,
    entries: BTreeMap<String, Digest>,
) -> Result<PartitionEntries, String> {
    let mut partitions = BTreeMap::<u16, BTreeMap<String, Digest>>::new();
    for (object_id, value_digest) in entries {
        let partition = head
            .partition_for(&object_id)
            .map_err(|error| error.to_string())?;
        partitions
            .entry(partition)
            .or_default()
            .insert(object_id, value_digest);
    }
    Ok(partitions)
}

fn partition_removals(
    head: &PartitionedMapHead,
    object_ids: &BTreeSet<String>,
) -> Result<PartitionRemovals, String> {
    let mut partitions = PartitionRemovals::new();
    for object_id in object_ids {
        let partition = head
            .partition_for(object_id)
            .map_err(|error| error.to_string())?;
        partitions
            .entry(partition)
            .or_default()
            .insert(object_id.clone());
    }
    Ok(partitions)
}

fn build_partitions(
    head: &PartitionedMapHead,
    entries: &PartitionEntries,
) -> Result<BTreeMap<u16, MapHead>, String> {
    entries
        .iter()
        .map(|(partition, entries)| {
            let context = head
                .partition_context(*partition)
                .map_err(|error| error.to_string())?;
            build_partition(&context, entries).map(|built| (*partition, built))
        })
        .collect()
}

fn build_partition(
    context: &AccumulatorContext,
    entries: &BTreeMap<String, Digest>,
) -> Result<MapHead, String> {
    let mut head = MapHead::empty(context);
    let mut nodes = BTreeMap::<Digest, MapStoredNode>::new();
    for (object_id, value_digest) in entries {
        let mutation = plan_map_put(context, head, object_id.clone(), *value_digest, |digest| {
            Ok(nodes.get(&digest).cloned())
        })
        .map_err(|error| error.to_string())?;
        if mutation.outcome != MapMutationOutcome::Inserted {
            return Err("Scout projection commitment contains a duplicate identity".into());
        }
        apply_memory(context, &mut nodes, &mutation)?;
        head = mutation.next;
    }
    Ok(head)
}

fn apply_memory(
    context: &AccumulatorContext,
    nodes: &mut BTreeMap<Digest, MapStoredNode>,
    mutation: &scout_accumulator::MapMutation,
) -> Result<(), String> {
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
    for digest in &mutation.gc_candidates {
        if !written.contains_key(digest) && nodes.remove(digest).is_none() {
            return Err("Scout projection commitment path is incomplete".into());
        }
    }
    Ok(())
}

fn validate_routing<'a>(
    head: &PartitionedMapHead,
    partition: u16,
    object_ids: impl IntoIterator<Item = &'a String>,
) -> Result<(), String> {
    for object_id in object_ids {
        if head
            .partition_for(object_id)
            .map_err(|error| error.to_string())?
            != partition
        {
            return Err("Scout projection commitment entry is misrouted".into());
        }
    }
    Ok(())
}

pub(super) fn object_id(
    kind: &str,
    identity: &(impl Serialize + ?Sized),
) -> Result<String, String> {
    let digest = Sha256::digest(
        serde_json::to_vec(&("scout-projection-object-v2", kind, identity))
            .map_err(|error| error.to_string())?,
    );
    Ok(format!("{kind}:{digest:x}"))
}

fn value_digest(
    kind: &str,
    identity: &(impl Serialize + ?Sized),
    value: &impl Serialize,
) -> Result<Digest, String> {
    Ok(Digest::from_bytes(
        Sha256::digest(
            serde_json::to_vec(&("scout-projection-value-v2", kind, identity, value))
                .map_err(|error| error.to_string())?,
        )
        .into(),
    ))
}

#[cfg(test)]
mod tests;
