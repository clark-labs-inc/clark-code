use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hash::{hash_tagged, AccumulatorContext, Digest};
use crate::persistent::{plan_insert, AccumulatorHead, StoredNode};
use crate::tree::{validate_object_id, AccumulatorError, InsertOutcome};

const PARTITIONED_ROOT_TAG: &[u8] = b"scout-partitioned-accumulator-root-v1";
const PARTITION_NAMESPACE_TAG: &str = "scout-partitioned-accumulator-v1";

pub const PARTITIONED_ACCUMULATOR_SCHEMA_VERSION: u16 = 1;
pub const DEFAULT_PARTITION_BITS: u8 = 8;
pub const MAX_PARTITION_BITS: u8 = 16;

/// Self-describing commitment to a fixed partition map.
///
/// The digest commits to the base context, partitioning algorithm, every
/// non-empty partition head, and the total member count. An absent partition
/// means the canonical empty accumulator for that exact derived context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionedAccumulatorRoot {
    pub schema_version: u16,
    pub partition_bits: u8,
    pub digest: Digest,
    pub count: u64,
}

/// A mergeable manifest of disjoint, fixed accumulator partitions.
///
/// Each object belongs to exactly one partition selected by the leading
/// `partition_bits` of its key in the base context. Independently populated
/// partitions can therefore be composed without loading members from other
/// partitions. Updating one partition changes only its radix path plus this
/// bounded manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionedAccumulatorHead {
    pub schema_version: u16,
    pub context: AccumulatorContext,
    pub partition_bits: u8,
    pub root: PartitionedAccumulatorRoot,
    partitions: BTreeMap<u16, AccumulatorHead>,
}

impl PartitionedAccumulatorHead {
    pub fn empty(
        context: AccumulatorContext,
        partition_bits: u8,
    ) -> Result<Self, AccumulatorError> {
        Self::from_partitions(context, partition_bits, BTreeMap::new())
    }

    pub fn from_partitions(
        context: AccumulatorContext,
        partition_bits: u8,
        partitions: BTreeMap<u16, AccumulatorHead>,
    ) -> Result<Self, AccumulatorError> {
        context.validate()?;
        validate_partition_bits(partition_bits)?;
        validate_partitions(&context, partition_bits, &partitions)?;
        let root = compose_root(&context, partition_bits, &partitions)?;
        Ok(Self {
            schema_version: PARTITIONED_ACCUMULATOR_SCHEMA_VERSION,
            context,
            partition_bits,
            root,
            partitions,
        })
    }

    pub fn validate(&self) -> Result<(), AccumulatorError> {
        if self.schema_version != PARTITIONED_ACCUMULATOR_SCHEMA_VERSION
            || self.root.schema_version != PARTITIONED_ACCUMULATOR_SCHEMA_VERSION
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

    pub fn partitions(&self) -> &BTreeMap<u16, AccumulatorHead> {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionedAccumulatorMutation {
    pub previous: PartitionedAccumulatorHead,
    pub next: PartitionedAccumulatorHead,
    pub partition: u16,
    pub outcome: InsertOutcome,
    pub nodes: Vec<StoredNode>,
    pub obsolete_nodes: Vec<Digest>,
}

/// Validated hot-path editor for applying many inserts to one manifest.
///
/// Construction validates the complete manifest once. Each subsequent insert
/// validates only the selected partition path and recomposes the bounded
/// partition-root manifest, avoiding repeated validation and cloning of every
/// unaffected partition.
pub struct PartitionedAccumulatorEditor {
    head: PartitionedAccumulatorHead,
}

impl PartitionedAccumulatorEditor {
    pub fn new(head: PartitionedAccumulatorHead) -> Result<Self, AccumulatorError> {
        head.validate()?;
        Ok(Self { head })
    }

    pub fn head(&self) -> &PartitionedAccumulatorHead {
        &self.head
    }

    pub fn into_head(self) -> PartitionedAccumulatorHead {
        self.head
    }

    pub fn insert(
        &mut self,
        object_id: impl Into<String>,
        mut read_node: impl FnMut(u16, Digest) -> Result<Option<StoredNode>, AccumulatorError>,
    ) -> Result<PartitionedAccumulatorUpdate, AccumulatorError> {
        let object_id = object_id.into();
        let partition = self.head.partition_for(&object_id)?;
        let context = self.head.partition_context(partition)?;
        let partition_head = self
            .head
            .partitions
            .get(&partition)
            .copied()
            .unwrap_or_else(|| AccumulatorHead::empty(&context));
        let mutation = plan_insert(&context, partition_head, object_id, |digest| {
            read_node(partition, digest)
        })?;
        let previous_root = self.head.root;
        if mutation.next.root.count == 0 {
            self.head.partitions.remove(&partition);
        } else {
            self.head.partitions.insert(partition, mutation.next);
        }
        self.head.root = compose_root(
            &self.head.context,
            self.head.partition_bits,
            &self.head.partitions,
        )?;
        Ok(PartitionedAccumulatorUpdate {
            previous_root,
            next_root: self.head.root,
            partition,
            outcome: mutation.outcome,
            nodes: mutation.nodes,
            obsolete_nodes: mutation.obsolete_nodes,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionedAccumulatorUpdate {
    pub previous_root: PartitionedAccumulatorRoot,
    pub next_root: PartitionedAccumulatorRoot,
    pub partition: u16,
    pub outcome: InsertOutcome,
    pub nodes: Vec<StoredNode>,
    pub obsolete_nodes: Vec<Digest>,
}

/// Plans one persistent insert while touching only the selected partition.
///
/// `read_node` receives the partition before the content-addressed node digest,
/// so a store can keep partition namespaces physically separate.
pub fn plan_partitioned_insert(
    head: PartitionedAccumulatorHead,
    object_id: impl Into<String>,
    read_node: impl FnMut(u16, Digest) -> Result<Option<StoredNode>, AccumulatorError>,
) -> Result<PartitionedAccumulatorMutation, AccumulatorError> {
    let previous = head.clone();
    let mut editor = PartitionedAccumulatorEditor::new(head)?;
    let update = editor.insert(object_id, read_node)?;
    Ok(PartitionedAccumulatorMutation {
        previous,
        next: editor.into_head(),
        partition: update.partition,
        outcome: update.outcome,
        nodes: update.nodes,
        obsolete_nodes: update.obsolete_nodes,
    })
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
    partitions: &BTreeMap<u16, AccumulatorHead>,
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
            "{PARTITION_NAMESPACE_TAG}/{partition_bits}/{partition}/{}",
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
    partitions: &BTreeMap<u16, AccumulatorHead>,
) -> Result<PartitionedAccumulatorRoot, AccumulatorError> {
    let mut count = 0_u64;
    let mut encoded = Vec::with_capacity(partitions.len() * 42);
    for (partition, head) in partitions {
        count = count
            .checked_add(head.root.count)
            .ok_or(AccumulatorError::CountOverflow)?;
        encoded.extend_from_slice(&partition.to_be_bytes());
        encoded.extend_from_slice(head.root.digest.as_bytes());
        encoded.extend_from_slice(&head.root.count.to_be_bytes());
    }
    let schema_version = PARTITIONED_ACCUMULATOR_SCHEMA_VERSION.to_be_bytes();
    let count_bytes = count.to_be_bytes();
    let partition_count = (partitions.len() as u64).to_be_bytes();
    Ok(PartitionedAccumulatorRoot {
        schema_version: PARTITIONED_ACCUMULATOR_SCHEMA_VERSION,
        partition_bits,
        digest: hash_tagged(
            PARTITIONED_ROOT_TAG,
            &[
                &schema_version,
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
