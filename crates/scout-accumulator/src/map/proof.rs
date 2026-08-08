use serde::{Deserialize, Serialize};

use crate::hash::{AccumulatorContext, Digest};
use crate::proof::Direction;
use crate::tree::{bit_at, AccumulatorError};

use super::{root_from_summary, MapRoot, MapSubtreeCommitment};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MapProofTerminal {
    Empty,
    Leaf {
        object_id: String,
        value_digest: Digest,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapProofStep {
    pub branch_bit: u16,
    pub direction: Direction,
    pub sibling: MapSubtreeCommitment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapProof {
    pub context: AccumulatorContext,
    pub object_id: String,
    pub terminal: MapProofTerminal,
    pub steps: Vec<MapProofStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapProofStatus {
    Present { value_digest: Digest },
    Absent,
}

pub fn verify_map_proof(
    expected_root: &MapRoot,
    proof: &MapProof,
) -> Result<MapProofStatus, AccumulatorError> {
    proof.context.validate()?;
    validate_path_order(&proof.steps)?;
    let query_key = proof.context.object_key(&proof.object_id);
    let (status, mut current) = match &proof.terminal {
        MapProofTerminal::Empty => {
            if !proof.steps.is_empty() {
                return Err(AccumulatorError::InvalidProof(
                    "an empty map terminal cannot have branch steps",
                ));
            }
            let observed = root_from_summary(&proof.context, None);
            if &observed != expected_root {
                return Err(AccumulatorError::RootMismatch);
            }
            return Ok(MapProofStatus::Absent);
        }
        MapProofTerminal::Leaf {
            object_id,
            value_digest,
        } => {
            let terminal_key = proof.context.object_key(object_id);
            let status = if object_id == &proof.object_id {
                MapProofStatus::Present {
                    value_digest: *value_digest,
                }
            } else {
                if terminal_key == query_key {
                    return Err(AccumulatorError::KeyCollision);
                }
                MapProofStatus::Absent
            };
            (
                status,
                MapSubtreeCommitment::leaf(terminal_key, object_id, *value_digest),
            )
        }
    };

    for step in proof.steps.iter().rev() {
        if Direction::from_bit(bit_at(query_key, step.branch_bit)) != step.direction {
            return Err(AccumulatorError::InvalidProof(
                "map branch direction does not match the queried key",
            ));
        }
        let (observed_bit, parent) = match step.direction {
            Direction::Left => MapSubtreeCommitment::branch(current, step.sibling)?,
            Direction::Right => MapSubtreeCommitment::branch(step.sibling, current)?,
        };
        if observed_bit != step.branch_bit {
            return Err(AccumulatorError::InvalidProof(
                "map branch bit is not the canonical radix split",
            ));
        }
        current = parent;
    }

    let observed = root_from_summary(&proof.context, Some(current));
    if &observed != expected_root {
        return Err(AccumulatorError::RootMismatch);
    }
    Ok(status)
}

fn validate_path_order(steps: &[MapProofStep]) -> Result<(), AccumulatorError> {
    let mut previous = None;
    for step in steps {
        if step.branch_bit >= 256 {
            return Err(AccumulatorError::InvalidProof(
                "map branch bit lies outside a SHA-256 key",
            ));
        }
        if previous.is_some_and(|bit| step.branch_bit <= bit) {
            return Err(AccumulatorError::InvalidProof(
                "map branch bits are not strictly increasing",
            ));
        }
        previous = Some(step.branch_bit);
    }
    Ok(())
}
