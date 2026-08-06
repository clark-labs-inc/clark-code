use std::collections::BTreeMap;

use scout_accumulator::{
    AccumulatorContext, Digest, PartitionedAccumulatorEditor, PartitionedAccumulatorHead,
    StoredNode,
};
use serde::{Deserialize, Serialize};

use crate::scout::enterprise::contract::{
    canonical_digest, EnterpriseBatch, EnterpriseBatchId, EnterpriseEvent, EnterpriseEventId,
    EnterpriseId,
};

pub const ENTERPRISE_LEDGER_COMMITMENT_SCHEMA_VERSION: u16 = 1;
pub const ENTERPRISE_SNAPSHOT_COMMITMENT_SCHEMA_VERSION: u16 = 1;
pub const ENTERPRISE_SNAPSHOT_COMMITMENT_V2_SCHEMA_VERSION: u16 = 2;

const LEDGER_ACCUMULATOR_DOMAIN: &str = "clark.scout.enterprise-ledger";
const BATCH_ACCUMULATOR_NAMESPACE: &str = "batch";
const EVENT_ACCUMULATOR_NAMESPACE: &str = "event";
const LEDGER_ACCUMULATOR_PARTITION_BITS: u8 = 12;

/// Portable commitment to a generation of the immutable enterprise ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnterpriseLedgerCommitment {
    pub schema_version: u16,
    pub generation: u64,
    pub batch_set_root_v1: String,
    pub event_set_root_v1: String,
    pub batch_count: u64,
    pub event_count: u64,
    pub enterprise_ledger_root_v2: String,
}

impl EnterpriseLedgerCommitment {
    pub fn new(
        enterprise_id: &EnterpriseId,
        generation: u64,
        batch_set_root_v1: impl Into<String>,
        event_set_root_v1: impl Into<String>,
        batch_count: u64,
        event_count: u64,
    ) -> Result<Self, String> {
        let mut value = Self {
            schema_version: ENTERPRISE_LEDGER_COMMITMENT_SCHEMA_VERSION,
            generation,
            batch_set_root_v1: batch_set_root_v1.into(),
            event_set_root_v1: event_set_root_v1.into(),
            batch_count,
            event_count,
            enterprise_ledger_root_v2: String::new(),
        };
        value.enterprise_ledger_root_v2 = value.derived_root(enterprise_id)?;
        value.validate(enterprise_id)?;
        Ok(value)
    }

    pub fn from_batches<'a>(
        enterprise_id: &EnterpriseId,
        generation: u64,
        batches: impl IntoIterator<Item = &'a EnterpriseBatch>,
    ) -> Result<Self, String> {
        let mut batches_by_id = BTreeMap::<EnterpriseBatchId, &EnterpriseBatch>::new();
        let mut events_by_id = BTreeMap::<EnterpriseEventId, &EnterpriseEvent>::new();
        for batch in batches {
            batch.validate()?;
            if batch.enterprise_id != *enterprise_id {
                return Err("ledger commitment contains a batch for another enterprise".into());
            }
            if let Some(existing) = batches_by_id.insert(batch.batch_id.clone(), batch) {
                if existing != batch {
                    return Err("ledger commitment contains a batch-id collision".into());
                }
            }
            for event in &batch.events {
                match events_by_id.insert(event.event_id.clone(), event) {
                    Some(existing) if existing != event => {
                        return Err("ledger commitment contains an event-id collision".into())
                    }
                    _ => {}
                }
            }
        }
        let batch_set_root_v1 = build_set_root(
            enterprise_id,
            BATCH_ACCUMULATOR_NAMESPACE,
            "scout-batch-set-v1",
            batches_by_id.keys().map(EnterpriseBatchId::as_str),
        )?;
        let event_set_root_v1 = build_set_root(
            enterprise_id,
            EVENT_ACCUMULATOR_NAMESPACE,
            "scout-event-set-v1",
            events_by_id.keys().map(EnterpriseEventId::as_str),
        )?;
        Self::new(
            enterprise_id,
            generation,
            batch_set_root_v1,
            event_set_root_v1,
            u64::try_from(batches_by_id.len())
                .map_err(|_| "ledger batch count does not fit in u64".to_string())?,
            u64::try_from(events_by_id.len())
                .map_err(|_| "ledger event count does not fit in u64".to_string())?,
        )
    }

    pub fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_LEDGER_COMMITMENT_SCHEMA_VERSION {
            return Err("unsupported enterprise ledger commitment schema".into());
        }
        if self.generation == 0 {
            return Err("enterprise ledger commitment generation must be positive".into());
        }
        let (batch_partition_bits, committed_batch_count) = validate_partitioned_root_id(
            "enterprise batch-set root v1",
            &self.batch_set_root_v1,
            "scout-batch-set-v1",
        )?;
        let (event_partition_bits, committed_event_count) = validate_partitioned_root_id(
            "enterprise event-set root v1",
            &self.event_set_root_v1,
            "scout-event-set-v1",
        )?;
        if batch_partition_bits != event_partition_bits {
            return Err("enterprise batch and event roots use different partition routing".into());
        }
        if batch_partition_bits != LEDGER_ACCUMULATOR_PARTITION_BITS {
            return Err("enterprise ledger roots use unsupported partition routing".into());
        }
        if committed_batch_count != self.batch_count || committed_event_count != self.event_count {
            return Err("enterprise ledger commitment count mismatch".into());
        }
        validate_prefixed_root_id(
            "enterprise ledger root v2",
            &self.enterprise_ledger_root_v2,
            "scout-enterprise-ledger-v2:",
        )?;
        if self.derived_root(enterprise_id)? != self.enterprise_ledger_root_v2 {
            return Err("enterprise ledger commitment root mismatch".into());
        }
        Ok(())
    }

    pub fn derived_root(&self, enterprise_id: &EnterpriseId) -> Result<String, String> {
        Ok(format!(
            "scout-enterprise-ledger-v2:{}",
            canonical_digest(&(
                "scout-enterprise-ledger-v2",
                self.schema_version,
                enterprise_id.as_str(),
                self.generation,
                &self.batch_set_root_v1,
                &self.event_set_root_v1,
                self.batch_count,
                self.event_count,
            ))?
        ))
    }

    pub fn compatibility_batch_root(&self, enterprise_id: &EnterpriseId) -> Result<String, String> {
        canonical_digest(&(
            "scout-enterprise-checkpoint-compat-root-v2",
            enterprise_id.as_str(),
            "batch",
            &self.batch_set_root_v1,
        ))
    }

    pub fn compatibility_event_root(&self, enterprise_id: &EnterpriseId) -> Result<String, String> {
        canonical_digest(&(
            "scout-enterprise-checkpoint-compat-root-v2",
            enterprise_id.as_str(),
            "event",
            &self.event_set_root_v1,
        ))
    }
}

fn build_set_root<'a>(
    enterprise_id: &EnterpriseId,
    accumulator_namespace: &str,
    root_namespace: &str,
    object_ids: impl IntoIterator<Item = &'a str>,
) -> Result<String, String> {
    let context = AccumulatorContext::new(
        LEDGER_ACCUMULATOR_DOMAIN,
        enterprise_id.as_str(),
        accumulator_namespace,
    )
    .map_err(|error| error.to_string())?;
    let head = PartitionedAccumulatorHead::empty(context, LEDGER_ACCUMULATOR_PARTITION_BITS)
        .map_err(|error| error.to_string())?;
    let mut editor = PartitionedAccumulatorEditor::new(head).map_err(|error| error.to_string())?;
    let mut nodes = BTreeMap::<(u16, Digest), StoredNode>::new();
    for object_id in object_ids {
        let update = editor
            .insert(object_id, |partition, digest| {
                Ok(nodes.get(&(partition, digest)).cloned())
            })
            .map_err(|error| error.to_string())?;
        let partition_context = editor
            .head()
            .partition_context(update.partition)
            .map_err(|error| error.to_string())?;
        for node in update.nodes {
            let digest = node
                .digest(&partition_context)
                .map_err(|error| error.to_string())?;
            nodes.insert((update.partition, digest), node);
        }
    }
    let root = editor.into_head().root;
    Ok(format!(
        "{root_namespace}:{}:{}:{}",
        root.partition_bits,
        root.count,
        root.digest.to_hex()
    ))
}

/// Portable commitment joining legacy graph identity to incremental roots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnterpriseSnapshotCommitment {
    pub schema_version: u16,
    pub graph_digest: String,
    pub event_set_root_v1: String,
    pub projection_map_root_v1: String,
    pub enterprise_snapshot_root_v1: String,
}

impl EnterpriseSnapshotCommitment {
    pub fn new(
        enterprise_id: &EnterpriseId,
        graph_digest: impl Into<String>,
        event_set_root_v1: impl Into<String>,
        projection_map_root_v1: impl Into<String>,
    ) -> Result<Self, String> {
        let mut value = Self {
            schema_version: ENTERPRISE_SNAPSHOT_COMMITMENT_SCHEMA_VERSION,
            graph_digest: graph_digest.into(),
            event_set_root_v1: event_set_root_v1.into(),
            projection_map_root_v1: projection_map_root_v1.into(),
            enterprise_snapshot_root_v1: String::new(),
        };
        value.enterprise_snapshot_root_v1 = value.derived_root(enterprise_id)?;
        value.validate(enterprise_id)?;
        Ok(value)
    }

    pub fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_SNAPSHOT_COMMITMENT_SCHEMA_VERSION {
            return Err("unsupported enterprise snapshot commitment schema".into());
        }
        validate_lower_hex_digest("enterprise graph digest", &self.graph_digest)?;
        let (event_partition_bits, _) = validate_partitioned_root_id(
            "enterprise event-set root v1",
            &self.event_set_root_v1,
            "scout-event-set-v1",
        )?;
        let (projection_partition_bits, _) = validate_partitioned_root_id(
            "enterprise projection-map root v1",
            &self.projection_map_root_v1,
            "scout-projection-map-v1",
        )?;
        if event_partition_bits != projection_partition_bits {
            return Err(
                "enterprise event and projection roots use different partition routing".into(),
            );
        }
        validate_prefixed_root_id(
            "enterprise snapshot root v1",
            &self.enterprise_snapshot_root_v1,
            "scout-enterprise-snapshot-v1:",
        )?;
        if self.derived_root(enterprise_id)? != self.enterprise_snapshot_root_v1 {
            return Err("enterprise snapshot commitment root mismatch".into());
        }
        Ok(())
    }

    pub fn derived_root(&self, enterprise_id: &EnterpriseId) -> Result<String, String> {
        Ok(format!(
            "scout-enterprise-snapshot-v1:{}",
            canonical_digest(&(
                "scout-enterprise-snapshot-v1",
                enterprise_id.as_str(),
                &self.graph_digest,
                &self.event_set_root_v1,
                &self.projection_map_root_v1,
            ))?
        ))
    }
}

/// Portable commitment to the v2 materialized projection encoding.
///
/// This is intentionally a distinct type and root domain from the legacy v1
/// snapshot commitment. A v1 root is never interpreted as a v2 projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnterpriseSnapshotCommitmentV2 {
    pub schema_version: u16,
    pub graph_digest: String,
    pub event_set_root_v1: String,
    pub projection_map_root_v2: String,
    pub enterprise_snapshot_root_v2: String,
}

impl EnterpriseSnapshotCommitmentV2 {
    pub fn new(
        enterprise_id: &EnterpriseId,
        graph_digest: impl Into<String>,
        event_set_root_v1: impl Into<String>,
        projection_map_root_v2: impl Into<String>,
    ) -> Result<Self, String> {
        let mut value = Self {
            schema_version: ENTERPRISE_SNAPSHOT_COMMITMENT_V2_SCHEMA_VERSION,
            graph_digest: graph_digest.into(),
            event_set_root_v1: event_set_root_v1.into(),
            projection_map_root_v2: projection_map_root_v2.into(),
            enterprise_snapshot_root_v2: String::new(),
        };
        value.enterprise_snapshot_root_v2 = value.derived_root(enterprise_id)?;
        value.validate(enterprise_id)?;
        Ok(value)
    }

    pub fn validate(&self, enterprise_id: &EnterpriseId) -> Result<(), String> {
        if self.schema_version != ENTERPRISE_SNAPSHOT_COMMITMENT_V2_SCHEMA_VERSION {
            return Err("unsupported enterprise snapshot v2 commitment schema".into());
        }
        validate_lower_hex_digest("enterprise graph digest", &self.graph_digest)?;
        let (event_partition_bits, _) = validate_partitioned_root_id(
            "enterprise event-set root v1",
            &self.event_set_root_v1,
            "scout-event-set-v1",
        )?;
        let (projection_partition_bits, _) = validate_partitioned_root_id(
            "enterprise projection-map root v2",
            &self.projection_map_root_v2,
            "scout-projection-map-v2",
        )?;
        if event_partition_bits != projection_partition_bits {
            return Err(
                "enterprise event and projection roots use different partition routing".into(),
            );
        }
        validate_prefixed_root_id(
            "enterprise snapshot root v2",
            &self.enterprise_snapshot_root_v2,
            "scout-enterprise-snapshot-v2:",
        )?;
        if self.derived_root(enterprise_id)? != self.enterprise_snapshot_root_v2 {
            return Err("enterprise snapshot v2 commitment root mismatch".into());
        }
        Ok(())
    }

    pub fn derived_root(&self, enterprise_id: &EnterpriseId) -> Result<String, String> {
        Ok(format!(
            "scout-enterprise-snapshot-v2:{}",
            canonical_digest(&(
                "scout-enterprise-snapshot-v2",
                self.schema_version,
                enterprise_id.as_str(),
                &self.graph_digest,
                &self.event_set_root_v1,
                &self.projection_map_root_v2,
            ))?
        ))
    }
}

fn validate_partitioned_root_id(
    label: &str,
    value: &str,
    namespace: &str,
) -> Result<(u8, u64), String> {
    let mut parts = value.split(':');
    if parts.next() != Some(namespace) {
        return Err(format!("{label} has the wrong namespace"));
    }
    let partition_bits = parts
        .next()
        .ok_or_else(|| format!("{label} is missing partition bits"))?;
    let count = parts
        .next()
        .ok_or_else(|| format!("{label} is missing its member count"))?;
    let digest = parts
        .next()
        .ok_or_else(|| format!("{label} is missing its digest"))?;
    if parts.next().is_some() {
        return Err(format!("{label} has trailing fields"));
    }
    let partition_bits = parse_canonical_decimal(partition_bits, label)?;
    let partition_bits =
        u8::try_from(partition_bits).map_err(|_| format!("{label} partition bits overflow"))?;
    if partition_bits > 16 {
        return Err(format!("{label} partition bits exceed the supported range"));
    }
    let count = parse_canonical_decimal(count, label)?;
    validate_lower_hex_digest(label, digest)?;
    Ok((partition_bits, count))
}

fn parse_canonical_decimal(value: &str, label: &str) -> Result<u64, String> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("{label} contains a non-canonical decimal field"));
    }
    value
        .parse()
        .map_err(|_| format!("{label} decimal field overflows u64"))
}

fn validate_prefixed_root_id(label: &str, value: &str, prefix: &str) -> Result<(), String> {
    let digest = value
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{label} has the wrong namespace"))?;
    validate_lower_hex_digest(label, digest)
}

fn validate_lower_hex_digest(label: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{label} must be a lowercase 64-character hexadecimal digest"
        ));
    }
    Ok(())
}
