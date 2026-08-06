//! Persistent authenticated map commitments for mutable Scout projections.
//!
//! Keys are derived from an [`AccumulatorContext`] and stable object id. Leaves
//! commit to the object id and a caller-supplied value digest, never the value
//! itself. Tree nodes are content addressed and immutable; mutations return a
//! write plan for the changed radix path.

mod persistent;
mod proof;

use serde::{Deserialize, Serialize};

use crate::hash::{hash_tagged, hash_tagged_with_field, AccumulatorContext, Digest};
use crate::tree::{bit_at, common_prefix_bits, validate_object_id, AccumulatorError};

pub use persistent::{
    plan_map_put, plan_map_remove, prove_map_persistent, MapMutation, MapMutationOutcome,
};
pub use proof::{verify_map_proof, MapProof, MapProofStatus, MapProofStep, MapProofTerminal};

pub const AUTHENTICATED_MAP_SCHEMA_VERSION: u16 = 1;

const EMPTY_ROOT_TAG: &[u8] = b"scout-authenticated-map-empty-root-v1";
const ROOT_TAG: &[u8] = b"scout-authenticated-map-root-v1";
const LEAF_TAG: &[u8] = b"scout-authenticated-map-leaf-v1";
const BRANCH_TAG: &[u8] = b"scout-authenticated-map-branch-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapRoot {
    pub schema_version: u16,
    pub digest: Digest,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapSubtreeCommitment {
    pub digest: Digest,
    pub count: u64,
    pub min_key: Digest,
    pub max_key: Digest,
}

impl MapSubtreeCommitment {
    fn leaf(key: Digest, object_id: &str, value_digest: Digest) -> Self {
        Self {
            digest: hash_tagged_with_field(
                LEAF_TAG,
                &[key.as_bytes(), value_digest.as_bytes()],
                object_id.as_bytes(),
            ),
            count: 1,
            min_key: key,
            max_key: key,
        }
    }

    fn branch(left: Self, right: Self) -> Result<(u16, Self), AccumulatorError> {
        validate_summary(&left)?;
        validate_summary(&right)?;
        if left.max_key >= right.min_key {
            return Err(AccumulatorError::InvalidProof(
                "map branch child key ranges overlap or are reversed",
            ));
        }
        let min_key = left.min_key;
        let max_key = right.max_key;
        let branch_bit = common_prefix_bits(min_key, max_key);
        if branch_bit >= 256
            || bit_at(left.min_key, branch_bit) != 0
            || bit_at(left.max_key, branch_bit) != 0
            || bit_at(right.min_key, branch_bit) != 1
            || bit_at(right.max_key, branch_bit) != 1
        {
            return Err(AccumulatorError::InvalidProof(
                "map branch is not a canonical binary radix split",
            ));
        }
        let count = left
            .count
            .checked_add(right.count)
            .ok_or(AccumulatorError::CountOverflow)?;
        let bit_bytes = branch_bit.to_be_bytes();
        let left_count = left.count.to_be_bytes();
        let right_count = right.count.to_be_bytes();
        let digest = hash_tagged(
            BRANCH_TAG,
            &[
                &bit_bytes,
                left.digest.as_bytes(),
                &left_count,
                left.min_key.as_bytes(),
                left.max_key.as_bytes(),
                right.digest.as_bytes(),
                &right_count,
                right.min_key.as_bytes(),
                right.max_key.as_bytes(),
            ],
        );
        Ok((
            branch_bit,
            Self {
                digest,
                count,
                min_key,
                max_key,
            },
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MapStoredNode {
    Leaf {
        key: Digest,
        object_id: String,
        value_digest: Digest,
    },
    Branch {
        branch_bit: u16,
        left: MapSubtreeCommitment,
        right: MapSubtreeCommitment,
    },
}

impl MapStoredNode {
    pub fn commitment(
        &self,
        context: &AccumulatorContext,
    ) -> Result<MapSubtreeCommitment, AccumulatorError> {
        match self {
            Self::Leaf {
                key,
                object_id,
                value_digest,
            } => {
                validate_object_id(object_id)?;
                if context.object_key(object_id) != *key {
                    return Err(AccumulatorError::InvalidProof(
                        "map leaf key does not match its context and object id",
                    ));
                }
                Ok(MapSubtreeCommitment::leaf(*key, object_id, *value_digest))
            }
            Self::Branch {
                branch_bit,
                left,
                right,
            } => {
                let (observed_bit, commitment) = MapSubtreeCommitment::branch(*left, *right)?;
                if observed_bit != *branch_bit {
                    return Err(AccumulatorError::InvalidProof(
                        "map branch bit is not its canonical radix split",
                    ));
                }
                Ok(commitment)
            }
        }
    }

    pub fn digest(&self, context: &AccumulatorContext) -> Result<Digest, AccumulatorError> {
        Ok(self.commitment(context)?.digest)
    }

    fn leaf(context: &AccumulatorContext, object_id: String, value_digest: Digest) -> Self {
        Self::Leaf {
            key: context.object_key(&object_id),
            object_id,
            value_digest,
        }
    }

    fn branch(
        left: MapSubtreeCommitment,
        right: MapSubtreeCommitment,
    ) -> Result<Self, AccumulatorError> {
        let (branch_bit, _) = MapSubtreeCommitment::branch(left, right)?;
        Ok(Self::Branch {
            branch_bit,
            left,
            right,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapHead {
    pub root: MapRoot,
    pub summary: Option<MapSubtreeCommitment>,
}

impl MapHead {
    pub fn empty(context: &AccumulatorContext) -> Self {
        Self::from_summary(context, None)
    }

    pub fn from_summary(
        context: &AccumulatorContext,
        summary: Option<MapSubtreeCommitment>,
    ) -> Self {
        Self {
            root: root_from_summary(context, summary),
            summary,
        }
    }

    pub fn validate(&self, context: &AccumulatorContext) -> Result<(), AccumulatorError> {
        if self.root.schema_version != AUTHENTICATED_MAP_SCHEMA_VERSION {
            return Err(AccumulatorError::UnsupportedVersion);
        }
        if let Some(summary) = &self.summary {
            validate_summary(summary)?;
        }
        if self.root != root_from_summary(context, self.summary) {
            return Err(AccumulatorError::RootMismatch);
        }
        Ok(())
    }
}

fn root_from_summary(
    context: &AccumulatorContext,
    summary: Option<MapSubtreeCommitment>,
) -> MapRoot {
    let version = AUTHENTICATED_MAP_SCHEMA_VERSION.to_be_bytes();
    let context_digest = context.digest();
    match summary {
        None => MapRoot {
            schema_version: AUTHENTICATED_MAP_SCHEMA_VERSION,
            digest: hash_tagged(EMPTY_ROOT_TAG, &[&version, context_digest.as_bytes()]),
            count: 0,
        },
        Some(summary) => {
            let count = summary.count.to_be_bytes();
            MapRoot {
                schema_version: AUTHENTICATED_MAP_SCHEMA_VERSION,
                digest: hash_tagged(
                    ROOT_TAG,
                    &[
                        &version,
                        context_digest.as_bytes(),
                        summary.digest.as_bytes(),
                        &count,
                        summary.min_key.as_bytes(),
                        summary.max_key.as_bytes(),
                    ],
                ),
                count: summary.count,
            }
        }
    }
}

fn validate_summary(summary: &MapSubtreeCommitment) -> Result<(), AccumulatorError> {
    if summary.count == 0 {
        return Err(AccumulatorError::InvalidProof(
            "non-empty map subtree has a zero count",
        ));
    }
    if summary.min_key > summary.max_key {
        return Err(AccumulatorError::InvalidProof(
            "map subtree key range is reversed",
        ));
    }
    Ok(())
}
