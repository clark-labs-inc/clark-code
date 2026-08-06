use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hash::{hash_tagged, AccumulatorContext, Digest};
use crate::map::{
    plan_map_put, plan_map_remove, MapHead, MapMutationOutcome, MapRoot, MapStoredNode,
    AUTHENTICATED_MAP_SCHEMA_VERSION,
};
use crate::partitioned::MAX_PARTITION_BITS;
use crate::tree::{validate_object_id, AccumulatorError};

const PARTITIONED_MAP_ROOT_TAG: &[u8] = b"scout-partitioned-authenticated-map-root-v1";
const PARTITIONED_MAP_NAMESPACE_TAG: &str = "scout-partitioned-authenticated-map-v1";

pub const PARTITIONED_AUTHENTICATED_MAP_SCHEMA_VERSION: u16 = 1;

/// Self-describing commitment to a fixed partition map of authenticated maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionedMapRoot {
    pub schema_version: u16,
    pub partition_bits: u8,
    pub digest: Digest,
    pub count: u64,
}

/// Current-state manifest whose disjoint partitions contain authenticated maps.
///
/// Stable object ids are routed by leading bits of their key in `context`.
/// Values can therefore be inserted, replaced, or removed inside one partition
/// without loading map nodes from any other partition. Independently produced
/// partitions compose deterministically when they were built under the exact
/// same base context and routing version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionedMapHead {
    pub schema_version: u16,
    pub context: AccumulatorContext,
    pub partition_bits: u8,
    pub root: PartitionedMapRoot,
    partitions: BTreeMap<u16, MapHead>,
}

impl PartitionedMapHead {
    pub fn empty(
        context: AccumulatorContext,
        partition_bits: u8,
    ) -> Result<Self, AccumulatorError> {
        Self::from_partitions(context, partition_bits, BTreeMap::new())
    }

    pub fn from_partitions(
        context: AccumulatorContext,
        partition_bits: u8,
        partitions: BTreeMap<u16, MapHead>,
    ) -> Result<Self, AccumulatorError> {
        context.validate()?;
        validate_partition_bits(partition_bits)?;
        validate_partitions(&context, partition_bits, &partitions)?;
        let root = compose_root(&context, partition_bits, &partitions)?;
        Ok(Self {
            schema_version: PARTITIONED_AUTHENTICATED_MAP_SCHEMA_VERSION,
            context,
            partition_bits,
            root,
            partitions,
        })
    }

    pub fn validate(&self) -> Result<(), AccumulatorError> {
        if self.schema_version != PARTITIONED_AUTHENTICATED_MAP_SCHEMA_VERSION
            || self.root.schema_version != PARTITIONED_AUTHENTICATED_MAP_SCHEMA_VERSION
            || self.root.partition_bits != self.partition_bits
        {
            return Err(AccumulatorError::UnsupportedVersion);
        }
        self.context.validate()?;
        validate_partition_bits(self.partition_bits)?;
        validate_partitions(&self.context, self.partition_bits, &self.partitions)?;
        if self.root != compose_root(&self.context, self.partition_bits, &self.partitions)? {
            return Err(AccumulatorError::RootMismatch);
        }
        Ok(())
    }

    pub fn partitions(&self) -> &BTreeMap<u16, MapHead> {
        &self.partitions
    }

    pub fn partition_for(&self, object_id: &str) -> Result<u16, AccumulatorError> {
        validate_object_id(object_id)?;
        Ok(partition_for_key(
            self.context.object_key(object_id),
            self.partition_bits,
        ))
    }

    pub fn partition_context(
        &self,
        partition: u16,
    ) -> Result<AccumulatorContext, AccumulatorError> {
        partition_context(&self.context, self.partition_bits, partition)
    }
}

/// Validated hot-path editor for repeated current-state changes.
///
/// The complete manifest is validated once at construction. Each mutation
/// validates only the selected authenticated-map path and then recomposes the
/// bounded root manifest.
pub struct PartitionedMapEditor {
    head: PartitionedMapHead,
}

impl PartitionedMapEditor {
    pub fn new(head: PartitionedMapHead) -> Result<Self, AccumulatorError> {
        head.validate()?;
        Ok(Self { head })
    }

    pub fn head(&self) -> &PartitionedMapHead {
        &self.head
    }

    pub fn into_head(self) -> PartitionedMapHead {
        self.head
    }

    pub fn put(
        &mut self,
        object_id: impl Into<String>,
        value_digest: Digest,
        mut read_node: impl FnMut(u16, Digest) -> Result<Option<MapStoredNode>, AccumulatorError>,
    ) -> Result<PartitionedMapUpdate, AccumulatorError> {
        let object_id = object_id.into();
        let partition = self.head.partition_for(&object_id)?;
        let context = self.head.partition_context(partition)?;
        let partition_head = self
            .head
            .partitions
            .get(&partition)
            .copied()
            .unwrap_or_else(|| MapHead::empty(&context));
        let mutation = plan_map_put(
            &context,
            partition_head,
            object_id,
            value_digest,
            |digest| read_node(partition, digest),
        )?;
        self.apply(
            partition,
            mutation.next,
            mutation.outcome,
            mutation.nodes,
            mutation.gc_candidates,
        )
    }

    pub fn remove(
        &mut self,
        object_id: impl Into<String>,
        mut read_node: impl FnMut(u16, Digest) -> Result<Option<MapStoredNode>, AccumulatorError>,
    ) -> Result<PartitionedMapUpdate, AccumulatorError> {
        let object_id = object_id.into();
        let partition = self.head.partition_for(&object_id)?;
        let context = self.head.partition_context(partition)?;
        let partition_head = self
            .head
            .partitions
            .get(&partition)
            .copied()
            .unwrap_or_else(|| MapHead::empty(&context));
        let mutation = plan_map_remove(&context, partition_head, object_id, |digest| {
            read_node(partition, digest)
        })?;
        self.apply(
            partition,
            mutation.next,
            mutation.outcome,
            mutation.nodes,
            mutation.gc_candidates,
        )
    }

    fn apply(
        &mut self,
        partition: u16,
        next_partition: MapHead,
        outcome: MapMutationOutcome,
        nodes: Vec<MapStoredNode>,
        gc_candidates: Vec<Digest>,
    ) -> Result<PartitionedMapUpdate, AccumulatorError> {
        let previous_root = self.head.root;
        let replacement = (next_partition.root.count != 0).then_some(next_partition);
        let next_root = compose_root_with_replacement(
            &self.head.context,
            self.head.partition_bits,
            &self.head.partitions,
            partition,
            replacement,
        )?;
        match replacement {
            Some(head) => {
                self.head.partitions.insert(partition, head);
            }
            None => {
                self.head.partitions.remove(&partition);
            }
        }
        self.head.root = next_root;
        Ok(PartitionedMapUpdate {
            previous_root,
            next_root,
            partition,
            outcome,
            nodes,
            gc_candidates,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionedMapUpdate {
    pub previous_root: PartitionedMapRoot,
    pub next_root: PartitionedMapRoot,
    pub partition: u16,
    pub outcome: MapMutationOutcome,
    pub nodes: Vec<MapStoredNode>,
    /// Nodes unreachable from the new partition head.
    ///
    /// These remain garbage-collection candidates rather than deletion
    /// instructions because older signed heads may still reference them.
    pub gc_candidates: Vec<Digest>,
}

fn validate_partition_bits(partition_bits: u8) -> Result<(), AccumulatorError> {
    if partition_bits > MAX_PARTITION_BITS {
        return Err(AccumulatorError::InvalidPartition);
    }
    Ok(())
}

fn validate_partitions(
    context: &AccumulatorContext,
    partition_bits: u8,
    partitions: &BTreeMap<u16, MapHead>,
) -> Result<(), AccumulatorError> {
    let partition_count = 1_u32 << partition_bits;
    for (partition, head) in partitions {
        if u32::from(*partition) >= partition_count || head.root.count == 0 {
            return Err(AccumulatorError::InvalidPartition);
        }
        head.validate(&partition_context(context, partition_bits, *partition)?)?;
    }
    Ok(())
}

fn partition_context(
    context: &AccumulatorContext,
    partition_bits: u8,
    partition: u16,
) -> Result<AccumulatorContext, AccumulatorError> {
    let partition_count = 1_u32 << partition_bits;
    if u32::from(partition) >= partition_count {
        return Err(AccumulatorError::InvalidPartition);
    }
    AccumulatorContext::new(
        context.domain(),
        context.enterprise_id(),
        format!(
            "{PARTITIONED_MAP_NAMESPACE_TAG}/{partition_bits}/{partition}/{}",
            context.namespace()
        ),
    )
}

fn partition_for_key(key: Digest, partition_bits: u8) -> u16 {
    if partition_bits == 0 {
        return 0;
    }
    let prefix = u16::from_be_bytes([key.as_bytes()[0], key.as_bytes()[1]]);
    prefix >> (16 - partition_bits)
}

fn compose_root(
    context: &AccumulatorContext,
    partition_bits: u8,
    partitions: &BTreeMap<u16, MapHead>,
) -> Result<PartitionedMapRoot, AccumulatorError> {
    compose_encoded_root(
        context,
        partition_bits,
        partitions
            .iter()
            .map(|(partition, head)| (*partition, head.root)),
    )
}

fn compose_root_with_replacement(
    context: &AccumulatorContext,
    partition_bits: u8,
    partitions: &BTreeMap<u16, MapHead>,
    target: u16,
    replacement: Option<MapHead>,
) -> Result<PartitionedMapRoot, AccumulatorError> {
    let mut roots = partitions
        .iter()
        .filter(|(partition, _)| **partition != target)
        .map(|(partition, head)| (*partition, head.root))
        .collect::<Vec<_>>();
    if let Some(head) = replacement {
        roots.push((target, head.root));
    }
    roots.sort_by_key(|(partition, _)| *partition);
    compose_encoded_root(context, partition_bits, roots)
}

fn compose_encoded_root(
    context: &AccumulatorContext,
    partition_bits: u8,
    roots: impl IntoIterator<Item = (u16, MapRoot)>,
) -> Result<PartitionedMapRoot, AccumulatorError> {
    let mut count = 0_u64;
    let mut partition_count = 0_u64;
    let mut encoded = Vec::new();
    for (partition, root) in roots {
        count = count
            .checked_add(root.count)
            .ok_or(AccumulatorError::CountOverflow)?;
        partition_count = partition_count
            .checked_add(1)
            .ok_or(AccumulatorError::CountOverflow)?;
        encoded.extend_from_slice(&partition.to_be_bytes());
        encoded.extend_from_slice(&root.schema_version.to_be_bytes());
        encoded.extend_from_slice(root.digest.as_bytes());
        encoded.extend_from_slice(&root.count.to_be_bytes());
    }
    let schema_version = PARTITIONED_AUTHENTICATED_MAP_SCHEMA_VERSION.to_be_bytes();
    let inner_schema_version = AUTHENTICATED_MAP_SCHEMA_VERSION.to_be_bytes();
    let partition_count = partition_count.to_be_bytes();
    let count_bytes = count.to_be_bytes();
    Ok(PartitionedMapRoot {
        schema_version: PARTITIONED_AUTHENTICATED_MAP_SCHEMA_VERSION,
        partition_bits,
        digest: hash_tagged(
            PARTITIONED_MAP_ROOT_TAG,
            &[
                &schema_version,
                &inner_schema_version,
                context.digest().as_bytes(),
                &[partition_bits],
                &partition_count,
                &count_bytes,
                &encoded,
            ],
        ),
        count,
    })
}
