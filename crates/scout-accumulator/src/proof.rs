use serde::{Deserialize, Serialize};

use crate::hash::AccumulatorContext;
use crate::tree::{
    bit_at, root_from_summary, AccumulatorError, AccumulatorRoot, SubtreeCommitment,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
}

impl Direction {
    pub(crate) fn from_bit(bit: u8) -> Self {
        if bit == 0 {
            Self::Left
        } else {
            Self::Right
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProofTerminal {
    Empty,
    Leaf { object_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofStep {
    pub branch_bit: u16,
    pub direction: Direction,
    pub sibling: SubtreeCommitment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub context: AccumulatorContext,
    pub object_id: String,
    pub terminal: ProofTerminal,
    pub steps: Vec<ProofStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    Member,
    NonMember,
}

pub fn verify_proof(
    expected_root: &AccumulatorRoot,
    proof: &Proof,
) -> Result<ProofStatus, AccumulatorError> {
    proof.context.validate()?;
    validate_path_order(&proof.steps)?;
    let query_key = proof.context.object_key(&proof.object_id);
    let (status, mut current) = match &proof.terminal {
        ProofTerminal::Empty => {
            if !proof.steps.is_empty() {
                return Err(AccumulatorError::InvalidProof(
                    "an empty terminal cannot have branch steps",
                ));
            }
            let observed = root_from_summary(&proof.context, None);
            if &observed != expected_root {
                return Err(AccumulatorError::RootMismatch);
            }
            return Ok(ProofStatus::NonMember);
        }
        ProofTerminal::Leaf { object_id } => {
            let terminal_key = proof.context.object_key(object_id);
            let status = if object_id == &proof.object_id {
                ProofStatus::Member
            } else {
                if terminal_key == query_key {
                    return Err(AccumulatorError::KeyCollision);
                }
                ProofStatus::NonMember
            };
            (status, SubtreeCommitment::leaf(terminal_key, object_id))
        }
    };

    for step in proof.steps.iter().rev() {
        if Direction::from_bit(bit_at(query_key, step.branch_bit)) != step.direction {
            return Err(AccumulatorError::InvalidProof(
                "branch direction does not match the queried key",
            ));
        }
        let (observed_bit, parent) = match step.direction {
            Direction::Left => SubtreeCommitment::branch(current, step.sibling)?,
            Direction::Right => SubtreeCommitment::branch(step.sibling, current)?,
        };
        if observed_bit != step.branch_bit {
            return Err(AccumulatorError::InvalidProof(
                "branch bit is not the canonical radix split",
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

fn validate_path_order(steps: &[ProofStep]) -> Result<(), AccumulatorError> {
    let mut previous = None;
    for step in steps {
        if step.branch_bit >= 256 {
            return Err(AccumulatorError::InvalidProof(
                "branch bit lies outside a SHA-256 key",
            ));
        }
        if previous.is_some_and(|bit| step.branch_bit <= bit) {
            return Err(AccumulatorError::InvalidProof(
                "branch bits are not strictly increasing",
            ));
        }
        previous = Some(step.branch_bit);
    }
    Ok(())
}
