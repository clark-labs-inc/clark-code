//! Deterministic authenticated set accumulation for Scout.
//!
//! The accumulator is a grow-only, path-compressed binary Merkle radix tree.
//! Object paths are SHA-256 hashes of length-delimited context and object-id
//! fields. The canonical tree shape is determined only by the resulting key
//! set, so replicas converge regardless of insertion order.

#![forbid(unsafe_code)]

mod hash;
mod map;
mod partitioned;
mod partitioned_map;
mod persistent;
mod proof;
mod tree;

pub use hash::{AccumulatorContext, Digest};
pub use map::{
    plan_map_put, plan_map_remove, prove_map_persistent, verify_map_proof, MapHead, MapMutation,
    MapMutationOutcome, MapProof, MapProofStatus, MapProofStep, MapProofTerminal, MapRoot,
    MapStoredNode, MapSubtreeCommitment, AUTHENTICATED_MAP_SCHEMA_VERSION,
};
pub use partitioned::{
    plan_partitioned_insert, PartitionedAccumulatorEditor, PartitionedAccumulatorHead,
    PartitionedAccumulatorMutation, PartitionedAccumulatorRoot, PartitionedAccumulatorUpdate,
    DEFAULT_PARTITION_BITS, MAX_PARTITION_BITS, PARTITIONED_ACCUMULATOR_SCHEMA_VERSION,
};
pub use partitioned_map::{
    PartitionedMapEditor, PartitionedMapHead, PartitionedMapRoot, PartitionedMapUpdate,
    PARTITIONED_AUTHENTICATED_MAP_SCHEMA_VERSION,
};
pub use persistent::{
    plan_insert, prove_persistent, AccumulatorHead, AccumulatorMutation, StoredNode,
};
pub use proof::{verify_proof, Direction, Proof, ProofStatus, ProofStep, ProofTerminal};
pub use tree::{Accumulator, AccumulatorError, AccumulatorRoot, InsertOutcome, SubtreeCommitment};
